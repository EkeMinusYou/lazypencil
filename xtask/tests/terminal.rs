#[path = "../src/test_support/terminal.rs"]
mod terminal;
use base64::{Engine, engine::general_purpose::STANDARD};
use lazypencil_xtask::util;
use serde_json::Value;
use terminal::{Frame, KittyFrames, Screen};

// Fixed RFC 1950 bytes produced independently of the Rust codec.
const COMPRESSED_RGBA: &str = "eAH7z8Dwn+E/QwOIEjIJO/EfjQ8Ax3QLwQ==";

#[test]
fn frame_color_queries_follow_current_aligned_rgba_bytes_and_alpha() {
    let mut frame = Frame::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    assert!(frame.has_color([1, 2, 3, 4]));
    assert!(frame.has_color([5, 6, 7, 8]));
    assert!(!frame.has_color([2, 3, 4, 5]));
    assert!(!frame.has_color([1, 2, 3, 255]));
    frame.rgba[..4].copy_from_slice(&[9, 10, 11, 0]);
    assert!(!frame.has_color([1, 2, 3, 4]));
    assert!(frame.has_color([9, 10, 11, 0]));
    assert!(!frame.has_color([9, 10, 11, 255]));
    assert_eq!(frame, Frame::new(2, 1, frame.rgba.clone()).unwrap());
}

#[test]
fn owned_camera_feedback_never_satisfies_exact_frame_observations() {
    let wire = |id, pixels: &[u8]| {
        String::from_utf8(frame(pixels, 1, 1, 4))
            .unwrap()
            .replacen("a=T,", &format!("a=T,i={id},"), 1)
            .into_bytes()
    };
    for exact in [7_u32, 0x8000_0007] {
        let stream = [
            wire(exact, &[255, 0, 0, 255]),
            wire(exact ^ 0x8000_0000, &[0, 255, 0, 255]),
            wire(exact, &[0, 0, 255, 255]),
        ]
        .concat();
        for split in [1, 7, stream.len()] {
            let mut observer = KittyFrames::for_lazypencil();
            for bytes in stream.chunks(split) {
                observer.feed(bytes).unwrap();
            }
            assert_eq!(observer.images.len(), 2);
            assert_eq!(observer.images[0].rgba, [255, 0, 0, 255]);
            assert_eq!(observer.images[1].rgba, [0, 0, 255, 255]);
            assert_eq!(observer.camera_frames, 1);
            assert_eq!(observer.transmitted_resources.len(), 2);
            assert!(observer.transmitted_resources.contains(&exact));
            assert!(
                observer
                    .transmitted_resources
                    .contains(&(exact ^ 0x8000_0000))
            );
            assert_eq!(observer.last_camera.unwrap().rgba, [0, 255, 0, 255]);
        }
        let mut observer = KittyFrames::for_lazypencil();
        observer.feed(&wire(exact, &[255, 0, 0, 255])).unwrap();
        assert!(observer.feed(&wire(exact + 1, &[0, 0, 0, 255])).is_err());
    }
    assert!(
        KittyFrames::for_lazypencil()
            .feed(&frame(&[0, 0, 0, 255], 1, 1, 4))
            .is_err()
    );
    assert!(
        KittyFrames::for_lazypencil()
            .feed(&wire(0, &[0, 0, 0, 255]))
            .is_err()
    );
    let interrupted = String::from_utf8(frame(&[0; 8], 2, 1, 4))
        .unwrap()
        .replacen("a=T,", "a=T,i=7,", 1)
        .replacen("\x1b_Gm=0", "\x1b_Gi=8,m=0", 1);
    assert!(
        KittyFrames::for_lazypencil()
            .feed(interrupted.as_bytes())
            .is_err()
    );
    // The generic decoder continues accepting arbitrary valid Kitty IDs.
    let mut generic = KittyFrames::default();
    generic
        .feed(
            &[
                wire(7, &[255, 0, 0, 255]),
                wire(0x8000_0007_u32, &[0, 255, 0, 255]),
            ]
            .concat(),
        )
        .unwrap();
    assert_eq!(generic.images.len(), 2);
    assert_eq!(generic.camera_frames, 0);
}

fn frame(pixels: &[u8], width: usize, height: usize, chunk: usize) -> Vec<u8> {
    encoded_frame(pixels, width, height, chunk, false)
}

fn encoded_frame(
    payload: &[u8],
    width: usize,
    height: usize,
    chunk: usize,
    compressed: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    let parts = payload.chunks(chunk).collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        let parameters = if index == 0 {
            let encoding = if compressed { "o=z," } else { "" };
            format!("a=T,f=32,{encoding}s={width},v={height},")
        } else {
            String::new()
        };
        out.extend(
            format!(
                "\x1b_G{parameters}m={};{}\x1b\\",
                usize::from(index + 1 < parts.len()),
                STANDARD.encode(part)
            )
            .as_bytes(),
        );
    }
    out
}

#[test]
fn compressed_kitty_streams_preserve_every_rgba_byte_across_chunk_boundaries() {
    let payload = STANDARD.decode(COMPRESSED_RGBA).unwrap();
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 18, 52, 86, 200,
    ]
    .repeat(2);
    for chunk in [1, 7, payload.len()] {
        let wire = encoded_frame(&payload, 4, 2, chunk, true);
        for received in [1, 5, wire.len()] {
            let mut decoder = KittyFrames::default();
            for bytes in wire.chunks(received) {
                decoder.feed(bytes).unwrap();
            }
            decoder.feed(&frame(&pixels, 4, 2, 7)).unwrap();
            decoder.feed(&wire).unwrap();
            assert_eq!(decoder.images.len(), 3);
            assert_eq!(decoder.compressed_frames, 2);
            for image in &decoder.images {
                assert_eq!((image.width, image.height), (4, 2));
                assert_eq!(image.rgba, pixels);
                assert!(image.has_color([0, 0, 255, 0]));
                assert!(image.has_color([0, 255, 0, 128]));
            }
            assert!(decoder.payload.is_empty());
            assert!(decoder.pending.len() <= 2);
        }
    }
}

#[test]
fn compressed_kitty_rejects_truncation_checksums_trailing_data_and_expansion_beyond_dimensions() {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&[17; 512]).unwrap();
    let payload = encoder.finish().unwrap();
    let mut bad_checksum = payload.clone();
    *bad_checksum.last_mut().unwrap() ^= 1;
    let mut trailing = payload.clone();
    trailing.push(0);
    let mut concatenated = payload.clone();
    concatenated.extend_from_slice(&payload);
    for malformed in [
        payload[..payload.len() - 1].to_vec(),
        payload[..payload.len() - 4].to_vec(),
        bad_checksum,
        trailing,
        concatenated,
    ] {
        let mut decoder = KittyFrames::default();
        assert!(
            decoder
                .feed(&encoded_frame(&malformed, 8, 16, 7, true))
                .is_err()
        );
        assert!(decoder.images.is_empty());
    }
    for (width, height) in [(1, 1), (16, 16)] {
        let mut decoder = KittyFrames::default();
        assert!(
            decoder
                .feed(&encoded_frame(&payload, width, height, 7, true))
                .is_err()
        );
        assert!(decoder.images.is_empty());
    }
    let mut decoder = KittyFrames::default();
    let oversized_wire = encoded_frame(&[0; 69], 1, 1, 7, true);
    assert!(
        decoder
            .feed(&oversized_wire)
            .unwrap_err()
            .to_string()
            .contains("exceed declared")
    );
    assert!(decoder.images.is_empty());
    let mut decoder = KittyFrames::default();
    decoder
        .feed(b"\x1b_Ga=T,f=32,o=z,s=8,v=16,m=1;eA==\x1b\\")
        .unwrap();
    assert!(decoder.feed(b"\x1b_Go=z,m=0;AAAA\x1b\\").is_err());
    assert!(decoder.images.is_empty());
    assert!(
        KittyFrames::default()
            .feed(b"\x1b_Ga=T,f=32,o=unknown,s=1,v=1;AAAA\x1b\\")
            .is_err()
    );
}

#[test]
fn compressed_kitty_cannot_bypass_aggregate_retained_pixel_and_frame_limits() {
    let payload = STANDARD.decode(COMPRESSED_RGBA).unwrap();
    let wire = encoded_frame(&payload, 4, 2, 7, true);
    for frames in [
        vec![Frame::new(128, 64, vec![17; 128 * 64 * 4]).unwrap(); 2048],
        vec![Frame::new(1, 1, vec![17; 4]).unwrap(); 4096],
    ] {
        let count = frames.len();
        let mut decoder = KittyFrames::default();
        decoder.images.extend(frames);
        let error = decoder.feed(&wire).unwrap_err();
        assert!(error.to_string().contains("Retained Kitty images"));
        assert_eq!(decoder.images.len(), count);
        assert_eq!(decoder.compressed_frames, 0);
    }
}

#[test]
fn real_macos_capture_preserves_spaces_and_streamed_utf8() {
    let fixture: Value =
        util::json(&util::root().join("tests/fixtures/terminal/macos-saved-log-refresh.json"))
            .unwrap();
    let data = STANDARD
        .decode(fixture["terminal_base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(util::digest(&data), fixture["terminal_sha256"]);
    for size in [1, 7, 137, data.len()] {
        let mut screen = Screen::new(130, 34);
        for part in data.chunks(size) {
            screen.feed(part);
        }
        let text = screen.text();
        for expected in fixture["expected_text"].as_array().unwrap() {
            assert!(
                text.contains(expected.as_str().unwrap()),
                "chunk={size}\n{text}"
            );
        }
    }
}

#[test]
fn visible_phrases_accept_adjacent_panel_borders_and_require_the_original_separator() {
    for (input, expected) in [
        ("│Request complete │", true),
        ("│Request   complete│", true),
        ("Request\r\ncomplete", true),
        ("│Requestcomplete│", false),
        ("│complete Request│", false),
        ("│Request unrelated complete│", false),
    ] {
        let mut screen = Screen::new(60, 4);
        screen.feed(input.as_bytes());
        assert_eq!(
            screen.contains_words("Request", "complete"),
            expected,
            "{input}"
        );
    }
    let mut screen = Screen::new(60, 4);
    screen.feed("│Checkpoint restored│".as_bytes());
    assert!(screen.contains_words("Checkpoint", "restored"));
}

#[test]
fn screen_resize_and_wide_cell_overwrites_do_not_leave_orphan_halves() {
    let mut screen = Screen::new(8, 3);
    screen.feed(b"\x1b[3;8H");
    screen.resize(2, 1);
    assert_eq!(screen.cursor(), (1, 0));
    screen.feed(b"\x1b[KX");
    assert_eq!(screen.text(), " X");
    for (input, expected) in [
        ("あcd\x1b[1;1Ha", "a cd"),
        ("あcd\x1b[1;2Hb", " bcd"),
        ("aあc\x1b[1;1H語", "語 c"),
        ("abcあde\x1b[1;5H\x1b[K", "abc"),
        ("abcあde\x1b[1;4H\x1b[1K", "     de"),
    ] {
        let mut screen = Screen::new(12, 1);
        screen.feed(input.as_bytes());
        assert_eq!(screen.text(), expected, "{input:?}");
    }
}

#[test]
fn pixel_color_matching_respects_rgba_boundaries_and_alpha() {
    let red = [255, 0, 0, 255].repeat(8);
    assert!(red.windows(4).any(|p| p == [0, 0, 255, 255]));
    let f = Frame::new(4, 2, red).unwrap();
    assert!(f.has_color([255, 0, 0, 255]));
    assert!(!f.has_color([0, 0, 255, 255]));
    assert_eq!((f.width, f.height), (4, 2));
    assert!(Frame::new(1, 1, vec![0; 3]).is_err());
    assert!(Frame::new(0, 1, vec![]).is_err());
    assert!(Frame::new(usize::MAX, 1, vec![]).is_err());
}

#[test]
fn split_kitty_packets_are_consumed_once_and_never_replay_old_pixels() {
    let red = [255, 0, 0, 255].repeat(4);
    let blue = [0, 0, 255, 128].repeat(4);
    let mut stream = b"\x1b[3;4Htext\x1b_Ga=q,f=24,s=1,v=1;AAAA\x1b\\".to_vec();
    stream.extend(frame(&red, 2, 2, 6));
    stream.extend(b"more text");
    stream.extend(frame(&blue, 2, 2, 6));
    stream.extend(b"\x1b_Ga=d,d=I,i=31,q=2\x1b\\");
    let mut decoder = KittyFrames::default();
    for byte in stream {
        decoder.feed(&[byte]).unwrap();
    }
    assert_eq!(
        decoder.images.iter().map(|f| &f.rgba).collect::<Vec<_>>(),
        [&red, &blue]
    );
    for _ in 0..100 {
        decoder.feed(b"").unwrap();
    }
    assert_eq!(decoder.images.len(), 2);
    assert!(decoder.pending.len() <= 2);
    assert!(decoder.payload.is_empty());
    let pixels = (0u8..=255).cycle().take(25600).collect::<Vec<_>>();
    let mut decoder = KittyFrames::default();
    for part in frame(&pixels, 80, 80, 3072).chunks(4095) {
        decoder.feed(part).unwrap();
    }
    assert_eq!(decoder.images[0].rgba, pixels);
    assert_eq!(
        (decoder.images[0].width, decoder.images[0].height),
        (80, 80)
    );
    for _ in 0..20 {
        decoder
            .feed(&b"ordinary terminal output".repeat(1000))
            .unwrap();
        assert!(decoder.pending.len() <= 2);
    }
}

#[test]
fn corrupt_incomplete_interrupted_and_oversized_kitty_images_never_become_frames() {
    let incomplete = format!(
        "\x1b_Ga=T,f=32,s=2,v=2,m=1;{}\x1b\\",
        STANDARD.encode(b"first")
    );
    let mut decoder = KittyFrames::default();
    decoder.feed(incomplete.as_bytes()).unwrap();
    assert!(decoder.images.is_empty());
    assert!(
        decoder
            .feed(format!("\x1b_Gm=0;{}\x1b\\", STANDARD.encode(b"last")).as_bytes())
            .is_err()
    );
    assert!(
        KittyFrames::default()
            .feed(b"\x1b_Ga=T,f=32,s=1,v=1,m=0;invalid!\x1b\\")
            .is_err()
    );
    let mut decoder = KittyFrames::default();
    decoder.feed(incomplete.as_bytes()).unwrap();
    assert!(decoder.feed(&frame(&[0; 16], 2, 2, 3072)).is_err());
    for parameters in [
        "f=100,s=1,v=1",
        "f=32,o=z,s=1,v=1",
        "f=32,s=0,v=1",
        "f=32,s=8193,v=1",
        "f=32,s=8192,v=8192",
    ] {
        assert!(
            KittyFrames::default()
                .feed(format!("\x1b_Ga=T,{parameters},m=0;AAAA\x1b\\").as_bytes())
                .is_err()
        );
    }
    for ending in [b"".as_slice(), b"\x1b\\"] {
        let mut packet = b"\x1b_G".to_vec();
        packet.extend(vec![b'x'; 8193]);
        packet.extend(ending);
        assert!(KittyFrames::default().feed(&packet).is_err());
    }
}

#[test]
fn sixel_raster_decodes_palette_changes_repeats_and_bands() {
    assert_eq!(
        terminal::sixel_red_pixels(b"\"1;1;2;6#0;2;100;0;0!2~").unwrap(),
        (12, 12)
    );
    assert_eq!(
        terminal::sixel_red_pixels(b"\"1;1;2;6#0;2;100;0;0!2~$#1;2;0;0;100!2~").unwrap(),
        (0, 12)
    );
    assert!(terminal::sixel_red_pixels(b"\"1;1;1;1!2~").is_err());
}
