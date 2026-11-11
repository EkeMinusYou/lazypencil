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
fn canvas_background_uses_terminal_rgb_for_every_protocol() {
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
            assert_eq!(backend.canvas_background(), background);
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
            assert_eq!(translucent[3], 255);
            assert!(translucent[0] > 120 && translucent[1] < 34 && translucent[2] < 51);
            for (generation, background) in [(2, [250, 250, 252, 255]), (3, [30, 30, 46, 255])] {
                worker.set_canvas_background(image::Rgba(background));
                worker.request(Request {
                    generation,
                    cells: Size::new(80, 60),
                    view: preview.view,
                    refresh: false,
                    refresh_pages: false,
                    check_only: true,
                    attachment: None,
                });
                let updated =
                    tokio::time::timeout(std::time::Duration::from_secs(15), worker.next())
                        .await
                        .unwrap()
                        .result
                        .unwrap();
                let pixels = updated.image.bitmap().unwrap();
                assert_eq!(pixels.get_pixel(0, 0).0, background);
                assert_eq!(pixels.get_pixel(25, 15).0, [255, 255, 255, 255]);
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
fn native_kitty_resize_and_upload_use_terminal_background_for_padding() {
    let cells = Size::new(8, 4);
    let area = Rect::new(0, 0, cells.width, cells.height);
    for compressed in [false, true] {
        let backend = Backend {
            protocol: ProtocolType::Kitty,
            kitty_compression: compressed,
            font: FontSize::new(1, 1),
            background: image::Rgba([17, 34, 51, 255]),
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
        let (rgba, _) = pixels.as_chunks::<4>();
        assert!(rgba.iter().all(|pixel| pixel[3] == 255));
        assert!(rgba.contains(&[17, 34, 51, 255]));
        assert!(rgba.contains(&[255, 255, 255, 255]));
        assert_eq!(&pixels[(8 - 1) * 4..8 * 4], &[17, 34, 51, 255]);
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
fn native_kitty_rgb_omits_only_fully_opaque_alpha_without_changing_pixels() {
    for compressed in [false, true] {
        for alpha in [255, 254] {
            let backend = Backend {
                font: FontSize::new(1, 1),
                protocol: ProtocolType::Kitty,
                tmux: false,
                kitty_compression: compressed,
                ..Backend::halfblocks()
            };
            let pixels = image::RgbaImage::from_fn(32, 16, |x, y| {
                image::Rgba([x as u8 * 7, y as u8 * 13, 93, alpha])
            });
            let Encoded::Native(encoded) = backend
                .encode_preview(
                    DynamicImage::ImageRgba8(pixels.clone()),
                    Size::new(32, 16),
                    77,
                )
                .unwrap()
            else {
                panic!("Expected native frame")
            };
            assert!(encoded.upload.contains(if !compressed {
                "f=100,"
            } else if alpha == 255 {
                "f=24,"
            } else {
                "f=32,"
            }));
            let mut decoder = observer::KittyFrames::default();
            for chunk in encoded.upload.as_bytes().chunks(137) {
                decoder.feed(chunk).unwrap();
            }
            assert_eq!(decoder.images.len(), 1);
            assert_eq!(decoder.images[0].rgba(), pixels.as_raw());
        }
    }
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
        let replay = encoded.clone();
        let mut buffer = Buffer::empty(bounds);
        presentation.begin();
        presentation.render(&mut buffer, area, &replay);
        presentation.finish(&mut buffer, false);
        let wire = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            wire.contains("a=t,"),
            "Revisited frames must upload into the inactive slot"
        );
        decoder.feed(wire.as_bytes()).unwrap();
        assert_eq!(decoder.images.len(), 2);
        assert_eq!(decoder.images[1].rgba(), pixels.as_raw());
    }
}

#[test]
fn local_kitty_upload_links_preserve_pixels_after_cached_sources_are_evicted() {
    for compressed in [false, true] {
        for alpha in [255, 254] {
            let local = Arc::new(LocalTransfer::new().unwrap());
            let backend = Backend {
                font: FontSize::new(1, 1),
                protocol: ProtocolType::Kitty,
                kitty_compression: compressed,
                local_transfer: Some(local.clone()),
                ..Backend::halfblocks()
            };
            let pixels =
                image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8, y as u8, 56, alpha]));
            let Encoded::Native(encoded) = backend
                .encode_preview(
                    DynamicImage::ImageRgba8(pixels.clone()),
                    Size::new(3, 2),
                    42,
                )
                .unwrap()
            else {
                panic!("Expected a native image")
            };
            local.drain();
            let first = encoded.upload(42);
            let second = encoded.clone().upload(43);
            assert!(first.contains(",t=t;"));
            if compressed {
                assert!(first.contains(",f=32,"));
                assert!(!first.contains(",o=z,"));
            }
            assert!(second.contains("i=43,"));
            assert!(!first.contains("m=1"));
            drop(encoded);
            let mut decoder = observer::KittyFrames::default();
            decoder.allow_local_files = true;
            decoder.feed(first.as_bytes()).unwrap();
            decoder.feed(second.as_bytes()).unwrap();
            assert_eq!(decoder.local_file_frames, 2);
            assert_eq!(decoder.images.len(), 2);
            assert!(
                decoder
                    .images
                    .iter()
                    .all(|frame| frame.rgba() == pixels.as_raw())
            );
            local.drain();
            assert_eq!(
                std::fs::read_dir(local.directory.path()).unwrap().count(),
                0
            );
        }
    }
}

#[test]
fn evicted_local_transfer_files_fall_back_to_lossless_inline_pixels() {
    for compressed in [false, true] {
        let local = Arc::new(LocalTransfer::new().unwrap());
        let backend = Backend {
            font: FontSize::new(1, 1),
            protocol: ProtocolType::Kitty,
            kitty_compression: compressed,
            local_transfer: Some(local.clone()),
            ..Backend::halfblocks()
        };
        let pixels = image::RgbaImage::from_pixel(80, 64, image::Rgba([12, 34, 56, 127]));
        let Encoded::Native(encoded) = backend
            .encode_preview(pixels.clone().into(), Size::new(80, 64), 42)
            .unwrap()
        else {
            panic!("Expected a native image")
        };
        // Removing the original reproduces eviction while the encoded source lives.
        local.drain();
        for entry in std::fs::read_dir(local.directory.path()).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        let wire = encoded.upload(43);
        assert!(!wire.contains(",t=t;"));
        let mut decoder = observer::KittyFrames::default();
        decoder.feed(wire.as_bytes()).unwrap();
        assert_eq!(decoder.images.len(), 1);
        assert_eq!(decoder.images[0].rgba(), pixels.as_raw());
    }
}

#[test]
fn native_image_placements_preserve_the_canvas_background_without_terminal_images() {
    let area = Rect::new(0, 0, 3, 2);
    let background = ratatui::style::Color::Rgb(17, 34, 51);
    let backend = Backend {
        font: FontSize::new(1, 1),
        ..Backend::native_for_test()
    };
    let image = backend
        .encode_preview(
            image::RgbaImage::from_pixel(3, 2, image::Rgba([12, 34, 56, 255])).into(),
            Size::new(3, 2),
            42,
        )
        .unwrap();
    let mut buffer = Buffer::empty(area);
    for cell in &mut buffer.content {
        cell.set_bg(background).set_symbol("old");
    }
    let mut presentation = Presentation::default();
    presentation.rectangular(true);
    presentation.begin();
    presentation.render(&mut buffer, area, &image);
    presentation.finish(&mut buffer, false);
    assert!(buffer.content.iter().all(|cell| cell.bg == background));
    assert!(
        buffer
            .content
            .iter()
            .all(|cell| !cell.symbol().contains("old"))
    );
}

#[test]
fn local_file_creation_failure_preserves_the_negotiated_inline_pixels() {
    for compressed in [false, true] {
        let local = Arc::new(LocalTransfer::new().unwrap());
        std::fs::remove_dir_all(local.directory.path()).unwrap();
        let pixels = image::RgbaImage::from_pixel(80, 64, image::Rgba([12, 34, 56, 127]));
        let encoded = NativeImage::new(
            pixels.clone().into(),
            Size::new(80, 64),
            42,
            compressed,
            Some(&local),
        )
        .unwrap();
        assert!(encoded.raw_upload.is_none());
        let wire = encoded.upload(43);
        assert!(!wire.contains(",t=t;"));
        assert_eq!(wire.contains("o=z"), compressed);
        let mut decoder = observer::KittyFrames::default();
        decoder.feed(wire.as_bytes()).unwrap();
        assert_eq!(decoder.images[0].rgba(), pixels.as_raw());
    }
}

#[test]
fn local_kitty_upload_failure_falls_back_to_the_exact_inline_pixels() {
    let local = Arc::new(LocalTransfer::new().unwrap());
    let pixels = image::RgbaImage::from_pixel(3, 2, image::Rgba([12, 34, 56, 255]));
    let encoded = NativeImage::new(
        DynamicImage::ImageRgba8(pixels.clone()),
        Size::new(3, 2),
        42,
        true,
        Some(&local),
    )
    .unwrap();
    local.drain();
    std::fs::remove_dir_all(local.directory.path()).unwrap();
    let wire = encoded.upload(43);
    assert!(!wire.contains(",t=t;"));
    let mut decoder = observer::KittyFrames::default();
    decoder.feed(wire.as_bytes()).unwrap();
    assert_eq!(decoder.images[0].rgba(), pixels.as_raw());
}

#[cfg(unix)]
#[test]
fn local_kitty_probe_requires_its_own_positive_response() {
    assert!(local_transfer_response(
        b"typed\x1b_Gi=2147483645;OK\x1b\\tail"
    ));
    assert!(!local_transfer_response(b"\x1b_Gi=1;OK\x1b\\"));
    assert!(!local_transfer_response(b"\x1b_Gi=2147483645;ENOENT\x1b\\"));
    assert!(!local_transfer_response(b"\x1b_Gi=2147483645;OK"));
    assert_eq!(
        startup_text(b"hi\x1b_Gi=2147483645;OK\x1b\\there"),
        "hithere"
    );
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
fn native_selection_is_composed_with_cached_and_fresh_camera_frames() {
    use super::super::{preview, selection::render::State};
    let directory = tempfile::TempDir::new().unwrap();
    let store = crate::store::Store::init(
        directory.path().join("design.pen"),
        Some(directory.path().join("data")),
    )
    .unwrap();
    std::fs::write(
        store.path(),
        serde_json::json!({"version":"2.21","children":[{
            "id":"shape", "type":"rectangle", "width":100, "height":80, "fill":"#ff0000"
        }]})
        .to_string(),
    )
    .unwrap();
    let backend = Backend {
        font: FontSize::new(10, 20),
        protocol: ProtocolType::Kitty,
        tmux: false,
        kitty_compression: false,
        ..Backend::halfblocks()
    };
    let worker = preview::Worker::start(store, vec![], backend, 42).unwrap();
    worker.set_selection(State {
        selected: vec!["shape".into()],
        marquee: None,
    });
    let size = Size::new(32, 16);
    let area = Rect::new(0, 0, size.width, size.height);
    let mut view = preview::View {
        center: [50.0, 40.0],
        zoom: Some(1.0),
        time: 0.0,
    };
    let mut presentation = Presentation::default();
    presentation.rectangular(true);
    let mut observer = observer::KittyFrames::for_lazypencil();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for generation in 1..=4 {
                match generation {
                    2 => view.pan([64.0, 0.0]),
                    3 => view.pan([7.0, -9.0]),
                    4 => view.zoom = Some(1.25),
                    _ => {}
                }
                worker.request(preview::Request {
                    generation,
                    cells: size,
                    view,
                    refresh: false,
                    refresh_pages: false,
                    check_only: false,
                    attachment: None,
                });
                let frame = tokio::time::timeout(std::time::Duration::from_secs(15), worker.next())
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                if generation == 2 {
                    assert!(
                        frame.frame_time.is_zero(),
                        "Selection must preserve prepared camera cache hits"
                    );
                }
                let original = frame.image.bitmap().unwrap();
                assert!(
                    !original
                        .pixels()
                        .any(|pixel| pixel.0 == [137, 180, 250, 255]),
                    "Exported scene pixels stay undecorated"
                );
                let selected = frame.selection.unwrap();
                let mut buffer = Buffer::empty(area);
                presentation.begin();
                presentation.render(&mut buffer, area, &selected.image);
                presentation.finish(&mut buffer, false);
                observer.feed(buffer[(0, 0)].symbol().as_bytes()).unwrap();
                assert_eq!(
                    observer.images.len(),
                    generation as usize,
                    "Each completed viewport needs only one upload"
                );
                assert!(
                    observer
                        .images
                        .last()
                        .unwrap()
                        .has_color([137, 180, 250, 255])
                );
                assert!(
                    presentation.source_size().is_some(),
                    "Selected exact frames must remain eligible for progressive navigation"
                );
                assert_eq!(observer.camera_frames, 0);
            }
        });
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
