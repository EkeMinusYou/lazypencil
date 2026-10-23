#![cfg(feature = "rendering")]

use lazypencil::{
    document::Document,
    geometry::Rect,
    layout,
    project::Project,
    render::{self, RenderOptions},
    text::Fonts,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::{fs, process::Command};
use tempfile::TempDir;

fn render(children: Value, options: RenderOptions) -> render::RenderedImage {
    try_render(children, options).unwrap()
}
fn try_render(
    children: Value,
    options: RenderOptions,
) -> lazypencil::Result<render::RenderedImage> {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("scene.pen");
    let source = json!({"version":"2.21","children":children}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[])?;
    let snapshot = layout::compute(project.resolve()?, &mut Fonts::new(&[]).unwrap())?;
    render::png(&project, &snapshot, &options)
}
fn raster(output: &render::RenderedImage) -> Image {
    Image::from_encoded(Data::new_copy(&output.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
}
fn pixel(output: &render::RenderedImage, x: i32, y: i32) -> Color {
    raster(output).peek_pixels().unwrap().get_color((x, y))
}
fn scene(radius: f64, alpha: &str) -> Value {
    let mut children=(0..30).map(|index|json!({"id":format!("stripe-{index}"),"type":"rectangle","x":index*4,"width":4,"height":80,
        "fill":format!("#{}{alpha}",if index%2==0 {"000000"} else {"FFFFFF"})})).collect::<Vec<_>>();
    children.push(json!({"id":"glass","type":"frame","layout":"none","x":20,"y":20,"width":80,"height":40,"cornerRadius":8,
        "effect":{"type":"background_blur","radius":radius},"children":[{"id":"sharp","type":"rectangle","x":30,"y":10,"width":10,"height":10,"fill":"#FF0000"}]}));
    json!([{"id":"canvas","type":"frame","layout":"none","width":120,"height":80,"children":children}])
}
fn assert_crop(
    full: &render::RenderedImage,
    crop: &render::RenderedImage,
    x: i32,
    y: i32,
    tolerance: i32,
) {
    let a = raster(full);
    let a = a.peek_pixels().unwrap();
    let b = raster(crop);
    let b = b.peek_pixels().unwrap();
    for j in 0..crop.height as i32 {
        for i in 0..crop.width as i32 {
            let a = a.get_color((x + i, y + j));
            let b = b.get_color((i, j));
            for (a, b) in [
                (a.a(), b.a()),
                (a.r(), b.r()),
                (a.g(), b.g()),
                (a.b(), b.b()),
            ] {
                assert!(
                    (a as i32 - b as i32).abs() <= tolerance,
                    "Mismatch at {i},{j}: {a} vs {b}"
                );
            }
        }
    }
}

#[test]
fn background_blur_softens_the_backdrop_but_keeps_foreground_children_sharp() {
    let output = render(scene(8.0, "FF"), Default::default());
    assert_eq!((output.width, output.height), (120, 80));
    assert_eq!(pixel(&output, 10, 40), Color::BLACK);
    assert_eq!(
        pixel(&output, 20, 20),
        Color::WHITE,
        "The rounded corner must retain the original backdrop"
    );
    let blurred = pixel(&output, 42, 40);
    assert!(blurred.r() > 90 && blurred.r() < 170, "{blurred:?}");
    assert_eq!(pixel(&output, 55, 35), Color::RED);
    assert!(output.dynamic_artifacts.is_empty());
}

#[test]
fn transparent_backgrounds_are_replaced_without_increasing_their_alpha() {
    for alpha in ["80", "40"] {
        let output = render(scene(8.0, alpha), Default::default());
        let original = if alpha == "80" { 128_i32 } else { 64_i32 };
        for point in [(42, 40), (20, 20), (21, 25), (98, 40), (10, 40)] {
            assert!(
                (pixel(&output, point.0, point.1).a() as i32 - original).abs() <= 1,
                "{point:?}"
            );
        }
    }
    let mut zero = scene(0.0, "80");
    zero[0]["children"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["children"] = json!([]);
    let output = render(zero.clone(), Default::default());
    zero[0]["children"].as_array_mut().unwrap().pop();
    let plain = render(zero, Default::default());
    assert_crop(&plain, &output, 0, 0, 1);
}

#[test]
fn parent_opacity_sees_the_external_backdrop_including_outside_parent_bounds() {
    let output = render(
        json!([
            {"id":"back","type":"rectangle","width":80,"height":60,"fill":"#0000FF"},
            {"id":"parent","type":"frame","layout":"none","x":20,"y":10,"width":40,"height":40,"fill":"#FF0000","opacity":0.5,"children":[
                {"id":"glass","type":"rectangle","width":40,"height":40,"effect":{"type":"background_blur","radius":8}}
            ]}
        ]),
        Default::default(),
    );
    let middle = pixel(&output, 40, 30);
    assert!(
        (middle.r() as i32 - 128).abs() < 2 && (middle.b() as i32 - 127).abs() < 2,
        "{middle:?}"
    );
    let edge = pixel(&output, 21, 30);
    assert!(edge.r() > 50 && edge.r() < 110, "{edge:?}");
    assert!(edge.b() > 145 && edge.b() < 205, "{edge:?}");
    assert_eq!(pixel(&output, 10, 30), Color::BLUE);
}

#[test]
fn translucent_backdrops_keep_alpha_through_multiple_parent_opacity_layers() {
    let output = render(
        json!([
            {"id":"back","type":"rectangle","width":80,"height":60,"fill":"#0000FF80"},
            {"id":"outer","type":"frame","layout":"none","x":10,"y":10,"width":60,"height":40,"opacity":0.5,"children":[
                {"id":"inner","type":"frame","layout":"none","width":60,"height":40,"opacity":0.5,"children":[
                    {"id":"glass","type":"rectangle","width":60,"height":40,"effect":{"type":"background_blur","radius":8}},
                    {"id":"label","type":"rectangle","x":20,"y":10,"width":10,"height":10,"fill":"#FFFFFF"}
                ]}
            ]}
        ]),
        Default::default(),
    );
    for point in [(20, 30), (11, 30), (65, 30)] {
        assert!(
            (pixel(&output, point.0, point.1).a() as i32 - 128).abs() <= 1,
            "{point:?}"
        );
    }
    let label = pixel(&output, 35, 25);
    assert!((label.a() as i32 - 160).abs() <= 2, "{label:?}");
    assert!((label.r() as i32 - 102).abs() <= 3, "{label:?}");
    assert_eq!(label.b(), 255);
}

#[test]
fn a_shadow_after_background_blur_uses_painted_alpha_for_its_new_pixels() {
    let output = render(
        json!([
            {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#0000FF80"},
            {"id":"glass","type":"rectangle","x":20,"y":10,"width":20,"height":20,"effect":[
                {"type":"background_blur","radius":0},
                {"type":"shadow","offset":{"x":20,"y":0},"blur":0,"color":"#FF000080"}
            ]}
        ]),
        Default::default(),
    );
    let shadow = pixel(&output, 45, 20);
    assert!((shadow.a() as i32 - 160).abs() <= 2, "{shadow:?}");
    assert!((shadow.r() as i32 - 102).abs() <= 3, "{shadow:?}");
    assert!((shadow.b() as i32 - 153).abs() <= 3, "{shadow:?}");
    assert_eq!(pixel(&output, 25, 20).a(), 128);
    assert_eq!(pixel(&output, 70, 20).a(), 128);
}

#[test]
fn parent_clipping_and_rotated_shapes_limit_the_backdrop_composite() {
    let mut value = scene(8.0, "FF");
    let glass = value[0]["children"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    glass["rotation"] = json!(15);
    glass["children"] = json!([]);
    let clipped = json!({"id":"clip","type":"frame","layout":"none","x":30,"y":20,"width":60,"height":40,"clip":true,"cornerRadius":12,"children":[glass.clone()]});
    value[0]["children"].as_array_mut().unwrap().pop();
    let plain = render(value.clone(), Default::default());
    value[0]["children"].as_array_mut().unwrap().push(clipped);
    let output = render(value, Default::default());
    for point in [(30, 20), (31, 21), (90, 40), (29, 45)] {
        assert_eq!(
            pixel(&output, point.0, point.1),
            pixel(&plain, point.0, point.1),
            "{point:?}"
        );
    }
    assert_ne!(pixel(&output, 60, 40), pixel(&plain, 60, 40));
}

#[test]
fn scaled_region_and_node_crops_keep_the_same_scene_context_and_pixels() {
    let children = scene(8.0, "80");
    let full = render(
        children.clone(),
        RenderOptions {
            scale: 2.0,
            ..Default::default()
        },
    );
    let crop = render(
        children.clone(),
        RenderOptions {
            scale: 2.0,
            region: Some(Rect::new(37.0, 23.0, 21.0, 17.0)),
            ..Default::default()
        },
    );
    assert_eq!((crop.width, crop.height), (42, 34));
    assert_crop(&full, &crop, 74, 46, 1);
    let selected = render(
        children,
        RenderOptions {
            scale: 2.0,
            node: Some("glass".into()),
            ..Default::default()
        },
    );
    assert_eq!((selected.width, selected.height), (160, 80));
    assert_crop(&full, &selected, 40, 40, 1);
}

#[test]
fn background_and_foreground_filters_follow_declared_order() {
    let mut children = scene(4.0, "FF");
    let glass = children[0]["children"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    glass["children"] = json!([]);
    glass["effect"] = json!([{"type":"background_blur","radius":4},{"type":"blur","radius":8}]);
    let background_first = render(children.clone(), Default::default());
    children[0]["children"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["effect"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let background_last = render(children, Default::default());
    assert_ne!(background_first.png, background_last.png);
    assert_eq!(pixel(&background_last, 19, 40), Color::BLACK);
    assert_ne!(pixel(&background_first, 19, 40), Color::BLACK);
}

#[test]
fn disabled_filters_skip_work_and_invalid_or_oversized_filters_fail_explicitly() {
    for (radius, code) in [
        (-1.0, "INVALID_DOCUMENT"),
        (1_000_000.0, "INVALID_ARGUMENT"),
    ] {
        assert_eq!(
            try_render(scene(radius, "FF"), Default::default())
                .err()
                .unwrap()
                .code,
            code
        );
    }
    let mut children = scene(1_000_000.0, "FF");
    children[0]["children"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["effect"]["enabled"] = json!(false);
    assert!(try_render(children, Default::default()).is_ok());
}

#[test]
fn real_cli_background_blur_does_not_need_a_browser_installation() {
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("scene.pen");
    fs::write(
        &file,
        json!({"version":"2.21","children":scene(8.0,"80")}).to_string(),
    )
    .unwrap();
    let image = directory.path().join("preview.png");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .env(
            "LAZYPENCIL_CHROMIUM",
            directory.path().join("absent-chromium"),
        )
        .args(["--json", "render"])
        .arg(&file)
        .arg("--output")
        .arg(&image)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["width"], 120);
    assert!(result["dynamic_artifacts"].as_array().unwrap().is_empty());
    let image = Image::from_encoded(Data::new_copy(&fs::read(image).unwrap()))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert!((image.peek_pixels().unwrap().get_color((42, 40)).a() as i32 - 128).abs() <= 1);
    let crop = directory.path().join("crop.png");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .env(
            "LAZYPENCIL_CHROMIUM",
            directory.path().join("absent-chromium"),
        )
        .args(["--json", "render"])
        .arg(&file)
        .args(["--region", "-2", "0", "8", "8"])
        .arg("--output")
        .arg(&crop)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["region"]["x"], -2.0);
    assert_eq!(result["width"], 8);
    let image = Image::from_encoded(Data::new_copy(&fs::read(crop).unwrap()))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((0, 3)).a(), 0);
    assert_eq!(pixels.get_color((3, 3)).a(), 128);
}
