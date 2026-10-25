use super::*;
use lazypencil::{layout, project::Project, render, store::Store};
use serde_json::json;
use skia_safe::{Color, Data, Image};
use std::{
    process::{Child, ExitStatus, Stdio},
    time::{Duration, Instant},
};

#[path = "fixture.rs"]
mod fixture_helper;

fn color_font(directory: &Path) -> PathBuf {
    let output = directory.join("color.ttf");
    let child = Command::new(fixture_helper::helper())
        .arg("font-fixture")
        .arg(&output)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    assert!(completed(child, Duration::from_secs(10)).success());
    output
}

fn completed(mut child: Child, limit: Duration) -> ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Color-font verification process exceeded {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn render_scene(
    directory: &Path,
    font: &PathBuf,
    version: &str,
    properties: Value,
    options: &render::RenderOptions,
) -> (render::RenderedImage, PathBuf) {
    let mut node = json!({"id":"emoji","type":"text","x":10,"y":10,"width":360,"height":240,
        "textGrowth":"fixed-width-height","fontFamily":"LP Test Color","fontSize":100,
        "content":"\u{1f600}","fill":"#0000ff"});
    node.as_object_mut()
        .unwrap()
        .extend(properties.as_object().unwrap().clone());
    let file = directory.join("scene.pen");
    fs::write(
        &file,
        json!({"version":version,"children":[{
            "id":"page","type":"frame","layout":"none","width":400,"height":260,"children":[node]
        }]})
        .to_string(),
    )
    .unwrap();
    let store = Store::open(&file, Some(directory.join("data"))).unwrap();
    let project = Project::load(&store).unwrap();
    let snapshot = layout::compute(
        project.resolve().unwrap(),
        &mut Fonts::new(std::slice::from_ref(font)).unwrap(),
    )
    .unwrap();
    (render::png(&project, &snapshot, options).unwrap(), file)
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
        let mut block = fonts.layout(&request, None).unwrap();
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
            let (output, _) = render_scene(
                directory.path(),
                &font,
                version,
                json!({"content":content}),
                &Default::default(),
            );
            let (red, green) = palette_pixels(&output.png);
            assert!(
                red > 1000 && green > 1000,
                "{version} {content}: red={red}, green={green}"
            );
        }
    }
}

#[test]
fn color_glyph_paints_keep_native_palettes_with_fallbacks_decorations_and_compositing() {
    let directory = tempfile::tempdir().unwrap();
    let font = color_font(directory.path());
    let mut image = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
    image.canvas().clear(Color::BLUE);
    fs::write(
        directory.path().join("blue.png"),
        skia_safe::png_encoder::encode_image(None, &image.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    for version in ["2.19", "2.21"] {
        for properties in [
            json!({"content":"\u{263a}\u{1f600}"}),
            json!({"content":"日\u{1f600}"}),
            json!({"underline":true,"strikethrough":true}),
            json!({"fill":{"type":"gradient","gradientType":"linear","rotation":0,"colors":[{"position":0,"color":"#0000ff"},{"position":1,"color":"#00ffff"}]}}),
            json!({"fill":{"type":"image","url":"blue.png","mode":"cover"}}),
            json!({"fill":{"type":"mesh_gradient","columns":2,"rows":2,"colors":["#0000ff","#0000ff","#0000ff","#0000ff"]}}),
        ] {
            let (output, _) = render_scene(
                directory.path(),
                &font,
                version,
                properties.clone(),
                &Default::default(),
            );
            let (red, green) = palette_pixels(&output.png);
            assert!(
                red > 1000 && green > 1000,
                "{version} {properties}: red={red}, green={green}"
            );
        }
        let (output, _) = render_scene(
            directory.path(),
            &font,
            version,
            json!({"fill":"#0000ff80","opacity":0.5}),
            &Default::default(),
        );
        let image = Image::from_encoded(Data::new_copy(&output.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let pixel = image.peek_pixels().unwrap().get_color((30, 50));
        assert_eq!((pixel.r(), pixel.g(), pixel.b()), (255, 0, 0));
        assert!((pixel.a() as i32 - 64).abs() <= 1);
    }
}

#[test]
fn color_font_cli_uses_the_same_captured_faces_and_pixels_as_library_rendering() {
    let directory = tempfile::tempdir().unwrap();
    let font = color_font(directory.path());
    for version in ["2.19", "2.21"] {
        let (expected, document) = render_scene(
            directory.path(),
            &font,
            version,
            json!({"content":"日\u{1f469}\u{200d}\u{1f4bb}"}),
            &Default::default(),
        );
        let original = fs::read(&document).unwrap();
        let output = directory.path().join("cli.png");
        let report_path = directory.path().join("cli.json");
        let report = fs::File::create(&report_path).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .arg("--json")
            .arg("--data-dir")
            .arg(directory.path().join("cli-data"))
            .arg("render")
            .arg(&document)
            .arg("--font")
            .arg(&font)
            .arg("--output")
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(report)
            .spawn()
            .unwrap();
        assert!(completed(child, Duration::from_secs(30)).success());
        let report: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(report["diagnostics"], json!([]));
        assert_eq!(fs::read(output).unwrap(), expected.png);
        assert_eq!(fs::read(document).unwrap(), original);
    }
}
