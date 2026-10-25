#[path = "../src/test_support/terminal.rs"]
mod terminal;
use base64::{Engine, engine::general_purpose::STANDARD};
use lazypencil_xtask::util;
use serde_json::Value;
use terminal::{Frame, KittyFrames, Screen};

fn frame(pixels: &[u8], width: usize, height: usize, chunk: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let parts = pixels.chunks(chunk).collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        let parameters = if index == 0 {
            format!("a=T,f=32,s={width},v={height},")
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
    let f = Frame::new(red).unwrap();
    assert!(f.has_color([255, 0, 0, 255]));
    assert!(!f.has_color([0, 0, 255, 255]));
    assert!(Frame::new(vec![0; 3]).is_err());
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
