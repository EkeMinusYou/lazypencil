//! Original XWD fixtures check the screen observer without launching a GUI.
use flate2::{Crc, read::ZlibDecoder};
use lazypencil_xtask::{
    fixtures::oracles::Raster,
    terminal::{
        DISPLAY_HEIGHT, DISPLAY_WIDTH,
        raster::{self, Region},
    },
};
use std::io::Read;

const COLORS: [[u8; 4]; 6] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 255, 255],
    [17, 34, 51, 255],
    [0, 0, 0, 255],
];

fn xwd(order: u32, bpp: u32) -> Vec<u8> {
    let pixel_bytes = bpp as usize / 8;
    let stride = 16;
    let mut words = [0u32; 25];
    for (i, v) in [
        (0, 106),
        (1, 7),
        (2, 2),
        (3, 24),
        (4, 3),
        (5, 2),
        (7, order),
        (8, 32),
        (10, 32),
        (11, bpp),
        (12, stride),
        (13, 4),
        (14, 0xff0000),
        (15, 0xff00),
        (16, 0xff),
        (17, 8),
        (18, 256),
        (19, 1),
        (20, 3),
        (21, 2),
    ] {
        words[i] = v;
    }
    let mut bytes = words
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect::<Vec<_>>();
    bytes.extend(b"owned\0");
    bytes.extend([0x91; 12]); // One color-map entry is separate from the image.
    for row in COLORS.chunks_exact(3) {
        let start = bytes.len();
        for color in row {
            let pixel = u32::from_be_bytes([0, color[0], color[1], color[2]]);
            if order == 0 {
                bytes.extend(&pixel.to_le_bytes()[..pixel_bytes]);
            } else {
                bytes.extend(&pixel.to_be_bytes()[4 - pixel_bytes..]);
            }
        }
        bytes.resize(start + stride as usize, 0x72);
    }
    bytes
}

#[test]
fn screenshot_pixels_respect_xwd_header_endianness_pixel_order_and_row_padding() {
    for order in [0, 1] {
        for bpp in [24, 32] {
            let frame = raster::decode(&xwd(order, bpp)).unwrap();
            assert_eq!((frame.width, frame.height), (3, 2));
            assert_eq!(frame.pixels, COLORS);
        }
    }
    let mut swapped = xwd(0, 32);
    swapped[14 * 4..15 * 4].copy_from_slice(&0xffu32.to_be_bytes());
    swapped[16 * 4..17 * 4].copy_from_slice(&0xff0000u32.to_be_bytes());
    let expected = COLORS.map(|[r, g, b, a]| [b, g, r, a]);
    assert_eq!(raster::decode(&swapped).unwrap().pixels, expected);
    let mut rgba_visual = xwd(0, 32);
    rgba_visual[12..16].copy_from_slice(&32u32.to_be_bytes());
    assert_eq!(raster::decode(&rgba_visual).unwrap().pixels, COLORS);
}

#[test]
fn malformed_or_unbounded_screenshots_fail_before_pixel_allocation() {
    let original = xwd(0, 32);
    for length in 0..original.len() {
        assert!(
            raster::decode(&original[..length]).is_err(),
            "prefix {length}"
        );
    }
    for (word, value) in [
        (0, 100),
        (0, u32::MAX),
        (1, 8),
        (2, 1),
        (3, 16),
        (4, 0),
        (4, u32::MAX),
        (5, 0),
        (6, 1),
        (7, 2),
        (11, 64),
        (12, 11),
        (4, DISPLAY_WIDTH + 1),
        (5, DISPLAY_HEIGHT + 1),
        (12, DISPLAY_WIDTH * 4 + 1),
        (13, 3),
        (14, 0),
        (14, 0xff),
        (14, 0xff000000),
        (19, u32::MAX),
    ] {
        let mut bytes = original.clone();
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
        assert!(
            raster::decode(&bytes).is_err(),
            "word {word}, value {value}"
        );
    }
    let mut bytes = original.clone();
    bytes.push(0);
    assert!(raster::decode(&bytes).is_err());
    let mut bytes = original;
    bytes[105] = 1;
    assert!(raster::decode(&bytes).is_err());
}

#[test]
fn wide_cjk_windows_fit_the_owned_display_and_keep_codec_limits_aligned() {
    for (width, height) in [(2204, 2), (DISPLAY_WIDTH, 2), (2, DISPLAY_HEIGHT)] {
        let mut bytes = xwd(0, 32);
        bytes.truncate(118); // Header, owned window name and one color-map entry.
        for (word, value) in [
            (4, width),
            (5, height),
            (12, width * 4),
            (20, width),
            (21, height),
        ] {
            bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
        }
        bytes.extend([51, 34, 17, 0].repeat((width * height) as usize));
        let frame = raster::decode(&bytes).unwrap();
        assert_eq!((frame.width, frame.height), (width, height));
        assert!(frame.pixels.iter().all(|pixel| *pixel == [17, 34, 51, 255]));
        let png = raster::png(&frame).unwrap();
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), width);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), height);
    }
}

#[test]
fn screenshot_png_retains_every_pixel_and_valid_chunk_boundaries() {
    let frame = raster::decode(&xwd(1, 24)).unwrap();
    let png = raster::png(&frame).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let mut position = 8;
    let mut types = Vec::new();
    let mut compressed = Vec::new();
    while position < png.len() {
        let size = u32::from_be_bytes(png[position..position + 4].try_into().unwrap()) as usize;
        let tag = &png[position + 4..position + 8];
        let body = &png[position + 8..position + 8 + size];
        let mut crc = Crc::new();
        crc.update(tag);
        crc.update(body);
        assert_eq!(
            crc.sum().to_be_bytes(),
            png[position + 8 + size..position + 12 + size]
        );
        if tag == b"IHDR" {
            assert_eq!(body, &[0, 0, 0, 3, 0, 0, 0, 2, 8, 6, 0, 0, 0]);
        } else if tag == b"IDAT" {
            compressed.extend(body);
        }
        types.push(tag);
        position += size + 12;
    }
    assert_eq!(types, [b"IHDR", b"IDAT", b"IEND"]);
    let mut raw = Vec::new();
    ZlibDecoder::new(compressed.as_slice())
        .read_to_end(&mut raw)
        .unwrap();
    let expected = COLORS
        .chunks_exact(3)
        .flat_map(|row| std::iter::once(0).chain(row.iter().flatten().copied()))
        .collect::<Vec<_>>();
    assert_eq!(raw, expected);
    assert!(
        raster::png(&Raster {
            width: 0,
            height: 1,
            pixels: vec![]
        })
        .is_err()
    );
}

#[test]
fn connected_markers_exclude_scattered_ui_text_and_keep_separate_graphics() {
    let red = [255, 0, 0, 255];
    let mut frame = Raster::solid(10, 8, [0, 0, 0, 255]);
    for y in 1..4 {
        for x in 2..6 {
            frame.pixels[y * 10 + x] = red;
        }
    }
    for y in 5..7 {
        for x in 7..9 {
            frame.pixels[y * 10 + x] = red;
        }
    }
    frame.pixels[0] = red;
    frame.pixels[1] = red; // Diagonal adjacency does not join the main marker.
    assert_eq!(
        raster::regions(&frame, red, 3),
        vec![
            Region {
                left: 2,
                top: 1,
                right: 6,
                bottom: 4,
                pixels: 12
            },
            Region {
                left: 7,
                top: 5,
                right: 9,
                bottom: 7,
                pixels: 4
            },
        ]
    );
}
