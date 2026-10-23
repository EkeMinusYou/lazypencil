#![cfg(feature = "rendering")]

use lazypencil::{
    geometry::Rect,
    render::{self, RenderOptions},
    service,
    store::Store,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::fs;
use tempfile::TempDir;

const MODES: [&str; 18] = [
    "normal",
    "darken",
    "multiply",
    "linearBurn",
    "colorBurn",
    "light",
    "screen",
    "linearDodge",
    "colorDodge",
    "overlay",
    "softLight",
    "hardLight",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];
const BASE: [f64; 3] = [48.0 / 255.0, 144.0 / 255.0, 208.0 / 255.0];
const INK: [f64; 3] = [192.0 / 255.0, 80.0 / 255.0, 144.0 / 255.0];

fn fixture(children: Value) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let path = temp.path().join("design.pen");
    fs::write(
        &path,
        json!({"version":"2.21","children":children}).to_string(),
    )
    .unwrap();
    let store = Store::open(path, Some(temp.path().join("data"))).unwrap();
    (temp, store)
}
fn try_render(
    children: Value,
    options: RenderOptions,
) -> lazypencil::Result<render::RenderedImage> {
    let (_temp, store) = fixture(children);
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None)?;
    render::png(&project, &snapshot, &options)
}
fn raster(output: &render::RenderedImage) -> Image {
    Image::from_encoded(Data::new_copy(&output.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
}

// Independent unpremultiplied reference equations for standard color blending.
// They intentionally do not call Skia or the production blend implementation.
fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn sat(c: [f64; 3]) -> f64 {
    c.into_iter().fold(f64::NEG_INFINITY, f64::max) - c.into_iter().fold(f64::INFINITY, f64::min)
}
fn set_lum(c: [f64; 3], target: f64) -> [f64; 3] {
    let delta = target - lum(c);
    let mut out = c.map(|v| v + delta);
    let low = out.into_iter().fold(f64::INFINITY, f64::min);
    let high = out.into_iter().fold(f64::NEG_INFINITY, f64::max);
    if low < 0.0 {
        out = out.map(|v| target + (v - target) * target / (target - low));
    }
    if high > 1.0 {
        out = out.map(|v| target + (v - target) * (1.0 - target) / (high - target));
    }
    out
}
fn set_sat(c: [f64; 3], target: f64) -> [f64; 3] {
    let mut order = [0, 1, 2];
    order.sort_by(|a, b| c[*a].total_cmp(&c[*b]));
    let [lo, mid, hi] = order;
    let mut out = [0.0; 3];
    if c[hi] > c[lo] {
        out[mid] = (c[mid] - c[lo]) * target / (c[hi] - c[lo]);
        out[hi] = target;
    }
    out
}
fn color_blend(mode: &str, b: [f64; 3], s: [f64; 3]) -> [f64; 3] {
    match mode {
        "hue" => return set_lum(set_sat(s, sat(b)), lum(b)),
        "saturation" => return set_lum(set_sat(b, sat(s)), lum(b)),
        "color" => return set_lum(s, lum(b)),
        "luminosity" => return set_lum(b, lum(s)),
        _ => {}
    }
    std::array::from_fn(|i| {
        let (b, s) = (b[i], s[i]);
        match mode {
            "normal" => s,
            "darken" => b.min(s),
            "multiply" => b * s,
            "linearBurn" => (b + s - 1.0).max(0.0),
            "colorBurn" => {
                if b == 1.0 {
                    1.0
                } else if s == 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - b) / s).min(1.0)
                }
            }
            "light" => b.max(s),
            "screen" => b + s - b * s,
            "linearDodge" => (b + s).min(1.0),
            "colorDodge" => {
                if b == 0.0 {
                    0.0
                } else if s == 1.0 {
                    1.0
                } else {
                    (b / (1.0 - s)).min(1.0)
                }
            }
            "overlay" => {
                if b <= 0.5 {
                    2.0 * b * s
                } else {
                    1.0 - 2.0 * (1.0 - b) * (1.0 - s)
                }
            }
            "hardLight" => {
                if s <= 0.5 {
                    2.0 * b * s
                } else {
                    1.0 - 2.0 * (1.0 - b) * (1.0 - s)
                }
            }
            "softLight" => {
                if s <= 0.5 {
                    b - (1.0 - 2.0 * s) * b * (1.0 - b)
                } else {
                    let d = if b <= 0.25 {
                        ((16.0 * b - 12.0) * b + 4.0) * b
                    } else {
                        b.sqrt()
                    };
                    b + (2.0 * s - 1.0) * (d - b)
                }
            }
            "difference" => (b - s).abs(),
            "exclusion" => b + s - 2.0 * b * s,
            _ => panic!("Unknown reference mode {mode}"),
        }
    })
}
fn over(mode: &str, b: [f64; 3], ba: f64, s: [f64; 3], sa: f64) -> [f64; 4] {
    let color = color_blend(mode, b, s);
    let alpha = sa + ba * (1.0 - sa);
    let rgb = std::array::from_fn::<_, 3, _>(|i| {
        sa * (1.0 - ba) * s[i] + sa * ba * color[i] + (1.0 - sa) * ba * b[i]
    });
    [rgb[0], rgb[1], rgb[2], alpha]
}
fn assert_pixel(actual: Color, expected: [f64; 4], context: &str, tolerance: i32) {
    let alpha = expected[3];
    let expected = [
        alpha * 255.0,
        if alpha > 0.0 {
            expected[0] / alpha * 255.0
        } else {
            0.0
        },
        if alpha > 0.0 {
            expected[1] / alpha * 255.0
        } else {
            0.0
        },
        if alpha > 0.0 {
            expected[2] / alpha * 255.0
        } else {
            0.0
        },
    ];
    for (component, expected) in [actual.a(), actual.r(), actual.g(), actual.b()]
        .into_iter()
        .zip(expected)
    {
        assert!(
            (component as f64 - expected).abs() <= tolerance as f64,
            "{context}: {actual:?}, expected {expected}, delta {}",
            component as f64 - expected
        );
    }
}

#[test]
fn every_outer_shadow_mode_matches_alpha_compositing_against_the_actual_backdrop() {
    let cases = [
        (255_u8, 255_u8, 255_u8),
        (128, 128, 128),
        (255, 128, 0),
        (0, 255, 128),
    ];
    let mut children = Vec::new();
    for (row, (object, shadow, back)) in cases.iter().copied().enumerate() {
        for (column, mode) in MODES.iter().enumerate() {
            let (x, y) = (column * 28, row * 20);
            children.push(json!({"id":format!("back-{row}-{column}"),"type":"rectangle","x":x,"y":y,"width":24,"height":16,"fill":format!("#3090D0{back:02X}")}));
            children.push(json!({"id":format!("object-{row}-{column}"),"type":"rectangle","x":x,"y":y+4,"width":8,"height":8,"fill":format!("#203040{object:02X}"),
                "effect":{"type":"shadow","offset":{"x":8,"y":0},"blur":0,"color":format!("#C05090{shadow:02X}"),"blendMode":mode}}));
        }
    }
    let output = try_render(json!(children), Default::default()).unwrap();
    assert!(output.dynamic_artifacts.is_empty());
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    for (row, (object, shadow, back)) in cases.iter().copied().enumerate() {
        for (column, mode) in MODES.iter().enumerate() {
            let expected = over(
                mode,
                BASE,
                back as f64 / 255.0,
                INK,
                object as f64 / 255.0 * shadow as f64 / 255.0,
            );
            assert_pixel(
                pixels.get_color(((column * 28 + 10) as i32, (row * 20 + 8) as i32)),
                expected,
                &format!("outer {mode}, case {row}"),
                5,
            );
        }
    }
}

#[test]
fn every_inner_shadow_mode_preserves_object_alpha_and_blends_only_inside_it() {
    let cases = [(255_u8, 255_u8), (128, 128), (0, 255)];
    let mut children = Vec::new();
    for (row, (object, shadow)) in cases.iter().copied().enumerate() {
        for (column, mode) in MODES.iter().enumerate() {
            children.push(json!({"id":format!("inner-{row}-{column}"),"type":"rectangle","x":column*24,"y":row*20,"width":16,"height":12,"fill":format!("#3090D0{object:02X}"),
                "effect":{"type":"shadow","shadowType":"inner","offset":{"x":8,"y":0},"blur":0,"color":format!("#C05090{shadow:02X}"),"blendMode":mode}}));
        }
    }
    let output = try_render(
        json!(children),
        RenderOptions {
            region: Some(Rect::new(0.0, 0.0, 432.0, 60.0)),
            ..Default::default()
        },
    )
    .unwrap();
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    for (row, (object, shadow)) in cases.iter().copied().enumerate() {
        for (column, mode) in MODES.iter().enumerate() {
            let alpha = object as f64 / 255.0;
            for (dx, mask) in [(2, 1.0), (12, 1.0 - alpha)] {
                let strength = shadow as f64 / 255.0 * mask;
                let mixed = color_blend(mode, BASE, INK);
                let expected = std::array::from_fn::<_, 3, _>(|i| {
                    alpha * ((1.0 - strength) * BASE[i] + strength * mixed[i])
                });
                assert_pixel(
                    pixels.get_color(((column * 24 + dx) as i32, (row * 20 + 6) as i32)),
                    [expected[0], expected[1], expected[2], alpha],
                    &format!("inner {mode}, case {row}, x {dx}"),
                    5,
                );
            }
            assert_eq!(
                pixels
                    .get_color(((column * 24 + 18) as i32, (row * 20 + 6) as i32))
                    .a(),
                0
            );
        }
    }
}

#[test]
fn paint_color_blending_uses_source_over_alpha_including_linear_modes() {
    let mut children = Vec::new();
    for (column, mode) in MODES.iter().enumerate() {
        children.push(json!({"id":format!("back-{column}"),"type":"rectangle","x":column*20,"width":16,"height":16,"fill":"#3090D080"}));
        children.push(json!({"id":format!("paint-{column}"),"type":"rectangle","x":column*20,"width":16,"height":16,"fill":{"type":"color","color":"#C0509080","blendMode":mode}}));
    }
    let output = try_render(json!(children), Default::default()).unwrap();
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    for (column, mode) in MODES.iter().enumerate() {
        assert_pixel(
            pixels.get_color(((column * 20 + 8) as i32, 8)),
            over(mode, BASE, 128.0 / 255.0, INK, 128.0 / 255.0),
            &format!("paint {mode}"),
            5,
        );
    }
}

#[test]
fn blended_shadow_after_background_blur_does_not_reapply_transparent_background_alpha() {
    let output=try_render(json!([
        {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#0000FF80"},
        {"id":"glass","type":"rectangle","x":20,"y":10,"width":20,"height":20,"effect":[{"type":"background_blur","radius":0},
            {"type":"shadow","offset":{"x":20,"y":0},"blur":0,"color":"#FF000080","blendMode":"multiply"}]}
    ]),Default::default()).unwrap();
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    let alpha = 128.0 / 255.0;
    assert_pixel(
        pixels.get_color((45, 20)),
        over(
            "multiply",
            [0.0, 0.0, 1.0],
            alpha,
            [1.0, 0.0, 0.0],
            alpha * alpha,
        ),
        "shadow after backdrop blur",
        3,
    );
    assert_eq!(pixels.get_color((25, 20)).a(), 128);
    assert_eq!(pixels.get_color((70, 20)).a(), 128);
}

#[test]
fn invalid_active_blend_modes_fail_but_disabled_effects_do_not_evaluate_them() {
    for mode in [json!(17), Value::Null, json!("not-a-mode")] {
        for shadow_type in ["inner", "outer"] {
            let effect = json!({"type":"shadow","shadowType":shadow_type,"blendMode":mode});
            let error=try_render(json!([{"id":"box","type":"rectangle","width":20,"height":20,"fill":"#FFFFFF","effect":effect}]),Default::default()).err().unwrap();
            assert_eq!(
                error.code,
                if mode.is_string() {
                    "UNSUPPORTED_FEATURE"
                } else {
                    "INVALID_DOCUMENT"
                }
            );
            assert_eq!(error.details.as_ref().unwrap()["node_id"], "box");
            assert_eq!(error.details.as_ref().unwrap()["property"], "effect");
            let mut disabled = effect;
            disabled["enabled"] = json!(false);
            try_render(json!([{"id":"box","type":"rectangle","width":20,"height":20,"fill":"#FFFFFF","effect":disabled}]),Default::default()).unwrap();
        }
    }
    for kind in [json!("unknown"), json!(17), Value::Null] {
        let error=try_render(json!([{"id":"box","type":"rectangle","width":20,"height":20,"fill":"#FFFFFF","effect":{"type":"shadow","shadowType":kind,"blendMode":"multiply"}}]),Default::default()).err().unwrap();
        assert_eq!(error.code, "INVALID_DOCUMENT");
    }
}

#[test]
fn oversized_unknown_modes_return_bounded_diagnostics_instead_of_echoing_document_values() {
    let error = render::blend(&"invalid".repeat(1024 * 1024)).unwrap_err();
    let diagnostic = serde_json::to_string(&error).unwrap();
    assert_eq!(error.code, "UNSUPPORTED_FEATURE");
    assert!(diagnostic.len() < 2048);
    assert_eq!(error.details.as_ref().unwrap()["mode_truncated"], true);
    assert_eq!(
        error.details.as_ref().unwrap()["supported"]
            .as_array()
            .unwrap()
            .len(),
        18
    );
}

fn premul(rgb: [f64; 3], alpha: f64) -> [f64; 4] {
    [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]
}
fn source_over(source: [f64; 4], destination: [f64; 4]) -> [f64; 4] {
    std::array::from_fn(|i| source[i] + destination[i] * (1.0 - source[3]))
}

#[test]
fn nested_opacity_is_applied_after_the_shadow_and_parent_clips_both_color_and_coverage() {
    let mut children = Vec::new();
    for (column, mode) in MODES.iter().enumerate() {
        for (row, clip) in [false, true].into_iter().enumerate() {
            let (x, y) = (column * 48, row * 24);
            children.push(json!({"id":format!("back-{row}-{column}"),"type":"rectangle","x":x,"y":y,"width":40,"height":20,"fill":"#3090D080"}));
            children.push(json!({"id":format!("parent-{row}-{column}"),"type":"frame","layout":"none","x":x,"y":y,"width":if clip {20}else{40},"height":20,"fill":"#B0402080","opacity":0.5,"clip":clip,
                "children":[{"id":format!("child-{row}-{column}"),"type":"rectangle","x":8,"y":4,"width":12,"height":12,"fill":"#20406080","opacity":0.5,
                    "effect":{"type":"shadow","offset":{"x":4,"y":0},"blur":0,"color":"#C0509080","blendMode":mode}}]}));
        }
    }
    let output = try_render(json!(children), Default::default()).unwrap();
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    let alpha = 128.0 / 255.0;
    let base = premul(BASE, alpha);
    let parent = premul([176.0 / 255.0, 64.0 / 255.0, 32.0 / 255.0], alpha);
    let backdrop = source_over(parent, base);
    let backdrop_rgb = [
        backdrop[0] / backdrop[3],
        backdrop[1] / backdrop[3],
        backdrop[2] / backdrop[3],
    ];
    for (column, mode) in MODES.iter().enumerate() {
        let strength = alpha * alpha;
        let mixed = over(mode, backdrop_rgb, backdrop[3], INK, strength);
        let shadow = [
            mixed[0] - backdrop[0] * (1.0 - strength),
            mixed[1] - backdrop[1] * (1.0 - strength),
            mixed[2] - backdrop[2] * (1.0 - strength),
            strength,
        ];
        let object = premul([32.0 / 255.0, 64.0 / 255.0, 96.0 / 255.0], alpha);
        let child = source_over(object, shadow).map(|v| v * 0.5);
        let parent = source_over(child, parent).map(|v| v * 0.5);
        let expected = source_over(parent, base);
        for row in 0..2 {
            assert_pixel(
                pixels.get_color(((column * 48 + 17) as i32, row * 24 + 8)),
                expected,
                &format!("parent opacity {mode}, row {row}"),
                5,
            );
        }
        assert_pixel(
            pixels.get_color(((column * 48 + 22) as i32, 32)),
            base,
            &format!("parent clip {mode}"),
            2,
        );
    }
}

fn compare_crop(full: &render::RenderedImage, crop: &render::RenderedImage, scale: f32) {
    let a = raster(full);
    let a = a.peek_pixels().unwrap();
    let b = raster(crop);
    let b = b.peek_pixels().unwrap();
    let (ox, oy) = (
        ((crop.region.x - full.region.x) * scale).round() as i32,
        ((crop.region.y - full.region.y) * scale).round() as i32,
    );
    assert!(
        ox >= 0
            && oy >= 0
            && ox + crop.width as i32 <= full.width as i32
            && oy + crop.height as i32 <= full.height as i32,
        "Comparison viewport is outside the full raster: {:?} vs {:?}",
        crop.region,
        full.region
    );
    for y in 0..crop.height as i32 {
        for x in 0..crop.width as i32 {
            let source = a.get_color((ox + x, oy + y));
            let target = b.get_color((x, y));
            for (source, target) in [source.a(), source.r(), source.g(), source.b()]
                .into_iter()
                .zip([target.a(), target.r(), target.g(), target.b()])
            {
                assert!(
                    (source as i32 - target as i32).abs() <= 2,
                    "Crop mismatch at {x},{y}: {source} vs {target}"
                );
            }
        }
    }
}

#[test]
fn cropped_scaled_and_rotated_blended_effects_keep_scene_context_and_declared_order() {
    let mut variants = Vec::new();
    for reverse in [false, true] {
        let mut effects = vec![
            json!({"type":"shadow","offset":{"x":10,"y":0},"blur":6,"color":"#C0509080","blendMode":"multiply"}),
            json!({"type":"blur","radius":6}),
        ];
        if reverse {
            effects.reverse();
        }
        for rotation in [0, 90] {
            let mut children=(0..30).map(|index|json!({"id":format!("stripe-{index}"),"type":"rectangle","x":index*4,"width":4,"height":80,"fill":if index%2==0 {"#3090D080"}else{"#E0B02080"}})).collect::<Vec<_>>();
            children.push(json!({"id":"object","type":"rectangle","x":30,"y":20,"width":20,"height":20,"rotation":rotation,"fill":"#204060B0","effect":effects}));
            let (_temp, store) = fixture(json!(children));
            let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
            for scale in [1.0, 2.0] {
                let full = render::png(
                    &project,
                    &snapshot,
                    &RenderOptions {
                        region: Some(Rect::new(-40.0, -40.0, 200.0, 160.0)),
                        scale,
                        ..Default::default()
                    },
                )
                .unwrap();
                let crop = render::png(
                    &project,
                    &snapshot,
                    &RenderOptions {
                        region: Some(Rect::new(35.0, 15.0, 30.0, 35.0)),
                        scale,
                        ..Default::default()
                    },
                )
                .unwrap();
                compare_crop(&full, &crop, scale);
                let node = render::png(
                    &project,
                    &snapshot,
                    &RenderOptions {
                        node: Some("object".into()),
                        scale,
                        ..Default::default()
                    },
                )
                .unwrap();
                compare_crop(&full, &node, scale);
                if rotation == 0 && scale == 1.0 {
                    variants.push(full.png);
                }
            }
        }
    }
    assert_ne!(
        variants[0], variants[1],
        "Changing effect order must not be normalized away"
    );
}

#[test]
fn shadow_offset_rotation_matches_normal_shadow_geometry_on_transparent_backgrounds() {
    let mut normal = Vec::new();
    let mut blended = Vec::new();
    for (column, mode) in ["multiply", "linearBurn", "linearDodge", "hue"]
        .iter()
        .enumerate()
    {
        for (row, rotation) in [90, 180, 270].into_iter().enumerate() {
            let node = json!({"id":format!("node-{column}-{row}"),"type":"rectangle","x":20+column*50,"y":20+row*50,"width":16,"height":12,"rotation":rotation,"fill":"#3090D080",
                "effect":{"type":"shadow","offset":{"x":8,"y":0},"blur":0,"color":"#C0509080","blendMode":mode}});
            blended.push(node.clone());
            let mut plain = node;
            plain["effect"]["blendMode"] = json!("normal");
            normal.push(plain);
        }
    }
    for scale in [1.0, 2.0] {
        let options = RenderOptions {
            region: Some(Rect::new(0.0, 0.0, 220.0, 180.0)),
            scale,
            ..Default::default()
        };
        let a = try_render(json!(normal), options.clone()).unwrap();
        let b = try_render(json!(blended), options).unwrap();
        let first = raster(&a);
        let first = first.peek_pixels().unwrap();
        let second = raster(&b);
        let second = second.peek_pixels().unwrap();
        // The native filter and intermediate-layer paths can quantize low-alpha
        // colors differently. Compare alpha and physically meaningful premultiplied RGB.
        for y in 0..a.height as i32 {
            for x in 0..a.width as i32 {
                let first = first.get_color((x, y));
                let second = second.get_color((x, y));
                assert!(
                    (first.a() as i32 - second.a() as i32).abs() <= 1,
                    "Geometry mismatch at {x},{y}: {first:?} vs {second:?}"
                );
                for (a, b) in [first.r(), first.g(), first.b()].into_iter().zip([
                    second.r(),
                    second.g(),
                    second.b(),
                ]) {
                    let delta =
                        (a as f64 * first.a() as f64 - b as f64 * second.a() as f64).abs() / 255.0;
                    assert!(delta <= 2.0, "Premultiplied mismatch at {x},{y}: {delta}");
                }
            }
        }
    }
}

#[test]
fn linear_modes_apply_to_images_gradients_and_strokes_without_adding_alpha() {
    let (temp, store) = fixture(json!([]));
    let mut source = skia_safe::surfaces::raster_n32_premul((2, 2)).unwrap();
    source.canvas().clear(Color::from_argb(128, 192, 80, 144));
    fs::write(
        temp.path().join("ink.png"),
        skia_safe::png_encoder::encode_image(None, &source.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let mut children = Vec::new();
    for (column, mode) in ["linearBurn", "linearDodge"].iter().enumerate() {
        for (row, kind) in ["image", "gradient", "stroke"].iter().enumerate() {
            let (x, y) = (column * 28, row * 28);
            children.push(json!({"id":format!("back-{row}-{column}"),"type":"rectangle","x":x,"y":y,"width":24,"height":24,"fill":"#3090D080"}));
            let mut paint = json!({"id":format!("paint-{row}-{column}"),"type":"rectangle","x":x,"y":y,"width":16,"height":16});
            match *kind {
                "image" => paint["fill"] = json!({"type":"image","url":"ink.png","blendMode":mode}),
                "gradient" => {
                    paint["fill"] = json!({"type":"gradient","blendMode":mode,"gradientType":"linear","colors":[{"position":0,"color":"#C0509080"},{"position":1,"color":"#C0509080"}]})
                }
                "stroke" => {
                    paint["stroke"] = json!({"type":"color","color":"#C0509080","blendMode":mode});
                    paint["strokeWidth"] = json!(4);
                    paint["strokeAlignment"] = json!("inner");
                }
                _ => unreachable!(),
            }
            children.push(paint);
        }
    }
    fs::write(
        store.path(),
        json!({"version":"2.21","children":children}).to_string(),
    )
    .unwrap();
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let output = render::png(&project, &snapshot, &Default::default()).unwrap();
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    for (column, mode) in ["linearBurn", "linearDodge"].iter().enumerate() {
        for (row, kind) in ["image", "gradient", "stroke"].iter().enumerate() {
            let dx = if *kind == "stroke" { 1 } else { 8 };
            assert_pixel(
                pixels.get_color(((column * 28 + dx) as i32, (row * 28 + 8) as i32)),
                over(mode, BASE, 128.0 / 255.0, INK, 128.0 / 255.0),
                &format!("{kind} {mode}"),
                5,
            );
        }
    }
}

#[test]
fn zero_area_blended_effects_remain_empty_without_starting_a_shader_runtime() {
    let (temp, store) = fixture(
        json!([{"id":"empty","type":"rectangle","width":0,"height":20,"effect":[
        {"type":"shadow","blur":0,"blendMode":"multiply"},
        {"type":"shader","url":"unused.glsl"}]}]),
    );
    fs::write(
        temp.path().join("unused.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(1.0);}",
    )
    .unwrap();
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let output = render::png(&project, &snapshot, &Default::default()).unwrap();
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    assert!(output.dynamic_artifacts.is_empty());
    for y in 0..output.height as i32 {
        assert_eq!(pixels.get_color((0, y)).a(), 0);
    }
}

#[test]
fn bundled_shadow_study_renders_all_modes_with_native_text_and_no_runtime() {
    let temp = TempDir::new().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/shadow-blends.pen");
    let store = Store::open(path, Some(temp.path().join("data"))).unwrap();
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    assert!(
        snapshot
            .items
            .iter()
            .all(|item| item.record.diagnostics.is_empty())
    );
    let output = render::png(&project, &snapshot, &Default::default()).unwrap();
    assert_eq!((output.width, output.height), (1040, 1000));
    assert!(output.dynamic_artifacts.is_empty());
    assert!(output.assets.is_empty());
}

#[test]
#[ignore = "Requires verified headless Chromium, software WebGL, and its native sandbox"]
fn real_webgl_pass_through_preserves_blended_shadows_in_both_effect_orders() {
    let executable = std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
        .expect("Set LAZYPENCIL_TEST_CHROMIUM to a verified Chromium executable");
    let (temp, store) = fixture(json!([]));
    fs::write(temp.path().join("copy.glsl"),"precision highp float;\n/* @resolution */ uniform vec2 size;\nuniform sampler2D object;\nvoid main(){gl_FragColor=texture2D(object,gl_FragCoord.xy/size);}").unwrap();
    let mut runtime =
        lazypencil::dynamic::DynamicRuntime::new(store.data_dir()).with_executable(executable);
    for mode in ["multiply", "linearBurn", "linearDodge", "hue"] {
        for shader_first in [false, true] {
            let native_effects = vec![
                json!({"type":"background_blur","radius":0}),
                json!({"type":"shadow","offset":{"x":20,"y":0},"blur":4,"color":"#C0509080","blendMode":mode}),
            ];
            let children = json!([
                {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#3090D080"},
                {"id":"glass","type":"rectangle","x":20,"y":10,"width":20,"height":20,"cornerRadius":4,"fill":"#20406040","opacity":0.75,"effect":native_effects}
            ]);
            fs::write(
                store.path(),
                json!({"version":"2.21","children":children}).to_string(),
            )
            .unwrap();
            let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
            let baseline = render::png(&project, &snapshot, &Default::default()).unwrap();
            let mut effects = native_effects;
            let shader =
                json!({"type":"shader","url":"copy.glsl","uniforms":{"object":"pencil:object"}});
            effects.insert(if shader_first { 0 } else { effects.len() }, shader);
            let mut children = children;
            children[1]["effect"] = json!(effects);
            fs::write(
                store.path(),
                json!({"version":"2.21","children":children}).to_string(),
            )
            .unwrap();
            let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
            let rendered = render::png_with_runtime(
                &project,
                &snapshot,
                &Default::default(),
                &mut runtime,
                None,
            )
            .unwrap();
            assert!(!rendered.dynamic_artifacts.is_empty());
            compare_crop(&baseline, &rendered, 1.0);
        }
    }
}
