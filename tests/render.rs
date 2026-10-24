#![cfg(feature = "rendering")]

use lazypencil::{
    layout,
    project::Project,
    render::{self, RenderOptions},
    store::Store,
    text::Fonts,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::{fs, process::Command};
use tempfile::TempDir;

fn render_scene(
    children: Value,
    options: RenderOptions,
) -> lazypencil::Result<render::RenderedImage> {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("scene.pen");
    fs::write(
        &path,
        json!({"version":"2.21","children":children}).to_string(),
    )
    .unwrap();
    let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
    let project = Project::load(&store)?;
    let snapshot = layout::compute(project.resolve()?, &mut Fonts::new(&[]).unwrap())?;
    render::png(&project, &snapshot, &options)
}

fn pixel(png: &[u8], x: i32, y: i32) -> Color {
    let image = Image::from_encoded(Data::new_copy(png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    image.peek_pixels().unwrap().get_color((x, y))
}

#[test]
fn renders_reordered_fills_rotation_clipping_and_node_opacity() {
    let result=render_scene(json!([{"id":"frame","type":"frame","layout":"none","width":100,"height":100,"fill":"#FFF","clip":true,
        "children":[
            {"id":"rect","type":"rectangle","x":20,"y":20,"width":40,"height":40,"fill":["#F00",{"type":"color","color":"#0000FF80"}]},
            {"id":"outside","type":"rectangle","x":90,"y":0,"width":30,"height":10,"fill":"#0F0"},
            {"id":"rotated","type":"rectangle","x":80,"y":60,"width":20,"height":10,"rotation":90,"opacity":0.5,"fill":"#000"},
            {"id":"disabled","type":"rectangle","enabled":false,"width":100,"height":100,"fill":"#000"}
        ]}]),RenderOptions::default()).unwrap();
    assert_eq!((result.width, result.height), (100, 100));
    assert_eq!(pixel(&result.png, 5, 5), Color::WHITE);
    let mixed = pixel(&result.png, 30, 30);
    assert!((mixed.r() as i32 - 127).abs() <= 1);
    assert!((mixed.b() as i32 - 128).abs() <= 1);
    assert_eq!(pixel(&result.png, 95, 5), Color::GREEN);
    let faded = pixel(&result.png, 85, 50);
    assert!((faded.r() as i32 - 127).abs() <= 1);
    assert_eq!(faded.r(), faded.g());
}

#[test]
fn text_masks_paint_glyphs_without_painting_the_text_box() {
    let result = render_scene(
        json!([{"id":"text","type":"text","content":"日本語 ABC","fontSize":32,"fill":"#2563EB"}]),
        RenderOptions::default(),
    )
    .unwrap();
    let image = Image::from_encoded(Data::new_copy(&result.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let pixmap = image.peek_pixels().unwrap();
    let mut transparent = 0;
    let mut ink = 0;
    for y in 0..result.height as i32 {
        for x in 0..result.width as i32 {
            let color = pixmap.get_color((x, y));
            if color.a() == 0 {
                transparent += 1;
            }
            if color.a() > 200 {
                ink += 1;
                assert!((color.b() as i32 - 235).abs() <= 1);
            }
        }
    }
    assert!(transparent > ink);
    assert!(ink > 100);
}

#[test]
fn linear_and_radial_gradients_and_svg_geometry_render_actual_pixels() {
    let gradient = json!({"type":"gradient","gradientType":"linear","colors":[
        {"color":"#F00","position":0},{"color":"#00F","position":1}]});
    let result = render_scene(
        json!([{"id":"rect","type":"rectangle","width":100,"height":100,"fill":gradient}]),
        RenderOptions::default(),
    )
    .unwrap();
    assert!(pixel(&result.png, 50, 5).b() > 200);
    assert!(pixel(&result.png, 50, 95).r() > 200);
    let radial=render_scene(json!([{"id":"radial","type":"rectangle","width":100,"height":100,"fill":{
        "type":"gradient","gradientType":"radial","colors":[{"color":"#F00","position":0},{"color":"#00F","position":1}]
    }}]),RenderOptions::default()).unwrap();
    assert!(pixel(&radial.png, 50, 50).r() > 240);
    assert!(pixel(&radial.png, 1, 1).b() > 240);
    let path = render_scene(
        json!([{"id":"path","type":"path","width":100,"height":100,"viewBox":[0,0,10,10],
        "geometry":"M0 0 L10 0 L0 10 Z","fill":"#0F0"}]),
        RenderOptions::default(),
    )
    .unwrap();
    assert_eq!(pixel(&path.png, 10, 10), Color::GREEN);
    assert_eq!(pixel(&path.png, 90, 90).a(), 0);
}

#[test]
fn ring_ellipses_have_a_transparent_hole_and_inside_strokes_stay_inside() {
    let ring=render_scene(json!([{"id":"ring","type":"ellipse","width":100,"height":100,"innerRadius":0.5,"fill":"#F00"}]),RenderOptions::default()).unwrap();
    assert_eq!(pixel(&ring.png, 50, 50).a(), 0);
    assert_eq!(pixel(&ring.png, 50, 10), Color::RED);
    let border=render_scene(json!([{"id":"border","type":"rectangle","width":100,"height":100,"stroke":"#00F","strokeWidth":8,"strokeAlignment":"inner"}]),RenderOptions::default()).unwrap();
    assert_eq!((border.width, border.height), (100, 100));
    assert_eq!(pixel(&border.png, 2, 50), Color::BLUE);
    assert_eq!(pixel(&border.png, 50, 50).a(), 0);
}

#[test]
fn shadow_bounds_are_included_in_export_instead_of_cropping_the_effect() {
    let shadow = render_scene(
        json!([{"id":"shadow","type":"rectangle","width":20,"height":20,"fill":"#F00",
        "effect":{"type":"shadow","offset":{"x":20,"y":0},"blur":0,"color":"#00F"}}]),
        RenderOptions::default(),
    )
    .unwrap();
    assert_eq!((shadow.width, shadow.height), (40, 20));
    assert_eq!(pixel(&shadow.png, 5, 5), Color::RED);
    assert_eq!(pixel(&shadow.png, 25, 5), Color::BLUE);
    let blur = render_scene(
        json!([{"id":"blur","type":"rectangle","width":20,"height":20,"fill":"#F00",
        "effect":{"type":"blur","radius":4}}]),
        RenderOptions::default(),
    )
    .unwrap();
    assert_eq!((blur.width, blur.height), (32, 32));
    let edge = pixel(&blur.png, 5, 16);
    assert!(edge.a() > 0 && edge.a() < 255);
}

#[test]
fn image_fills_use_the_library_origin_and_track_the_image_hash() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("lib")).unwrap();
    let source = render_scene(
        json!([{"id":"source","type":"rectangle","width":4,"height":4,"fill":"#0F0"}]),
        RenderOptions::default(),
    )
    .unwrap();
    let asset = temp.path().join("lib/asset.png");
    fs::write(&asset, &source.png).unwrap();
    fs::write(temp.path().join("lib/ui.lib.pen"),json!({"version":"2.21","children":[
        {"id":"image","type":"rectangle","reusable":true,"width":20,"height":20,"fill":{"type":"image","url":"asset.png"}}
    ]}).to_string()).unwrap();
    let path = temp.path().join("scene.pen");
    fs::write(&path,json!({"version":"2.21","imports":{"ui":"lib/ui.lib.pen"},"children":[{"id":"instance","type":"ref","ref":"ui:image"}]}).to_string()).unwrap();
    let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
    let project = Project::load(&store).unwrap();
    let snapshot =
        layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap()).unwrap();
    let result = render::png(&project, &snapshot, &RenderOptions::default()).unwrap();
    assert_eq!(pixel(&result.png, 10, 10), Color::GREEN);
    assert_eq!(result.assets[0].path, asset.canonicalize().unwrap());
    assert_eq!(
        result.assets[0].hash,
        lazypencil::document::hash(&source.png)
    );
}

#[test]
fn render_limits_and_missing_runtime_return_errors() {
    let children = json!([{"id":"large","type":"rectangle","width":1000000,"height":1000000}]);
    assert_eq!(
        render_scene(children, RenderOptions::default())
            .err()
            .unwrap()
            .code,
        "INVALID_ARGUMENT"
    );
    let browser = json!([{"id":"browser","type":"browser","width":100,"height":100,"url":"https://example.com"}]);
    assert_eq!(
        render_scene(browser, RenderOptions::default())
            .err()
            .unwrap()
            .code,
        "UNSUPPORTED_FEATURE"
    );
}

#[test]
fn the_native_render_entry_accepts_ellipse_side_widths_and_keeps_top_width_bounds() {
    let unequal_ellipse = json!([{"id":"ellipse","type":"ellipse","width":100,"height":100,"stroke":"#123456","strokeWidth":{"top":2,"right":4,"bottom":6,"left":8}}]);
    let mut uniform_ellipse = unequal_ellipse.clone();
    uniform_ellipse[0]["strokeWidth"] = json!(2);
    let output = render_scene(unequal_ellipse, RenderOptions::default()).unwrap();
    let reference = render_scene(uniform_ellipse, RenderOptions::default()).unwrap();
    assert_eq!((output.width, output.height), (102, 102));
    assert_eq!(output.region, reference.region);
    assert_eq!(output.png, reference.png);
    assert!(pixel(&output.png, 51, 1).a() > 200);
}

#[test]
fn node_crops_retain_overlap_context_and_skip_unrelated_dynamic_nodes() {
    let output=render_scene(json!([
        {"id":"target","type":"rectangle","width":20,"height":20,"fill":"#FF0000"},
        {"id":"overlap","type":"rectangle","x":5,"y":5,"width":10,"height":10,"fill":"#0000FF"},
        {"id":"outside","type":"browser","x":1000,"y":1000,"width":100,"height":100,"url":"https://example.com"}
    ]),RenderOptions {node:Some("target".into()),..Default::default()}).unwrap();
    assert_eq!((output.width, output.height), (20, 20));
    assert_eq!(pixel(&output.png, 2, 2), Color::RED);
    assert_eq!(pixel(&output.png, 10, 10), Color::BLUE);
    assert!(output.dynamic_artifacts.is_empty());
}

#[test]
fn real_cli_renders_the_editable_example_and_reports_matching_layout_metrics() {
    let temp = TempDir::new().unwrap();
    let output = temp.path().join("welcome.png");
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/welcome.pen");
    let run = |args: &[&str]| {
        let result = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .arg("--json")
            .arg("--data-dir")
            .arg(temp.path().join("data"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice::<Value>(&result.stdout).unwrap()
    };
    let rendered = run(&[
        "render",
        example.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(rendered["width"], 960);
    assert_eq!(rendered["height"], 640);
    assert_eq!(
        pixel(&fs::read(&output).unwrap(), 5, 5),
        Color::from_rgb(248, 250, 252)
    );
    let layout = run(&["layout", example.to_str().unwrap()]);
    assert_eq!(layout["snapshot_id"], rendered["snapshot_id"]);
    assert!(layout["diagnostics"].as_array().unwrap().is_empty());
    assert!(
        layout["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == "body" && v["text"]["line_count"].as_u64().unwrap() > 1)
    );
}

#[test]
fn render_output_cannot_overwrite_an_input_image_through_a_case_alias() {
    let temp = TempDir::new().unwrap();
    let asset = temp.path().join("Asset.PNG");
    let source = render_scene(
        json!([{"id":"source","type":"rectangle","width":4,"height":4,"fill":"#0F0"}]),
        RenderOptions::default(),
    )
    .unwrap();
    fs::write(&asset, &source.png).unwrap();
    let alias = temp.path().join("asset.png");
    if !alias.exists() {
        return;
    }
    let document = temp.path().join("scene.pen");
    fs::write(&document,json!({"version":"2.21","children":[{"id":"image","type":"rectangle","width":20,"height":20,"fill":{"type":"image","url":"Asset.PNG"}}]}).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "render"])
        .arg(&document)
        .arg("--output")
        .arg(&alias)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(fs::read(asset).unwrap(), source.png);
}

#[cfg(unix)]
#[test]
fn an_unrepresentable_output_path_is_rejected_before_creating_a_file() {
    use std::os::unix::ffi::OsStringExt;
    let temp = TempDir::new().unwrap();
    let output_path = temp
        .path()
        .join(std::ffi::OsString::from_vec(b"output-\xff.png".to_vec()));
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/welcome.pen");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "render"])
        .arg(example)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "INVALID_ARGUMENT");
    assert!(!output_path.exists());
}
