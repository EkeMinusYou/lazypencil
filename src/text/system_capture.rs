//! Freeze large CoreText fonts without asking Skia to retain a full SFNT stream.
use read_fonts::{
    FontData, FontRead,
    tables::name::{Name, NameId},
};
use sha2::{Digest, Sha256};
use skia_safe::{Data, Typeface};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};

pub(super) struct Captured {
    pub data: Data,
    pub digest: String,
    pub length: usize,
}

type CF = *const c_void;
#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontDescriptorCreateWithNameAndSize(name: CF, size: f64) -> CF;
    fn CTFontDescriptorCreateMatchingFontDescriptor(descriptor: CF, mandatory: CF) -> CF;
    fn CTFontDescriptorCopyAttribute(descriptor: CF, attribute: CF) -> CF;
    static kCTFontURLAttribute: CF;
    static kCTFontNameAttribute: CF;
    static kCTFontFormatAttribute: CF;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(
        allocator: CF,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> CF;
    fn CFURLGetFileSystemRepresentation(url: CF, resolve: u8, buffer: *mut u8, length: isize)
    -> u8;
    fn CFNumberGetValue(number: CF, kind: isize, value: *mut i32) -> u8;
    fn CFEqual(a: CF, b: CF) -> u8;
    fn CFRelease(value: *mut c_void);
}
struct Owned(CF);
impl Owned {
    fn new(value: CF) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // Every value wrapped here comes from a Core Foundation Create/Copy API.
        unsafe {
            CFRelease(self.0.cast_mut());
        }
    }
}

fn source(postscript: &str) -> Option<(PathBuf, u32)> {
    // Descriptor matching reads registry metadata, without creating another font.
    unsafe {
        let name = Owned::new(CFStringCreateWithBytes(
            std::ptr::null(),
            postscript.as_ptr(),
            postscript.len() as isize,
            0x08000100,
            0,
        ))?;
        let request = Owned::new(CTFontDescriptorCreateWithNameAndSize(name.0, 0.0))?;
        let descriptor = Owned::new(CTFontDescriptorCreateMatchingFontDescriptor(
            request.0,
            std::ptr::null(),
        ))?;
        let actual = Owned::new(CTFontDescriptorCopyAttribute(
            descriptor.0,
            kCTFontNameAttribute,
        ))?;
        if CFEqual(actual.0, name.0) == 0 {
            return None;
        }
        let url = Owned::new(CTFontDescriptorCopyAttribute(
            descriptor.0,
            kCTFontURLAttribute,
        ))?;
        let format = Owned::new(CTFontDescriptorCopyAttribute(
            descriptor.0,
            kCTFontFormatAttribute,
        ))?;
        let mut kind = 0;
        if CFNumberGetValue(format.0, 3, &mut kind) == 0 {
            return None;
        }
        let mut path = [0; 4096];
        if CFURLGetFileSystemRepresentation(url.0, 1, path.as_mut_ptr(), path.len() as isize) == 0 {
            return None;
        }
        let length = path.iter().position(|byte| *byte == 0)?;
        Some((
            std::ffi::OsString::from_vec(path[..length].to_vec()).into(),
            kind as u32,
        ))
    }
}

fn word(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
fn range(file: &mut File, offset: usize, length: usize, total: usize) -> Option<Vec<u8>> {
    if offset.checked_add(length)? > total || length > 1024 * 1024 {
        return None;
    }
    let mut bytes = vec![0; length];
    file.seek(SeekFrom::Start(offset as u64)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}
fn tables(
    file: &mut File,
    total: usize,
    postscript: &str,
) -> Option<BTreeMap<u32, (usize, usize)>> {
    let header = range(file, 0, 12, total)?;
    let offsets = if &header[..4] == b"ttcf" {
        let count = word(&header, 8)? as usize;
        if !(1..=256).contains(&count) {
            return None;
        }
        let bytes = range(file, 12, count * 4, total)?;
        (0..count)
            .map(|index| word(&bytes, index * 4).map(|value| value as usize))
            .collect::<Option<Vec<_>>>()?
    } else {
        vec![0]
    };
    for offset in offsets {
        let header = range(file, offset, 12, total)?;
        if !matches!(word(&header, 0)?, 0x10000 | 0x4f54544f | 0x74727565) {
            return None;
        }
        let count = (word(&header, 4)? >> 16) as usize;
        if !(1..=4095).contains(&count) {
            return None;
        }
        let entries = range(file, offset + 12, count * 16, total)?;
        let mut tables = BTreeMap::new();
        for entry in entries.as_chunks::<16>().0 {
            let tag = word(entry, 0)?;
            let start = word(entry, 8)? as usize;
            let length = word(entry, 12)? as usize;
            if start.checked_add(length)? > total || tables.insert(tag, (start, length)).is_some() {
                return None;
            }
        }
        let &(start, length) = tables.get(&u32::from_be_bytes(*b"name"))?;
        let bytes = range(file, start, length, total)?;
        let name = Name::read(FontData::new(&bytes)).ok()?;
        if name.name_record().iter().any(|record| {
            record.name_id() == NameId::POSTSCRIPT_NAME
                && record
                    .string(name.string_data())
                    .is_ok_and(|name| name.to_string() == postscript)
        }) {
            return Some(tables);
        }
    }
    None
}

fn checksum(bytes: &[u8]) -> u32 {
    let (words, tail) = bytes.as_chunks::<4>();
    let sum = words.iter().fold(0_u32, |sum, word| {
        sum.wrapping_add(u32::from_be_bytes(*word))
    });
    let mut padded = [0; 4];
    padded[..tail.len()].copy_from_slice(tail);
    sum.wrapping_add(u32::from_be_bytes(padded))
}

pub(super) fn capture(
    face: &Typeface,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Option<Captured> {
    let postscript = face.post_script_name()?;
    let (path, format) = source(&postscript)?;
    let mut input = crate::store::open_regular_file(&path).ok()?;
    let before = input.metadata().ok()?;
    let total = usize::try_from(before.len()).ok()?;
    if !(16 * 1024 * 1024..=256 * 1024 * 1024).contains(&total) {
        return None;
    }
    let tables = tables(&mut input, total, &postscript)?;
    let tags = face.read_table_tags()?;
    if tags.is_empty() || tags.len() > 4095 || tags.iter().any(|tag| !tables.contains_key(tag)) {
        return None;
    }
    // Match SkTypeface_Mac::onOpenStream exactly, including its directory order,
    // header convention and original head checksum adjustment.
    let magic = match format {
        1 => *b"OTTO",
        2 => [0, 1, 0, 0],
        3 | 5 => *b"true",
        _ => return None,
    };
    let mut header = vec![0; 12 + tags.len() * 16];
    header[..4].copy_from_slice(&magic);
    let mut selector = 0;
    let mut search = 1;
    while search < tags.len() >> 1 {
        selector += 1;
        search <<= 1;
    }
    search <<= 4;
    for (offset, value) in [
        (4, tags.len()),
        (6, search),
        (8, selector),
        (10, tags.len() * 16 - search),
    ] {
        header[offset..offset + 2].copy_from_slice(&(value as u16).to_be_bytes());
    }
    let mut length = header.len();
    let mut buffer = [0; 64 * 1024];
    let mut checksums = Vec::new();
    for (index, tag) in tags.iter().enumerate() {
        crate::cancel::check(cancel).ok()?;
        let &(start, size) = tables.get(tag)?;
        let mut sum = 0_u32;
        input.seek(SeekFrom::Start(start as u64)).ok()?;
        let mut remaining = size;
        while remaining > 0 {
            crate::cancel::check(cancel).ok()?;
            let count = remaining.min(buffer.len());
            input.read_exact(&mut buffer[..count]).ok()?;
            sum = sum.wrapping_add(checksum(&buffer[..count]));
            remaining -= count;
        }
        checksums.push(sum);
        let record = 12 + index * 16;
        for (offset, value) in [
            (record, *tag),
            (record + 4, sum),
            (record + 8, length as u32),
            (record + 12, size as u32),
        ] {
            header[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        length = length.checked_add(size.next_multiple_of(4))?;
        if length > 256 * 1024 * 1024 {
            return None;
        }
    }
    // Also verify that this file is the face selected by CoreText before
    // substituting a streamed capture for the native stream implementation.
    let &(start, size) = tables.get(&u32::from_be_bytes(*b"head"))?;
    let expected = range(&mut input, start, size, total)?;
    let mut native = vec![0; size];
    if face.get_table_data(u32::from_be_bytes(*b"head"), &mut native) != size || native != expected
    {
        return None;
    }
    let mut output = tempfile::NamedTempFile::new().ok()?;
    let mut digest = Sha256::new();
    output.write_all(&header).ok()?;
    digest.update(&header);
    for (index, tag) in tags.iter().enumerate() {
        let &(start, size) = tables.get(tag)?;
        input.seek(SeekFrom::Start(start as u64)).ok()?;
        let mut remaining = size;
        let mut sum = 0_u32;
        while remaining > 0 {
            crate::cancel::check(cancel).ok()?;
            let count = remaining.min(buffer.len());
            input.read_exact(&mut buffer[..count]).ok()?;
            sum = sum.wrapping_add(checksum(&buffer[..count]));
            output.write_all(&buffer[..count]).ok()?;
            digest.update(&buffer[..count]);
            remaining -= count;
        }
        if sum != checksums[index] {
            return None;
        }
        let padding = [0; 3];
        let padding = &padding[..size.next_multiple_of(4) - size];
        output.write_all(padding).ok()?;
        digest.update(padding);
    }
    let after = input.metadata().ok()?;
    if before.len() != after.len() || before.modified().ok()? != after.modified().ok()? {
        return None;
    }
    Some(Captured {
        data: Data::from_filename(output.path())?,
        digest: crate::document::digest_hex(digest.finalize()),
        length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streamed_large_native_capture_matches_every_byte_of_skias_font_stream() {
        let _pool = crate::native_memory::AutoreleasePool::new();
        let face = skia_safe::FontMgr::new()
            .match_family_style("Apple Color Emoji", skia_safe::FontStyle::normal())
            .unwrap();
        let captured = capture(&face, None)
            .expect("The installed large emoji font supports streaming capture");
        let (expected, index) = face.to_font_bytes().unwrap();
        assert_eq!(index, 0);
        assert_eq!(captured.length, expected.len());
        assert!(
            captured.data.as_bytes() == expected,
            "Streamed font differs at byte {:?}",
            captured
                .data
                .iter()
                .zip(&expected)
                .position(|(a, b)| a != b)
        );
        assert_eq!(captured.digest, crate::document::hash(&expected));
    }
}
