//! Original COLR/CPAL rectangles, Unicode variation mappings and required ligatures.
//! Table layouts follow the public OpenType specification, not another font's data.

use super::*;

fn rectangle(left: i16, right: i16) -> Vec<u8> {
    padded(join(&[
        i16s(&[1, left, 0, right, 700]),
        u16s(&[3, 0]),
        vec![1; 4],
        i16s(&[left, right - left, 0, left - right]),
        i16s(&[0, 0, 700, 0]),
    ]))
}

fn character_map() -> Vec<u8> {
    let mappings = [
        (0x200d, 5),
        (0x263a, 9),
        (0x1f3fb, 7),
        (0x1f469, 4),
        (0x1f4bb, 6),
        (0x1f600, 1),
    ];
    let mut full = join(&[
        u16s(&[12, 0]),
        u32s(&[16 + mappings.len() as u32 * 12, 0, mappings.len() as u32]),
    ]);
    for (point, glyph) in mappings {
        full.extend(u32s(&[point, point, glyph]));
    }
    // U+263A FE0E uses its ordinary glyph; FE0F selects the colored glyph.
    let variation = join(&[
        u16s(&[14]),
        u32s(&[49, 2]),
        vec![0, 0xfe, 0x0e],
        u32s(&[32, 0]),
        vec![0, 0xfe, 0x0f],
        u32s(&[0, 40]),
        u32s(&[1]),
        vec![0, 0x26, 0x3a, 0],
        u32s(&[1]),
        vec![0, 0x26, 0x3a],
        u16s(&[1]),
    ]);
    join(&[
        u16s(&[0, 2, 0, 5]),
        u32s(&[20 + full.len() as u32]),
        u16s(&[3, 10]),
        u32s(&[20]),
        full,
        variation,
    ])
}

fn ligatures() -> Vec<u8> {
    // The DFLT/rlig lookup maps woman + ZWJ + laptop to glyph 1 and
    // woman + light skin tone to glyph 8. Each result has one advance.
    let scripts = join(&[
        u16s(&[1]),
        b"DFLT".to_vec(),
        u16s(&[8, 4, 0, 0, 0xffff, 1, 0]),
    ]);
    let features = join(&[u16s(&[1]), b"rlig".to_vec(), u16s(&[8, 0, 1, 0])]);
    let substitution = u16s(&[
        1, 28, 1, 8, // LigatureSubst and one LigatureSet.
        2, 6, 14, // Offsets within the LigatureSet.
        1, 3, 5, 6, // woman + ZWJ + laptop -> glyph 1.
        8, 2, 7, // woman + skin tone -> glyph 8.
        1, 1, 4, // Coverage of the starting woman glyph.
    ]);
    let lookups = join(&[u16s(&[1, 4, 4, 0, 1, 8]), substitution]);
    join(&[
        u32s(&[0x10000]),
        u16s(&[10, 30, 44]),
        scripts,
        features,
        lookups,
    ])
}

pub fn generate() -> Vec<u8> {
    let mut font = tables("LP Test Color", "Regular", 400, 0x1f600, 800, false);
    let outlines = (0..10)
        .map(|glyph| match glyph {
            2 => rectangle(80, 400),
            3 => rectangle(400, 720),
            5 => vec![],
            _ => rectangle(80, 720),
        })
        .collect::<Vec<_>>();
    let mut offsets = vec![0];
    let mut glyf = Vec::new();
    let mut metrics = Vec::new();
    for (index, outline) in outlines.into_iter().enumerate() {
        glyf.extend(outline);
        offsets.push(glyf.len() as u32);
        metrics.extend(u16s(&[if index == 5 { 0 } else { 800 }, 80]));
    }
    font.insert(*b"glyf", glyf);
    font.insert(*b"loca", u32s(&offsets));
    font.insert(*b"hmtx", metrics);
    font.get_mut(b"hhea").unwrap()[34..36].copy_from_slice(&10u16.to_be_bytes());
    font.get_mut(b"maxp").unwrap()[4..6].copy_from_slice(&10u16.to_be_bytes());
    font.insert(*b"cmap", character_map());
    font.insert(*b"GSUB", ligatures());
    // Every base glyph uses the same two adjacent rectangular layers. The
    // modifier ligature reverses their palette entries to expose composition.
    let mut bases = Vec::new();
    let mut layers = Vec::new();
    for (index, glyph) in [1, 4, 6, 7, 8].into_iter().enumerate() {
        bases.extend(u16s(&[glyph, index as u16 * 2, 2]));
        layers.extend(u16s(&[
            2,
            if glyph == 8 { 1 } else { 0 },
            3,
            if glyph == 8 { 0 } else { 1 },
        ]));
    }
    font.insert(
        *b"COLR",
        join(&[u16s(&[0, 5]), u32s(&[14, 44]), u16s(&[10]), bases, layers]),
    );
    font.insert(
        *b"CPAL",
        join(&[
            u16s(&[0, 2, 1, 2]),
            u32s(&[14]),
            u16s(&[0]),
            vec![0, 0, 255, 255, 0, 255, 0, 255],
        ]),
    );
    sfnt(&font, 0)
}
