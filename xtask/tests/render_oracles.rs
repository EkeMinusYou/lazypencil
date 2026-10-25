use lazypencil_xtask::fixtures::oracles::scripts;
use lazypencil_xtask::fixtures::oracles::shapes;
use serde_json::json;

#[test]
fn geometric_scan_conversion_agrees_with_independent_area_equations() {
    let circle = json!({"version":"2.19","children":[{"type":"frame","width":100,"height":100,"fill":"#ffffff","children":[{"type":"ellipse","x":25,"y":25,"width":50,"height":50,"fill":"#ff0000","stroke":"#0000ff","strokeWidth":6,"strokeAlignment":"center"}]}]});
    let image = shapes::raster(&circle, 1);
    let red = image
        .pixels
        .iter()
        .map(|p| (p[0] as f64 - p[1] as f64) / 255.0)
        .sum::<f64>();
    let blue = image
        .pixels
        .iter()
        .map(|p| (p[2] as f64 - p[1] as f64) / 255.0)
        .sum::<f64>();
    assert!(
        (red - std::f64::consts::PI * 22.0 * 22.0).abs() < 2.0,
        "{red}"
    );
    assert!(
        (blue - std::f64::consts::PI * (28.0 * 28.0 - 22.0 * 22.0)).abs() < 2.0,
        "{blue}"
    );
    let mut triangle = circle;
    let node = &mut triangle["children"][0]["children"][0];
    node["type"] = json!("polygon");
    node["polygonCount"] = json!(3);
    node["strokeWidth"] = json!(0);
    node["cornerRadius"] = json!(5);
    let image = shapes::raster(&triangle, 1);
    let red = image
        .pixels
        .iter()
        .map(|p| (p[0] as f64 - p[1] as f64) / 255.0)
        .sum::<f64>();
    // Each corner uses radius 5; the original triangle has circumradius 25.
    let base = 3.0 * 3.0f64.sqrt() / 4.0 * 25.0 * 25.0;
    let cut = 3.0 * 5.0 * 5.0 * (3.0f64.sqrt() - std::f64::consts::PI / 3.0);
    assert!((red - (base - cut)).abs() < 2.0, "{red} vs {}", base - cut);
    triangle["children"][0]["children"][0]["cornerRadius"] = json!(50);
    let capped = shapes::raster(&triangle, 1);
    triangle["children"][0]["children"][0]["cornerRadius"] = json!(100);
    assert_eq!(capped.pixels, shapes::raster(&triangle, 1).pixels);
}
use lazypencil_xtask::fixtures::oracles::shaders::{self, Scene};

#[test]
fn encoded_script_oracles_keep_reset_rows_gaps_and_raw_null_cases() {
    let words = scripts::random_words();
    assert_eq!(words.len(), 37);
    let image = scripts::random_raster(2);
    for row in 1..4 {
        for y in 0..32 {
            for x in 0..1184 {
                assert_eq!(image.pixel(x, y), image.pixel(x, row * 48 + y));
            }
        }
    }
    assert_eq!(image.pixel(24, 0), [255; 4]);
    assert_eq!(image.pixel(0, 32), [255; 4]);
    assert_eq!(image.pixel(1327, 191), [255; 4]);
    let cases = scripts::input_cases();
    assert_eq!(cases.len(), 22);
    assert!(
        cases
            .iter()
            .find(|c| c.name == "number-null")
            .unwrap()
            .stored["value"]
            .is_null()
    );
    let image = scripts::input_raster(2);
    assert_eq!((image.width, image.height), (512, 528));
    for row in 0..22 {
        assert_eq!(image.pixel(0, row * 24 + 8), [255; 4]);
    }
}

#[test]
fn authored_shader_equations_preserve_pixels_at_both_scales_and_panel_gaps() {
    for scale in [1, 2] {
        let defaults = shaders::raster(Scene::Defaults, scale);
        assert_eq!((defaults.width, defaults.height), (280 * scale, 20 * scale));
        assert_eq!(
            defaults.pixel(110 * scale, 10 * scale),
            [0x20, 0x60, 0xa0, 255]
        );
        assert_eq!(defaults.pixel(230 * scale, 10 * scale), [128, 128, 0, 255]);
        assert_eq!(defaults.pixel(250 * scale, 10 * scale), [64, 0, 0, 255]);
        let annotations = shaders::raster(Scene::AnnotationBinding, scale);
        assert_eq!(annotations.pixel(35 * scale, 10 * scale), [255; 4]);
        assert_eq!(annotations.pixel(10 * scale, 10 * scale), [64, 64, 0, 255]);
        assert_eq!(
            annotations.pixel(50 * scale, 80 * scale),
            [128, 128, 0, 255]
        );
        assert_eq!(annotations.pixel(90 * scale, 80 * scale), [191, 0, 0, 255]);
        let logical = shaders::raster(Scene::LogicalCoordinates, scale);
        assert_eq!(logical.pixel(201 * scale, scale), [0, 255, 0, 255]);
        assert_eq!(logical.pixel(241 * scale, scale), [128, 128, 0, 255]);
        assert_eq!(logical.pixel(35 * scale, scale), [255; 4]);
        assert_eq!(logical.pixel(5 * scale, 18 * scale), [255; 4]);
        let top = logical.pixel(0, 0);
        let bottom = logical.pixel(0, 16 * scale - 1);
        assert!(top[1] > bottom[1]);
        assert_eq!(logical.pixel(360 * scale, 0)[2], 64);
    }
}
