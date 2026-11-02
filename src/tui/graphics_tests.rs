use super::*;
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

#[cfg(unix)]
#[test]
fn startup_graphics_replies_after_status_preserve_only_typed_text() {
    let bytes =
        "\x1b[6;20;10t\x1b[0n日本語\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\の下書き".as_bytes();
    let mut parser = Parser::new();
    let responses = bytes
        .iter()
        .flat_map(|byte| parser.push(char::from(*byte)))
        .collect::<Vec<_>>();
    assert!(responses.contains(&Response::Kitty));
    assert!(responses.contains(&Response::KittyCompression));
    assert!(responses.contains(&Response::Status));
    assert_eq!(startup_text(bytes), "日本語の下書き");
}

#[test]
fn kitty_virtual_placements_match_placeholder_cells_with_changed_pixel_metrics() {
    let size = Size::new(80, 24);
    let area = Rect::new(3, 2, size.width, size.height);
    let bounds = Rect::new(0, 0, 90, 30);
    for (width, height) in [(5, 10), (10, 20)] {
        for tmux in [false, true] {
            for compressed in [false, true] {
                let backend = Backend {
                    font: FontSize::new(width, height),
                    protocol: ProtocolType::Kitty,
                    tmux,
                    kitty_compression: compressed,
                };
                let pixels = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    u32::from(width) * u32::from(size.width),
                    u32::from(height) * u32::from(size.height),
                    image::Rgba([12, 34, 56, 255]),
                ));
                let protocol = backend.encode(pixels, size, 77).unwrap();
                let mut buffer = Buffer::empty(bounds);
                Image(&protocol).render(area, &mut buffer);
                let first = buffer[(area.x, area.y)].symbol();
                let parameters = first.split_once("_G").unwrap().1.split_once(';').unwrap().0;
                let fields = parameters
                    .split(',')
                    .filter_map(|field| field.split_once('='))
                    .collect::<std::collections::BTreeMap<_, _>>();
                // Native cells can be twice the reported/fallback metrics.
                // Without c/r, Kitty would display only a 40 x 12 grid.
                let natural_columns = fields["s"].parse::<u16>().unwrap() / (2 * width);
                assert_eq!(natural_columns, 40);
                assert_eq!(fields["c"].parse::<u16>().unwrap(), size.width);
                assert_eq!(fields["r"].parse::<u16>().unwrap(), size.height);
                assert_eq!(first.matches("U=1").count(), 1);
                assert!(!first.contains("a=p"));
                assert!(
                    buffer[(area.right() - 1, area.bottom() - 1)]
                        .symbol()
                        .contains('\u{10EEEE}')
                );
                assert_eq!(buffer[(0, 0)].symbol(), " ");
                let mut next = Buffer::empty(bounds);
                Image(&protocol).render(area, &mut next);
                assert!(
                    !next[(area.x, area.y)].symbol().contains("_G"),
                    "Stable redraws must retain one placement without retransmission"
                );
            }
        }
    }
}

#[test]
fn compressed_and_raw_kitty_transfers_preserve_identical_rgba_without_changing_placement() {
    let colors = [
        [255, 0, 0, 255],
        [0, 255, 0, 127],
        [0, 0, 255, 0],
        [18, 52, 86, 200],
    ];
    let source = image::RgbaImage::from_fn(64, 32, |x, y| {
        image::Rgba(colors[((x / 8 + y / 4) % 4) as usize])
    });
    let area = Rect::new(0, 0, 64, 32);
    let size = Size::new(64, 32);
    let mut lengths = Vec::new();
    for compressed in [false, true] {
        let backend = Backend {
            font: FontSize::new(1, 1),
            protocol: ProtocolType::Kitty,
            tmux: false,
            kitty_compression: compressed,
        };
        let protocol = backend
            .encode(DynamicImage::ImageRgba8(source.clone()), size, 77)
            .unwrap();
        assert_eq!(protocol.size(), size);
        let mut buffer = Buffer::empty(area);
        Image(&protocol).render(area, &mut buffer);
        let mut wire = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                wire.push_str(buffer[(x, y)].symbol());
            }
        }
        assert_eq!(wire.contains("o=z,"), compressed);
        let mut decoder = observer::KittyFrames::default();
        for bytes in wire.as_bytes().chunks(137) {
            decoder.feed(bytes).unwrap();
        }
        assert_eq!(decoder.images.len(), 1);
        assert_eq!(decoder.compressed_frames, usize::from(compressed));
        assert_eq!(
            (decoder.images[0].width, decoder.images[0].height),
            (64, 32)
        );
        assert_eq!(decoder.images[0].rgba(), source.as_raw().as_slice());
        lengths.push(wire.len());
    }
    assert!(
        lengths[1] < lengths[0],
        "Compressed transport must reduce bytes"
    );
}

#[test]
fn kitty_frames_switch_inactive_slots_and_keep_stable_redraws_without_retransmission() {
    let area = Rect::new(0, 0, 3, 2);
    let backend = Backend {
        font: FontSize::new(1, 1),
        protocol: ProtocolType::Kitty,
        tmux: false,
        kitty_compression: true,
    };
    let mut presentation = Presentation::default();
    let mut observer = observer::KittyFrames::for_lazypencil();
    let mut previous = Buffer::empty(area);
    let mut last_id = None;
    for camera in [false, false, true, true, false, false] {
        let source = 42 | if camera { 0x8000_0000 } else { 0 };
        let protocol = backend
            .encode(
                DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    3,
                    2,
                    image::Rgba([12, 34, 56, 255]),
                )),
                area.as_size(),
                source,
            )
            .unwrap();
        let mut next = Buffer::empty(area);
        Image(&protocol).render(area, &mut next);
        presentation.prepare(&mut next, area, camera);
        let target = presentation.visible.unwrap();
        assert_ne!(
            Some(target),
            last_id,
            "Never retransmit into the displayed slot"
        );
        assert_eq!((target ^ source) & !1, 0);
        let updates = previous.diff(&next);
        assert_eq!(
            updates.len(),
            6,
            "Replace every placeholder through ordinary diffing"
        );
        for (_, _, cell) in updates {
            observer.feed(cell.symbol().as_bytes()).unwrap();
            let [_, r, g, b] = target.to_be_bytes();
            assert_eq!(cell.fg, ratatui::style::Color::Rgb(r, g, b));
        }
        let mut stable = Buffer::empty(area);
        Image(&protocol).render(area, &mut stable);
        presentation.prepare(&mut stable, area, camera);
        assert!(!stable[(0, 0)].symbol().contains("_G"));
        assert_eq!(presentation.visible, Some(target));
        // Only the first cell loses the one-time transmission prefix.
        assert!(next.diff(&stable).len() <= 1);
        previous = stable;
        last_id = Some(target);
    }
    assert_eq!(observer.images.len(), 4);
    assert_eq!(observer.camera_frames, 2);
    assert!(observer.transmitted_resources.len() <= 4);
    assert!(
        observer
            .images
            .iter()
            .all(|frame| frame.rgba() == [12, 34, 56, 255].repeat(6))
    );
}

#[test]
fn rectangular_kitty_uploads_once_places_the_whole_image_and_restores_after_modals() {
    let area = Rect::new(2, 1, 8, 4);
    let bounds = Rect::new(0, 0, 16, 8);
    let backend = Backend {
        font: FontSize::new(10, 20),
        protocol: ProtocolType::Kitty,
        tmux: false,
        kitty_compression: true,
    };
    let pixels = image::RgbaImage::from_pixel(80, 80, image::Rgba([248, 250, 252, 255]));
    let protocol = backend
        .encode(DynamicImage::ImageRgba8(pixels.clone()), area.as_size(), 42)
        .unwrap();
    let mut presentation = Presentation::default();
    presentation.rectangular(true);
    let render = |presentation: &mut Presentation, modal| {
        presentation.begin();
        let mut buffer = Buffer::empty(bounds);
        Image(&protocol).render(area, &mut buffer);
        presentation.prepare(&mut buffer, area, false);
        presentation.finish(&mut buffer, modal);
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| !cell.symbol().contains('\u{10eeee}'))
        );
        buffer[(0, 0)].symbol().to_owned()
    };
    // An upload first encountered underneath a modal must survive until the
    // image can be shown, even though the encoder's transmit-once flag is set.
    assert_eq!(render(&mut presentation, true), " ");
    let wire = render(&mut presentation, false);
    assert!(!wire.contains("U=1"));
    assert_eq!(wire.matches("a=p,").count(), 1);
    assert!(wire.contains("c=8,r=4,C=1"));
    assert!(wire.contains("\x1b[2;3H"));
    assert!(wire.starts_with("\x1b7") && wire.ends_with("\x1b8 "));
    let mut observer = observer::KittyFrames::for_lazypencil();
    for bytes in wire.as_bytes().chunks(137) {
        observer.feed(bytes).unwrap();
    }
    assert_eq!(observer.images.len(), 1);
    assert_eq!(observer.images[0].rgba(), pixels.as_raw());
    assert_eq!(render(&mut presentation, false), " ");
    presentation.invalidate();
    let resized = render(&mut presentation, false);
    assert!(resized.contains("a=p,i=42,p=1"));
    assert!(!resized.contains("a=t,"));
    let hidden = render(&mut presentation, true);
    assert!(hidden.contains("a=d,d=i,i=42,p=1"));
    let restored = render(&mut presentation, false);
    assert!(restored.contains("a=p,i=42,p=1"));
    assert!(!restored.contains("a=t,"));
}

#[test]
fn native_kitty_uploads_preserve_pixels_compression_and_stable_redraws() {
    for compressed in [false, true] {
        let backend = Backend {
            font: FontSize::new(10, 20),
            protocol: ProtocolType::Kitty,
            tmux: false,
            kitty_compression: compressed,
        };
        let size = Size::new(80, 24);
        let pixels = image::RgbaImage::from_fn(800, 480, |x, y| {
            if x < 400 && y < 240 {
                image::Rgba([255, 0, 0, 128])
            } else {
                image::Rgba([248, 250, 252, 255])
            }
        });
        let encoded = backend
            .encode_preview(DynamicImage::ImageRgba8(pixels.clone()), size, 77)
            .unwrap();
        assert_eq!(encoded.size(), size);
        let area = Rect::new(2, 1, size.width, size.height);
        let bounds = Rect::new(0, 0, 90, 30);
        let mut presentation = Presentation::default();
        presentation.rectangular(true);
        let mut decoder = observer::KittyFrames::default();
        for draw in 0..2 {
            let mut buffer = Buffer::empty(bounds);
            presentation.begin();
            presentation.render(&mut buffer, area, &encoded);
            presentation.finish(&mut buffer, false);
            let wire = buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            if draw == 0 {
                assert!(wire.contains(if compressed {
                    "a=t,f=32,"
                } else {
                    "a=t,f=100,"
                }));
                assert_eq!(wire.contains("o=z"), compressed);
                assert!(
                    wire.len() < pixels.len() / 10,
                    "Flat design uploads should be far smaller than raw RGBA"
                );
                assert!(wire.find("a=t").unwrap() < wire.find("a=p").unwrap());
            } else {
                assert!(
                    !wire.contains("_G"),
                    "Stable draws must not retransmit or reposition the image"
                );
            }
            decoder.feed(wire.as_bytes()).unwrap();
        }
        assert_eq!(decoder.compressed_frames, usize::from(compressed));
        assert_eq!(decoder.images.len(), 1);
        assert_eq!(decoder.images[0].rgba(), pixels.as_raw());
        assert_eq!(presentation.source_size(), Some([800, 480]));
    }
}
