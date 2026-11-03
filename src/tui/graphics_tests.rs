use super::*;
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

#[cfg(unix)]
#[test]
fn background_queries_preserve_protocol_overrides_and_wrap_tmux() {
    for tmux in [false, true] {
        for capabilities in [false, true] {
            let query = probe_query(tmux, capabilities);
            assert!(query.contains("]11;?\x07"));
            assert_eq!(query.contains("_Gi=31"), capabilities);
            assert_eq!(query.contains("_Gi=32"), capabilities);
            assert_eq!(query.contains("[16t"), capabilities);
            assert_eq!(query.starts_with("\x1bPtmux;"), tmux);
            if tmux {
                assert!(query.contains("\x1b\x1b]11;?"));
                assert!(query.ends_with("\x1b\\"));
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn background_replies_accept_both_terminators_and_rgb_precisions_without_leaking_into_input() {
    for color in ["1/2/3", "11/22/33", "111/222/333", "1111/2222/3333"] {
        for terminator in ["\x07", "\x1b\\"] {
            let bytes = format!("日本\x1b[0n\x1b]11;rgb:{color}{terminator}語の下書き");
            assert_eq!(
                background_response(bytes.as_bytes()),
                Some(Response::Background(17, 34, 51))
            );
            assert_eq!(startup_text(bytes.as_bytes()), "日本語の下書き");
        }
    }
    for color in [
        "",
        "11/22",
        "11/22/33/44",
        "zz/22/33",
        "12345/22/33",
        "+1/22/33",
    ] {
        let bytes = format!("draft\x1b]11;rgb:{color}\x07text");
        assert_eq!(background_response(bytes.as_bytes()), None);
        assert_eq!(startup_text(bytes.as_bytes()), "drafttext");
    }
    for bytes in [
        b"draft\x1b]11;rgb:11/22/33".as_slice(),
        b"draft\x1b]11;rgb:11/22",
    ] {
        assert_eq!(background_response(bytes), None);
        assert_eq!(startup_text(bytes), "draft");
    }
    assert_eq!(background_response(b"ordinary input\x1b[0n"), None);
    assert_eq!(background_response(b"\x1b]10;rgb:1111/2222/3333\x07"), None);
}

#[test]
fn canvas_background_uses_terminal_rgb_for_opaque_protocols_and_alpha_for_kitty() {
    let fallback = image::Rgba([248, 250, 252, 255]);
    assert_eq!(Backend::halfblocks().canvas_background(), fallback);
    for protocol in [
        ProtocolType::Halfblocks,
        ProtocolType::Sixel,
        ProtocolType::Kitty,
    ] {
        for background in [fallback, image::Rgba([17, 34, 51, 255])] {
            let backend = Backend {
                protocol,
                background,
                ..Backend::halfblocks()
            };
            assert_eq!(
                backend.canvas_background(),
                if protocol == ProtocolType::Kitty {
                    image::Rgba([0, 0, 0, 0])
                } else {
                    background
                }
            );
        }
    }
}

#[test]
fn preview_worker_changes_only_the_canvas_and_preserves_explicit_document_fills() {
    use crate::{
        store::Store,
        tui::preview::{Request, View, Worker},
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("background.pen");
    let document = serde_json::json!({"version":"2.21","children":[
        {"id":"white","name":" ","type":"rectangle","width":20,"height":20,"fill":"#FFFFFF"},
        {"id":"translucent","name":" ","type":"rectangle","x":30,"width":20,"height":20,"fill":"#FF0000","opacity":0.5}
    ]}).to_string();
    std::fs::write(&path, &document).unwrap();
    let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for protocol in [
            ProtocolType::Kitty,
            ProtocolType::Sixel,
            ProtocolType::Halfblocks,
        ] {
            let backend = Backend {
                protocol,
                font: FontSize::new(1, 1),
                background: image::Rgba([17, 34, 51, 255]),
                ..Backend::halfblocks()
            };
            let background = backend.canvas_background();
            let worker = Worker::start(store.clone(), vec![], backend, 194).unwrap();
            worker.request(Request {
                generation: 1,
                cells: Size::new(80, 60),
                view: View {
                    center: [20.0, 20.0],
                    zoom: Some(1.0),
                    time: 0.0,
                },
                refresh: false,
                refresh_pages: false,
                check_only: false,
                attachment: None,
            });
            let output = tokio::time::timeout(std::time::Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            let preview = output.result.unwrap();
            let pixels = preview.image.bitmap().unwrap();
            assert_eq!(*pixels.get_pixel(0, 0), background);
            assert_eq!(*pixels.get_pixel(25, 15), image::Rgba([255, 255, 255, 255]));
            let translucent = pixels.get_pixel(55, 15).0;
            if protocol == ProtocolType::Kitty {
                assert_eq!(translucent, [255, 0, 0, 128]);
            } else {
                assert_eq!(translucent[3], 255);
                assert!(translucent[0] > 120 && translucent[1] < 34 && translucent[2] < 51);
            }
        }
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), document);
    let (project, layout) = crate::service::layout_snapshot(&store, &[], None, None).unwrap();
    let export = crate::render::png(
        &project,
        &layout,
        &crate::render::RenderOptions {
            region: Some(crate::geometry::Rect::new(-20.0, -10.0, 80.0, 60.0)),
            ..Default::default()
        },
    )
    .unwrap();
    let png = image::load_from_memory(&export.png).unwrap().into_rgba8();
    assert_eq!(
        png.get_pixel(0, 0).0[3],
        0,
        "PNG exports keep their transparent canvas"
    );
    assert_eq!(png.get_pixel(25, 15).0, [255, 255, 255, 255]);
}

#[test]
fn native_kitty_resize_and_upload_keep_canvas_transparent_and_artwork_opaque() {
    let cells = Size::new(8, 4);
    let area = Rect::new(0, 0, cells.width, cells.height);
    for compressed in [false, true] {
        let backend = Backend {
            protocol: ProtocolType::Kitty,
            kitty_compression: compressed,
            font: FontSize::new(1, 1),
            ..Backend::halfblocks()
        };
        // A square source leaves padding in the wide terminal placement.
        let source = image::RgbaImage::from_fn(2, 2, |x, _| {
            if x == 0 {
                image::Rgba([0, 0, 0, 0])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        let encoded = backend.encode_preview(source.into(), cells, 77).unwrap();
        let mut presentation = Presentation::default();
        presentation.rectangular(true);
        let mut buffer = Buffer::empty(area);
        presentation.begin();
        presentation.render(&mut buffer, area, &encoded);
        presentation.finish(&mut buffer, false);
        let mut decoder = observer::KittyFrames::default();
        decoder.feed(buffer[(0, 0)].symbol().as_bytes()).unwrap();
        assert_eq!(decoder.images.len(), 1);
        let pixels = decoder.images[0].rgba();
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] == 0));
        assert!(
            pixels
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 255, 255, 255])
        );
        assert_eq!(&pixels[(8 - 1) * 4..8 * 4], &[0, 0, 0, 0]);
        assert_eq!(buffer[(7, 3)].bg, ratatui::style::Color::Reset);
    }
}

#[test]
fn oversized_images_are_bounded_without_shrinking_the_kitty_placement() {
    let cells = Size::new(90, 2);
    let area = Rect::new(0, 0, cells.width, cells.height);
    for compressed in [false, true] {
        let backend = Backend {
            font: FontSize::new(100, 20),
            protocol: ProtocolType::Kitty,
            tmux: false,
            kitty_compression: compressed,
            ..Backend::halfblocks()
        };
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            20,
            20,
            image::Rgba([255, 0, 0, 255]),
        ));
        let encoded = backend.encode_preview(image, cells, 77).unwrap();
        assert_eq!(encoded.size(), cells);
        match encoded {
            Encoded::Native(image) => {
                assert!(image.pixels[0] <= super::super::viewport::MAX_SIDE);
                assert!(image.pixels[0] < 9000);
                let mut presentation = Presentation::default();
                presentation.rectangular(true);
                let mut buffer = Buffer::empty(area);
                presentation.begin();
                presentation.render(&mut buffer, area, &Encoded::Native(image));
                presentation.finish(&mut buffer, false);
                let wire = buffer[(0, 0)].symbol();
                assert!(wire.contains("c=90,r=2"));
            }
            Encoded::Protocol(protocol) => {
                let mut buffer = Buffer::empty(area);
                Image(&protocol).render(area, &mut buffer);
                let wire = buffer[(0, 0)].symbol();
                assert!(wire.contains("c=90,r=2"));
                let mut decoder = observer::KittyFrames::default();
                decoder.feed(wire.as_bytes()).unwrap();
                assert_eq!(decoder.images.len(), 1);
                assert!(decoder.images[0].width <= super::super::viewport::MAX_SIDE as usize);
                assert!(buffer[(89, 1)].symbol().contains('\u{10EEEE}'));
            }
        }
    }
    let backend = Backend {
        font: FontSize::new(100, 20),
        protocol: ProtocolType::Sixel,
        tmux: false,
        kitty_compression: false,
        ..Backend::halfblocks()
    };
    let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        20,
        20,
        image::Rgba([255, 0, 0, 255]),
    ));
    let protocol = backend.encode(image, cells, 77).unwrap();
    assert!(matches!(protocol, Protocol::Halfblocks(_)));
    assert_eq!(protocol.size(), cells);
}

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
                    ..Backend::halfblocks()
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
            ..Backend::halfblocks()
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
        ..Backend::halfblocks()
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
        ..Backend::halfblocks()
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
        assert_eq!(buffer[(area.x, area.y)].bg, ratatui::style::Color::Reset);
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
            ..Backend::halfblocks()
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

#[test]
fn native_kitty_resizes_keep_completed_frames_at_their_original_scale() {
    for compressed in [false, true] {
        let backend = Backend {
            font: FontSize::new(10, 20),
            protocol: ProtocolType::Kitty,
            tmux: false,
            kitty_compression: compressed,
            ..Backend::halfblocks()
        };
        let pixels = image::RgbaImage::from_fn(510, 640, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 56, 255])
        });
        let encoded = backend
            .encode_preview(
                DynamicImage::ImageRgba8(pixels.clone()),
                Size::new(51, 32),
                42,
            )
            .unwrap();
        let mut presentation = Presentation::default();
        presentation.rectangular(true);
        let mut observer = observer::KittyFrames::for_lazypencil();
        for (index, (area, position, size, crop)) in [
            // The initial viewport, Nodes hidden, then both side panels hidden.
            (
                Rect::new(23, 2, 51, 32),
                [24, 3],
                [51, 32],
                [0, 0, 510, 640],
            ),
            (Rect::new(1, 2, 73, 32), [13, 3], [51, 32], [0, 0, 510, 640]),
            (Rect::new(1, 2, 98, 32), [25, 3], [51, 32], [0, 0, 510, 640]),
            // A narrower and shorter viewport clips the center without scaling.
            (
                Rect::new(23, 2, 31, 20),
                [24, 3],
                [31, 20],
                [100, 120, 310, 400],
            ),
            // Restoring the viewport restores the whole retained image.
            (
                Rect::new(23, 2, 51, 32),
                [24, 3],
                [51, 32],
                [0, 0, 510, 640],
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 36));
            presentation.begin();
            presentation.render(&mut buffer, area, &encoded);
            presentation.finish(&mut buffer, false);
            let wire = buffer[(0, 0)].symbol();
            assert_eq!(wire.contains("a=t,"), index == 0);
            assert!(wire.contains(&format!("\x1b[{};{}H", position[1], position[0])));
            assert!(wire.contains(&format!("a=p,i=42,p=1,c={},r={},", size[0], size[1])));
            assert!(wire.contains(&format!(
                ",x={},y={},w={},h={};",
                crop[0], crop[1], crop[2], crop[3]
            )));
            assert_eq!(presentation.source_size(), Some([510, 640]));
            observer.feed(wire.as_bytes()).unwrap();
            assert_eq!(observer.images.len(), 1);
        }
        assert_eq!(observer.images[0].rgba(), pixels.as_raw());

        let completed = backend
            .encode_preview(
                DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    980,
                    640,
                    image::Rgba([12, 34, 56, 255]),
                )),
                Size::new(98, 32),
                42,
            )
            .unwrap();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 36));
        presentation.begin();
        presentation.render(&mut buffer, Rect::new(1, 2, 98, 32), &completed);
        presentation.finish(&mut buffer, false);
        let wire = buffer[(0, 0)].symbol();
        assert!(wire.contains("a=t,"));
        assert!(wire.contains("a=p,i=43,p=1,c=98,r=32,"));
        assert!(wire.contains(",x=0,y=0,w=980,h=640;"));
        assert!(wire.contains("a=d,d=i,i=42,p=1"));
        assert!(wire.find("a=t,").unwrap() < wire.find("a=p,").unwrap());
        assert!(wire.find("a=p,").unwrap() < wire.find("a=d,").unwrap());
        observer.feed(wire.as_bytes()).unwrap();
        assert_eq!(observer.images.len(), 2);
        assert_eq!(
            (observer.images[1].width, observer.images[1].height),
            (980, 640)
        );
    }
}

#[test]
fn native_menu_preserves_uncovered_pixels_and_restores_full_placement() {
    let backend = Backend {
        font: FontSize::new(10, 20),
        protocol: ProtocolType::Kitty,
        tmux: false,
        kitty_compression: false,
        ..Backend::halfblocks()
    };
    let area = Rect::new(2, 1, 8, 4);
    let bounds = Rect::new(0, 0, 16, 8);
    let encoded = backend
        .encode_preview(
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                80,
                80,
                image::Rgba([255, 0, 0, 255]),
            )),
            area.as_size(),
            42,
        )
        .unwrap();
    let mut presentation = Presentation::default();
    presentation.rectangular(true);
    let render = |presentation: &mut Presentation, overlay| {
        presentation.begin();
        let mut buffer = Buffer::empty(bounds);
        presentation.render(&mut buffer, area, &encoded);
        presentation.finish_with_overlay(&mut buffer, false, overlay);
        buffer[(0, 0)].symbol().to_owned()
    };
    assert!(render(&mut presentation, None).contains("a=t,"));
    let wire = render(&mut presentation, Some(Rect::new(5, 2, 2, 2)));
    assert_eq!(presentation.shown.len(), 4);
    assert_eq!(
        presentation
            .shown
            .iter()
            .map(|p| p.area.area())
            .sum::<u32>(),
        area.area() - 4
    );
    assert_eq!(presentation.shown[2].crop, Some([0, 20, 30, 40]));
    assert_eq!(presentation.shown[3].crop, Some([50, 20, 30, 40]));
    assert_eq!(wire.matches("a=p,").count(), 4);
    assert!(!wire.contains("a=t,"));
    assert_eq!(render(&mut presentation, Some(Rect::new(5, 2, 2, 2))), " ");
    let restored = render(&mut presentation, None);
    assert_eq!(presentation.shown.len(), 1);
    assert_eq!(presentation.shown[0].area, area);
    assert_eq!(restored.matches("a=d,").count(), 3);
    let outside = render(&mut presentation, Some(Rect::new(12, 1, 2, 2)));
    assert_eq!(
        outside, " ",
        "A menu outside Preview must not delete its image"
    );
}

#[test]
fn native_selection_frames_restore_original_pixels_after_deselection() {
    let backend = Backend {
        font: FontSize::new(10, 20),
        protocol: ProtocolType::Kitty,
        tmux: false,
        kitty_compression: false,
        ..Backend::halfblocks()
    };
    let size = Size::new(8, 4);
    let area = Rect::new(1, 1, 8, 4);
    let bounds = Rect::new(0, 0, 12, 8);
    let encode = |color, id| {
        backend
            .encode_preview(
                DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(80, 80, image::Rgba(color))),
                size,
                id,
            )
            .unwrap()
    };
    let original = encode([255, 0, 0, 255], 42);
    let selected = encode([59, 130, 246, 255], 42 ^ 0x8000_0000);
    let mut presentation = Presentation::default();
    presentation.rectangular(true);
    let mut observer = observer::KittyFrames::for_lazypencil();
    for image in [&original, &selected, &original] {
        let mut buffer = Buffer::empty(bounds);
        presentation.begin();
        presentation.render(&mut buffer, area, image);
        presentation.finish(&mut buffer, false);
        observer.feed(buffer[(0, 0)].symbol().as_bytes()).unwrap();
    }
    assert_eq!(observer.images.len(), 2);
    assert_eq!(observer.camera_frames, 1);
    assert!(observer.images.last().unwrap().has_color([255, 0, 0, 255]));
}
