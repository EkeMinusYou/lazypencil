//! Original two-color PNG glyphs in the public OpenType sbix structure.
use super::{base, i16s, join, sfnt, u16s, u32s};
use anyhow::Result;
use flate2::{Compression, Crc, write::ZlibEncoder};
use std::io::Write;

fn chunk(output: &mut Vec<u8>, tag: [u8; 4], bytes: &[u8]) {
    output.extend((bytes.len() as u32).to_be_bytes());
    output.extend(tag);
    output.extend(bytes);
    let mut crc = Crc::new();
    crc.update(&tag);
    crc.update(bytes);
    output.extend(crc.sum().to_be_bytes());
}

fn png(reversed: bool) -> Result<Vec<u8>> {
    let mut scanlines = Vec::new();
    for _ in 0..70 {
        scanlines.push(0);
        for x in 0..64 {
            scanlines.extend(if (x < 32) != reversed {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            });
        }
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&scanlines)?;
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(
        &mut output,
        *b"IHDR",
        &join(&[u32s(&[64, 70]), vec![8, 6, 0, 0, 0]]),
    );
    chunk(&mut output, *b"IDAT", &encoder.finish()?);
    chunk(&mut output, *b"IEND", &[]);
    Ok(output)
}

pub fn generate() -> Result<Vec<u8>> {
    let mut font = base("LP Test Bitmap");
    let normal = png(false)?;
    let reversed = png(true)?;
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    // One strike at 100 PPEM. Each bitmap covers 640 x 700 design units,
    // matching the original outlines in the shared 1,000 UPEM face.
    for glyph in 0..10 {
        offsets.push(48 + data.len() as u32);
        match glyph {
            1 | 8 => data.extend(join(&[
                i16s(&[0, 0]),
                b"png ".to_vec(),
                if glyph == 1 {
                    normal.clone()
                } else {
                    reversed.clone()
                },
            ])),
            4 | 6 | 7 => data.extend(join(&[i16s(&[0, 0]), b"dupe".to_vec(), u16s(&[1])])),
            _ => {}
        }
    }
    offsets.push(48 + data.len() as u32);
    let strike = join(&[u16s(&[100, 72]), u32s(&offsets), data]);
    // Version 1, required flag bit 0, no outline overlay, one strike at offset 12.
    font.insert(*b"sbix", join(&[u16s(&[1, 1]), u32s(&[1, 12]), strike]));
    Ok(sfnt(&font, 0))
}
