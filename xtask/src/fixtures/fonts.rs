//! Original rectangular diagnostic fonts. No third-party glyph outlines.
use crate::util;
use anyhow::Result;
use std::{collections::BTreeMap, fs, path::Path};

type Tables = BTreeMap<[u8; 4], Vec<u8>>;
fn u16s(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn i16s(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn u32s(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn join(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}
fn padded(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    bytes
}
fn checksum(bytes: &[u8]) -> u32 {
    padded(bytes.to_vec())
        .chunks_exact(4)
        .map(|p| u32::from_be_bytes(p.try_into().unwrap()))
        .fold(0, u32::wrapping_add)
}

fn names(family: &str, style: &str) -> Vec<u8> {
    let values = [
        (1, family.to_owned()),
        (2, style.to_owned()),
        (3, format!("lazypencil-test-{family}-{style}")),
        (4, format!("{family} {style}")),
        (5, "Version 1.0".into()),
        (6, format!("{family}-{style}").replace(' ', "")),
        (16, family.into()),
        (17, style.into()),
    ];
    let mut strings = Vec::new();
    let mut records = Vec::new();
    for (id, value) in values {
        let bytes = value
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        records.extend(u16s(&[
            3,
            1,
            0x409,
            id,
            bytes.len() as u16,
            strings.len() as u16,
        ]));
        strings.extend(bytes);
    }
    join(&[u16s(&[0, 8, 6 + records.len() as u16]), records, strings])
}
fn cmap(codepoint: u32) -> Vec<u8> {
    join(&[
        u16s(&[0, 1, 3, 10]),
        u32s(&[12]),
        u16s(&[12, 0]),
        u32s(&[28, 0, 1, codepoint, codepoint, 1]),
    ])
}
fn glyph(width: i16, skew: i16) -> Vec<u8> {
    let points = [
        (80, 0),
        (width - 80, 0),
        (width - 80 + skew, 700),
        (80 + skew, 700),
    ];
    let mut x = vec![points[0].0];
    let mut y = vec![points[0].1];
    for pair in points.windows(2) {
        x.push(pair[1].0 - pair[0].0);
        y.push(pair[1].1 - pair[0].1);
    }
    padded(join(&[
        i16s(&[1, 80, 0, width - 80 + skew, 700]),
        u16s(&[3, 0]),
        vec![1; 4],
        i16s(&x),
        i16s(&y),
    ]))
}
fn cff_index(values: &[Vec<u8>]) -> Vec<u8> {
    if values.is_empty() {
        return u16s(&[0]);
    }
    let body = values.concat();
    let width = if body.len() < 255 { 1 } else { 2 };
    let mut offsets = Vec::new();
    let mut current = 1usize;
    for value in values.iter().map(Some).chain(std::iter::once(None)) {
        let bytes = (current as u16).to_be_bytes();
        offsets.extend_from_slice(&bytes[2 - width..]);
        if let Some(value) = value {
            current += value.len();
        }
    }
    join(&[
        u16s(&[values.len() as u16]),
        vec![width as u8],
        offsets,
        body,
    ])
}
fn cff_number(value: i16) -> Vec<u8> {
    join(&[vec![28], i16s(&[value])])
}
fn cff(family: &str, width: i16) -> Vec<u8> {
    let header = vec![1, 0, 4, 4];
    let name = cff_index(&[format!("{family}-Regular").replace(' ', "").into_bytes()]);
    let empty = cff_index(&[]);
    let top_size = cff_index(&[vec![0; 8]]).len();
    let charset_offset = header.len() + name.len() + top_size + empty.len() * 2;
    let charset = join(&[vec![0], u16s(&[34])]);
    let charstrings_offset = charset_offset + charset.len();
    let top = cff_index(&[join(&[
        cff_number(charset_offset as i16),
        vec![15],
        cff_number(charstrings_offset as i16),
        vec![17],
    ])]);
    let outline = join(&[
        cff_number(width),
        cff_number(80),
        cff_number(0),
        vec![21],
        cff_number(width - 160),
        cff_number(0),
        cff_number(0),
        cff_number(700),
        cff_number(160 - width),
        cff_number(0),
        cff_number(0),
        cff_number(-700),
        vec![5, 14],
    ]);
    join(&[
        header,
        name,
        top,
        empty.clone(),
        empty,
        charset,
        cff_index(&[vec![14], outline]),
    ])
}

fn tables(
    family: &str,
    style: &str,
    weight: u16,
    codepoint: u32,
    width: i16,
    is_cff: bool,
) -> Tables {
    let italic = style == "Italic";
    let bold = weight >= 700;
    let head = join(&[
        u32s(&[0x10000, 0x10000, 0, 0x5f0f3cf5]),
        u16s(&[3, 1000]),
        vec![0; 16],
        i16s(&[0, 0, width, 800]),
        u16s(&[u16::from(bold) | if italic { 2 } else { 0 }, 8]),
        i16s(&[2, 1, 0]),
    ]);
    let hhea = join(&[
        u32s(&[0x10000]),
        i16s(&[800, -200, 0]),
        u16s(&[width as u16]),
        i16s(&[0, 0, width, 1, 0, 0, 0, 0, 0, 0, 0]),
        u16s(&[2]),
    ]);
    let mut os2 = vec![0; 96];
    os2[0..2].copy_from_slice(&u16s(&[4]));
    os2[2..10].copy_from_slice(&join(&[i16s(&[width]), u16s(&[weight, 5, 0])]));
    os2[58..62].copy_from_slice(b"LPEN");
    os2[62..68].copy_from_slice(&u16s(&[
        u16::from(italic) | if bold { 32 } else { 0 } | if style == "Regular" { 64 } else { 0 },
        codepoint.min(65535) as u16,
        codepoint.min(65535) as u16,
    ]));
    os2[68..78].copy_from_slice(&join(&[i16s(&[800, -200, 0]), u16s(&[800, 200])]));
    os2[86..96].copy_from_slice(&join(&[i16s(&[500, 700]), u16s(&[0, 32, 1])]));
    let post = join(&[
        u32s(&[0x30000, if italic { 0xfff40000 } else { 0 }]),
        i16s(&[-100, 50]),
        vec![0; 20],
    ]);
    let mut tables = [
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", u16s(&[width as u16, 0, width as u16, 80])),
        (*b"OS/2", os2),
        (*b"post", post),
        (*b"cmap", cmap(codepoint)),
        (*b"name", names(family, style)),
    ]
    .into_iter()
    .collect::<Tables>();
    if is_cff {
        tables.insert(*b"maxp", join(&[u32s(&[0x5000]), u16s(&[2])]));
        tables.insert(*b"CFF ", cff(family, width));
    } else {
        let shape = glyph(width, if italic { 100 } else { 0 });
        tables.insert(*b"glyf", join(&[shape.clone(), shape.clone()]));
        tables.insert(
            *b"loca",
            u32s(&[0, shape.len() as u32, shape.len() as u32 * 2]),
        );
        tables.insert(
            *b"maxp",
            join(&[
                u32s(&[0x10000]),
                u16s(&[2, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]),
            ]),
        );
    }
    tables
}

fn sfnt(tables: &Tables, start: usize) -> Vec<u8> {
    let count = tables.len();
    let power = 1usize << count.ilog2();
    let magic = if tables.contains_key(b"CFF ") {
        b"OTTO".to_vec()
    } else {
        u32s(&[0x10000])
    };
    let mut directory = join(&[
        magic,
        u16s(&[
            count as u16,
            (power * 16) as u16,
            power.ilog2() as u16,
            (count * 16 - power * 16) as u16,
        ]),
    ]);
    let mut body = Vec::new();
    for (tag, data) in tables {
        directory.extend_from_slice(tag);
        directory.extend(u32s(&[
            checksum(data),
            (start + 12 + count * 16 + body.len()) as u32,
            data.len() as u32,
        ]));
        body.extend(padded(data.clone()));
    }
    let mut result = join(&[directory.clone(), body]);
    if start == 0 {
        let index = tables.keys().position(|tag| tag == b"head").unwrap();
        let offset = u32::from_be_bytes(
            directory[12 + index * 16 + 8..12 + index * 16 + 12]
                .try_into()
                .unwrap(),
        ) as usize;
        let adjustment = 0xb1b0afbau32.wrapping_sub(checksum(&result));
        result[offset + 8..offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    result
}

fn collection(fonts: &[&Tables]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut offsets = Vec::new();
    let start = 12 + fonts.len() * 4 + 12;
    for font in fonts {
        let offset = start + body.len();
        offsets.push(offset as u32);
        body.extend(sfnt(font, offset));
    }
    join(&[
        b"ttcf".to_vec(),
        u32s(&[0x20000, fonts.len() as u32]),
        u32s(&offsets),
        u32s(&[0, 0, 0]),
        body,
    ])
}

pub fn generate() -> Vec<(&'static str, Vec<u8>)> {
    let regular = tables("LP Test Sans", "Regular", 400, 65, 600, false);
    let bold = tables("lp test sans", "Bold", 700, 65, 820, false);
    let italic = tables("LP Test Sans", "Italic", 400, 65, 600, false);
    let supplementary = tables("LP Test Supplement", "Regular", 400, 0xf0000, 740, false);
    let cff = tables("LP Test CFF", "Regular", 400, 65, 900, true);
    vec![
        ("regular.ttf", sfnt(&regular, 0)),
        ("bold.ttf", sfnt(&bold, 0)),
        ("italic.ttf", sfnt(&italic, 0)),
        ("supplement.ttf", sfnt(&supplementary, 0)),
        (
            "faces.ttc",
            collection(&[&regular, &bold, &italic, &supplementary]),
        ),
        ("cff.otf", sfnt(&cff, 0)),
        ("cff.otc", collection(&[&cff, &supplementary])),
    ]
}

pub fn write(output: &Path) -> Result<()> {
    fs::create_dir(output)?;
    let files = generate();
    let mut manifest = Vec::new();
    for (name, bytes) in &files {
        fs::write(output.join(name), bytes)?;
        manifest.push(format!(
            "  {}: {{\n    \"bytes\": {},\n    \"sha256\": {}\n  }}",
            serde_json::to_string(name)?,
            bytes.len(),
            serde_json::to_string(&util::digest(bytes))?
        ));
    }
    let manifest = format!("{{\n{}\n}}\n", manifest.join(",\n"));
    fs::write(output.join("manifest.json"), &manifest)?;
    print!("{manifest}");
    Ok(())
}
