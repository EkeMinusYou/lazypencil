//! Read only collection directories and names until a family is actually used.

use crate::{Error, Result, project::font_files::FontFile};
use std::collections::BTreeSet;

fn invalid(message: &str) -> Error {
    Error::new("INVALID_ARGUMENT", message)
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().expect("bounded font header"))
}

pub(super) fn families(source: &FontFile) -> Result<Vec<Option<Vec<String>>>> {
    let mut reader = source.reader()?;
    let header = source.read_range(&mut *reader, 0, 12)?;
    let offsets = if &header[..4] == b"ttcf" {
        let version = word(&header, 4);
        let count = word(&header, 8) as usize;
        if !matches!(version, 0x10000 | 0x20000) || !(1..=256).contains(&count) {
            return Err(invalid("Invalid font collection header"));
        }
        let end = 12 + 4 * count + if version == 0x20000 { 12 } else { 0 };
        let bytes = source.read_range(&mut *reader, 12, end - 12)?;
        (0..count)
            .map(|index| {
                let offset = word(&bytes, index * 4) as usize;
                if offset < end || !offset.is_multiple_of(4) {
                    return Err(invalid("Invalid font collection face offset"));
                }
                Ok(offset)
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![0]
    };
    let mut result = Vec::new();
    for offset in offsets {
        let header = source.read_range(&mut *reader, offset, 12)?;
        if !matches!(word(&header, 0), 0x10000 | 0x4f54544f | 0x74727565) {
            return Err(invalid("Unsupported font face format"));
        }
        let count = (word(&header, 4) >> 16) as usize;
        if !(1..=4095).contains(&count) {
            return Err(invalid("A font face needs 1..4095 tables"));
        }
        let entries = source.read_range(&mut *reader, offset + 12, count * 16)?;
        let mut seen = BTreeSet::new();
        let mut name = None;
        for entry in entries.chunks_exact(16) {
            let tag = word(entry, 0);
            let start = word(entry, 8) as usize;
            let length = word(entry, 12) as usize;
            if !seen.insert(tag)
                || start
                    .checked_add(length)
                    .is_none_or(|end| end > source.length)
            {
                return Err(invalid("Invalid or duplicate font table range"));
            }
            if tag == u32::from_be_bytes(*b"name") {
                name = Some(source.read_range(&mut *reader, start, length)?);
            }
        }
        let bytes = name.ok_or_else(|| invalid("Font has no name table"))?;
        let table = ttf_parser::name::Table::parse(&bytes)
            .ok_or_else(|| invalid("Invalid font name table"))?;
        let mut names = table
            .names
            .into_iter()
            .filter_map(|name| {
                if ![
                    ttf_parser::name_id::FAMILY,
                    ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
                ]
                .contains(&name.name_id)
                {
                    return None;
                }
                let value = name.to_string()?;
                if value.is_empty() || value.len() > 256 {
                    return None;
                }
                Some((
                    name.name_id != ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
                    name.language_id != 0x409,
                    value,
                ))
            })
            .collect::<Vec<_>>();
        names.sort();
        let mut unique = BTreeSet::new();
        let names = names
            .into_iter()
            .map(|(_, _, name)| name)
            .filter(|name| unique.insert(name.to_lowercase()))
            .collect::<Vec<_>>();
        // Legacy non-Unicode names retain native loading instead of inventing
        // an encoding or silently excluding a previously supported font.
        result.push((!names.is_empty()).then_some(names));
    }
    Ok(result)
}
