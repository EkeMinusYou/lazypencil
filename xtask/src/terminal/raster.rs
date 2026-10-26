//! Decode the bounded TrueColor XWD snapshots used by the hosted X server.
use crate::fixtures::oracles::Raster;
use anyhow::{Context, Result, ensure};
use flate2::{Compression, Crc, write::ZlibEncoder};
use serde::Serialize;
use std::{collections::VecDeque, io::Write};

/// XWD v7's header is big-endian; pixel byte order is a separate header field.
/// Accept RGB TrueColor windows, including 32-bit visuals on a 24-bit server.
pub fn decode(bytes: &[u8]) -> Result<Raster> {
    ensure!(bytes.len() >= 100, "Truncated XWD header");
    let word = |i: usize| u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    let header = word(0) as usize;
    let (width, height) = (word(4), word(5));
    ensure!(
        word(1) == 7 && word(2) == 2 && [24, 32].contains(&word(3)) && word(6) == 0,
        "Expected XWD v7 RGB ZPixmap"
    );
    ensure!(
        (1..=2048).contains(&width) && (1..=2048).contains(&height),
        "Screenshot exceeds the hosted display bounds"
    );
    ensure!(
        header > 100 && header <= bytes.len() && bytes[header - 1] == 0,
        "Invalid XWD window-name boundary"
    );
    let bpp = word(11);
    ensure!(
        word(7) <= 1 && [24, 32].contains(&bpp) && word(3) <= bpp && word(13) == 4,
        "Expected packed TrueColor screenshot pixels"
    );
    let masks = [word(14), word(15), word(16)];
    ensure!(
        masks
            .iter()
            .all(|m| m.count_ones() == 8 && (*m >> m.trailing_zeros()) == 255)
            && masks[0] & masks[1] == 0
            && masks[0] & masks[2] == 0
            && masks[1] & masks[2] == 0
            && masks.iter().all(|m| *m <= 0xffffff),
        "Expected disjoint eight-bit TrueColor channels"
    );
    let stride = word(12) as usize;
    let pixel_bytes = bpp as usize / 8;
    ensure!(
        stride >= width as usize * pixel_bytes && stride <= 8192,
        "Invalid screenshot row stride"
    );
    let offset = header
        .checked_add(word(19) as usize * 12)
        .context("XWD color-map offset overflow")?;
    let end = offset
        .checked_add(stride * height as usize)
        .context("XWD pixel offset overflow")?;
    ensure!(end == bytes.len(), "Truncated or trailing XWD image data");
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for row in bytes[offset..].chunks_exact(stride) {
        for p in row[..width as usize * pixel_bytes].chunks_exact(pixel_bytes) {
            let value = if word(7) == 0 {
                p.iter()
                    .enumerate()
                    .fold(0u32, |n, (i, b)| n | (*b as u32) << (i * 8))
            } else {
                p.iter().fold(0u32, |n, b| (n << 8) | *b as u32)
            };
            pixels.push([
                ((value & masks[0]) >> masks[0].trailing_zeros()) as u8,
                ((value & masks[1]) >> masks[1].trailing_zeros()) as u8,
                ((value & masks[2]) >> masks[2].trailing_zeros()) as u8,
                255,
            ]);
        }
    }
    Ok(Raster {
        width,
        height,
        pixels,
    })
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], bytes: &[u8]) {
    out.extend((bytes.len() as u32).to_be_bytes());
    out.extend(tag);
    out.extend(bytes);
    let mut crc = Crc::new();
    crc.update(tag);
    crc.update(bytes);
    out.extend(crc.sum().to_be_bytes());
}

pub fn png(frame: &Raster) -> Result<Vec<u8>> {
    ensure!(
        (1..=2048).contains(&frame.width)
            && (1..=2048).contains(&frame.height)
            && frame.pixels.len() == frame.width as usize * frame.height as usize,
        "Invalid screenshot raster"
    );
    let mut raw = Vec::with_capacity(frame.pixels.len() * 4 + frame.height as usize);
    for row in frame.pixels.chunks_exact(frame.width as usize) {
        raw.push(0);
        raw.extend(row.iter().flatten());
    }
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    z.write_all(&raw)?;
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = frame.width.to_be_bytes().to_vec();
    header.extend(frame.height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &z.finish()?);
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Region {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub pixels: usize,
}

/// Text-colored pixels cannot masquerade as the large, connected test markers.
pub fn regions(frame: &Raster, color: [u8; 4], minimum: usize) -> Vec<Region> {
    let width = frame.width as usize;
    let mut seen = vec![false; frame.pixels.len()];
    let mut out = Vec::new();
    for start in 0..frame.pixels.len() {
        if seen[start] || frame.pixels[start] != color {
            continue;
        }
        seen[start] = true;
        let mut queue = VecDeque::from([start]);
        let mut region = Region {
            left: frame.width,
            top: frame.height,
            right: 0,
            bottom: 0,
            pixels: 0,
        };
        while let Some(i) = queue.pop_front() {
            let (x, y) = ((i % width) as u32, (i / width) as u32);
            region.left = region.left.min(x);
            region.top = region.top.min(y);
            region.right = region.right.max(x + 1);
            region.bottom = region.bottom.max(y + 1);
            region.pixels += 1;
            for next in [
                (x > 0).then(|| i - 1),
                (x + 1 < frame.width).then_some(i + 1),
                (y > 0).then(|| i - width),
                (y + 1 < frame.height).then_some(i + width),
            ]
            .into_iter()
            .flatten()
            {
                if !seen[next] && frame.pixels[next] == color {
                    seen[next] = true;
                    queue.push_back(next);
                }
            }
        }
        if region.pixels >= minimum {
            out.push(region);
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.pixels));
    out
}
