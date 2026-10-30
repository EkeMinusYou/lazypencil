use super::*;
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

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
