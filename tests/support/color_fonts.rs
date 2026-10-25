use super::*;
use lazypencil::{layout, project::Project, render, store::Store};
use serde_json::json;
use skia_safe::{Color, Data, Image};
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

#[path = "fixture.rs"]
mod fixture_helper;

fn color_font(directory: &Path) -> PathBuf {
    let output = directory.join("color.ttf");
    let mut child = Command::new(fixture_helper::helper())
        .arg("font-fixture")
        .arg(&output)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("The deterministic color-font fixture exceeded its startup deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    output
}

fn palette_pixels(png: &[u8]) -> (usize, usize) {
    let image = Image::from_encoded(Data::new_copy(png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let pixels = image.peek_pixels().unwrap();
    let (mut red, mut green) = (0, 0);
    for y in 0..image.height() {
        for x in 0..image.width() {
            let color = pixels.get_color((x, y));
            if color == Color::RED {
                red += 1;
            } else if color == Color::GREEN {
                green += 1;
            }
        }
    }
    (red, green)
}

#[test]
fn original_color_font_shapes_zwj_modifiers_and_variation_selectors_before_painting() {
    let directory = tempfile::tempdir().unwrap();
    let file = color_font(directory.path());
    let mut fonts = Fonts::new(std::slice::from_ref(&file)).unwrap();
    for (content, glyph, colored) in [
        ("\u{1f600}", 1, true),
        ("\u{1f469}\u{200d}\u{1f4bb}", 1, true),
        ("\u{1f469}\u{1f3fb}", 8, true),
        ("\u{263a}\u{fe0f}", 1, true),
        ("\u{263a}\u{fe0e}", 9, false),
        ("\u{263a}", 9, false),
    ] {
        let mut request = spec(content, "LP Test Color");
        request.size = 100.0;
        request.color = Color::BLUE;
        let block = fonts.layout(&request, None).unwrap();
        let mut actual = Vec::new();
        block.paragraph.visit(|_, run| {
            if let Some(run) = run {
                actual.extend_from_slice(run.glyphs());
            }
        });
        assert_eq!(actual, [glyph], "{content}");
        assert!((block.metrics.width - 80.0).abs() < 0.1, "{content}");
        assert_eq!(block.metrics.fonts.len(), 1);
        assert_eq!(block.metrics.fonts[0].hash, hash(&fs::read(&file).unwrap()));
        let (red, green) = palette_pixels(&paint(&block));
        if colored {
            assert!(
                red > 1000 && green > 1000,
                "{content}: red={red}, green={green}"
            );
        } else {
            assert_eq!((red, green), (0, 0), "{content}");
        }
    }
}

#[test]
fn ordinary_render_preserves_intrinsic_color_glyphs_in_both_document_schemas() {
    let directory = tempfile::tempdir().unwrap();
    let font = color_font(directory.path());
    for version in ["2.19", "2.21"] {
        for content in [
            "\u{1f600}",
            "\u{1f469}\u{200d}\u{1f4bb}",
            "\u{1f469}\u{1f3fb}",
            "\u{263a}\u{fe0f}",
        ] {
            let file = directory.path().join("scene.pen");
            fs::write(&file, json!({"version":version,"children":[{
                "id":"page","type":"frame","layout":"none","width":160,"height":130,
                "children":[{"id":"emoji","type":"text","x":10,"y":10,"width":130,"height":110,
                    "textGrowth":"fixed-width-height","fontFamily":"LP Test Color","fontSize":100,
                    "content":content,"fill":"#0000ff"}]
            }]}).to_string()).unwrap();
            let store = Store::open(&file, Some(directory.path().join("data"))).unwrap();
            let project = Project::load(&store).unwrap();
            let snapshot = layout::compute(
                project.resolve().unwrap(),
                &mut Fonts::new(std::slice::from_ref(&font)).unwrap(),
            )
            .unwrap();
            let output = render::png(&project, &snapshot, &Default::default()).unwrap();
            let (red, green) = palette_pixels(&output.png);
            assert!(
                red > 1000 && green > 1000,
                "{version} {content}: red={red}, green={green}"
            );
        }
    }
}
