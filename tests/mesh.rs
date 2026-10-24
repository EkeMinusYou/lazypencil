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

fn fixture(children: Value) -> (TempDir, Store) {
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("mesh.pen");
    fs::write(
        &file,
        json!({"version":"2.21","children":children}).to_string(),
    )
    .unwrap();
    let store = Store::open(&file, Some(directory.path().join("data"))).unwrap();
    (directory, store)
}
fn output(children: Value, options: &RenderOptions) -> lazypencil::Result<render::RenderedImage> {
    let (_directory, store) = fixture(children);
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None)?;
    render::png(&project, &snapshot, options)
}
fn raster(output: &render::RenderedImage) -> Image {
    Image::from_encoded(Data::new_copy(&output.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
}
fn mesh(colors: Value) -> Value {
    json!({"type":"mesh_gradient","columns":2,"rows":2,"colors":colors})
}
fn scene(fill: Value, width: u32, height: u32) -> Value {
    json!([{"id":"tile","type":"rectangle","width":width,"height":height,"fill":fill}])
}
fn premul(color: Color) -> [f64; 4] {
    let alpha = color.a() as f64 / 255.0;
    [
        color.r() as f64 * alpha,
        color.g() as f64 * alpha,
        color.b() as f64 * alpha,
        color.a() as f64,
    ]
}
fn expected(colors: [Color; 4], u: f64, v: f64) -> [f64; 4] {
    let weights = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
    std::array::from_fn(|channel| {
        colors
            .iter()
            .zip(weights)
            .map(|(color, weight)| premul(*color)[channel] * weight)
            .sum()
    })
}
fn close(actual: Color, wanted: [f64; 4], tolerance: f64, context: &str) {
    let actual = premul(actual);
    for channel in 0..4 {
        assert!(
            (actual[channel] - wanted[channel]).abs() <= tolerance,
            "{context}: {actual:?} != {wanted:?}"
        );
    }
}

#[test]
fn regular_mesh_matches_an_independent_bilinear_color_and_alpha_field() {
    for colors in [
        [Color::RED, Color::GREEN, Color::BLUE, Color::WHITE],
        [
            Color::from_argb(0, 255, 0, 0),
            Color::from_argb(128, 0, 255, 0),
            Color::from_argb(192, 0, 0, 255),
            Color::WHITE,
        ],
    ] {
        let serialized: Vec<_> = colors
            .iter()
            .map(|c| format!("#{:02X}{:02X}{:02X}{:02X}", c.r(), c.g(), c.b(), c.a()))
            .collect();
        let mut fill = mesh(json!(serialized));
        // Omitted dimensions use the documented 2x2 default; omitted points use a regular grid.
        fill.as_object_mut().unwrap().remove("columns");
        fill.as_object_mut().unwrap().remove("rows");
        let rendered = output(scene(fill, 96, 64), &Default::default()).unwrap();
        assert!(rendered.dynamic_artifacts.is_empty());
        let image = raster(&rendered);
        let pixels = image.peek_pixels().unwrap();
        for y in 0..64 {
            for x in 0..96 {
                close(
                    pixels.get_color((x, y)),
                    expected(colors, (x as f64 + 0.5) / 96.0, (y as f64 + 0.5) / 64.0),
                    2.0,
                    &format!("{x},{y}"),
                );
            }
        }
    }
}

#[test]
fn shared_patch_boundaries_do_not_make_translucent_meshes_seamed_or_more_opaque() {
    let colors = [
        "#FF000080",
        "#00FF0080",
        "#0000FF80",
        "#FFFF0080",
        "#80808080",
        "#00FFFF80",
        "#FF00FF80",
        "#FFFFFF80",
        "#00000080",
    ];
    let fill = json!({"type":"mesh_gradient","columns":3,"rows":3,"colors":colors});
    let rendered = output(scene(fill, 120, 80), &Default::default()).unwrap();
    let image = raster(&rendered);
    let pixels = image.peek_pixels().unwrap();
    let parsed: Vec<_> = colors
        .iter()
        .map(|c| lazypencil::color::parse(c).unwrap())
        .collect();
    for y in 0..80 {
        for x in 0..120 {
            let column = (x / 60) as usize;
            let row = (y / 40) as usize;
            let corners = [
                parsed[row * 3 + column],
                parsed[row * 3 + column + 1],
                parsed[(row + 1) * 3 + column],
                parsed[(row + 1) * 3 + column + 1],
            ];
            close(
                pixels.get_color((x, y)),
                expected(
                    corners,
                    (x % 60) as f64 / 60.0 + 0.5 / 60.0,
                    (y % 40) as f64 / 40.0 + 0.5 / 40.0,
                ),
                2.0,
                &format!("{x},{y}"),
            );
        }
    }
}

#[test]
fn explicit_relative_handles_match_an_analytically_invertible_curved_surface() {
    let colors = [Color::RED, Color::GREEN, Color::BLUE, Color::WHITE];
    let fill = json!({"type":"mesh_gradient","columns":2,"rows":2,"colors":["#FF0000","#00FF00","#0000FF","#FFFFFF"],"points":[
        {"position":[0,0],"rightHandle":[0.3333333333333333,0.15],"bottomHandle":[0.12,0.3333333333333333]},
        {"position":[1,0],"leftHandle":[-0.3333333333333333,0.15],"bottomHandle":[0.12,0.3333333333333333]},
        {"position":[0,1],"rightHandle":[0.3333333333333333,0.15],"topHandle":[0.12,-0.3333333333333333]},
        {"position":[1,1],"leftHandle":[-0.3333333333333333,0.15],"topHandle":[0.12,-0.3333333333333333]}
    ]});
    let rendered = output(scene(fill, 160, 120), &Default::default()).unwrap();
    let image = raster(&rendered);
    let pixels = image.peek_pixels().unwrap();
    // This particular surface has x=u+0.36*v*(1-v), y=v+0.45*u*(1-u).
    // Invert that expression directly, independently of the renderer's Coons evaluation.
    for y in (20..110).step_by(11) {
        for x in (25..150).step_by(13) {
            let (px, py) = ((x as f64 + 0.5) / 160.0, (y as f64 + 0.5) / 120.0);
            let (mut u, mut v) = (px, py);
            for _ in 0..12 {
                let fx = u + 0.36 * v * (1.0 - v) - px;
                let fy = v + 0.45 * u * (1.0 - u) - py;
                let a = 0.36 * (1.0 - 2.0 * v);
                let b = 0.45 * (1.0 - 2.0 * u);
                let det = 1.0 - a * b;
                u -= (fx - a * fy) / det;
                v -= (fy - b * fx) / det;
            }
            assert!((0.0..1.0).contains(&u) && (0.0..1.0).contains(&v));
            close(
                pixels.get_color((x, y)),
                expected(colors, u, v),
                3.0,
                &format!("{x},{y}: {u},{v}"),
            );
        }
    }
    assert_eq!(
        pixels.get_color((80, 1)).a(),
        0,
        "The curved top edge must not be flattened"
    );
}

#[test]
fn moving_an_interior_vertex_changes_the_field_and_auto_handles_match_explicit_tangents() {
    let points: Vec<[f64; 2]> = (0..9)
        .map(|i| {
            if i == 4 {
                [0.7025, 0.3025]
            } else {
                [(i % 3) as f64 / 2.0, (i / 3) as f64 / 2.0]
            }
        })
        .collect();
    let mut fill = json!({"type":"mesh_gradient","columns":3,"rows":3,"colors":["#FF0000","#FF0000","#FF0000","#FF0000","#00FF00","#FF0000","#FF0000","#FF0000","#FF0000"],"points":points});
    let automatic = output(scene(fill.clone(), 200, 200), &Default::default()).unwrap();
    let image = raster(&automatic);
    assert!(image.peek_pixels().unwrap().get_color((140, 60)).g() >= 250);
    let mut explicit = Vec::new();
    for y in 0_usize..3 {
        for x in 0_usize..3 {
            let horizontal: [f64; 2] = std::array::from_fn(|axis| {
                (points[y * 3 + (x + 1).min(2)][axis] - points[y * 3 + x.saturating_sub(1)][axis])
                    / if x == 1 { 6.0 } else { 3.0 }
            });
            let vertical: [f64; 2] = std::array::from_fn(|axis| {
                (points[(y + 1).min(2) * 3 + x][axis] - points[y.saturating_sub(1) * 3 + x][axis])
                    / if y == 1 { 6.0 } else { 3.0 }
            });
            explicit.push(json!({"position":points[y*3+x],"leftHandle":horizontal.map(|v|-v),"rightHandle":horizontal,"topHandle":vertical.map(|v|-v),"bottomHandle":vertical}));
        }
    }
    fill["points"] = json!(explicit);
    let controlled = output(scene(fill.clone(), 200, 200), &Default::default()).unwrap();
    assert_eq!(automatic.png, controlled.png);
    fill["points"][4]["position"] = json!([0.5, 0.5]);
    let moved = output(scene(fill, 200, 200), &Default::default()).unwrap();
    assert_ne!(automatic.png, moved.png);
}

fn compare_images(a: &render::RenderedImage, b: &render::RenderedImage, tolerance: f64) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let a = raster(a);
    let b = raster(b);
    let a = a.peek_pixels().unwrap();
    let b = b.peek_pixels().unwrap();
    let mut maximum = (0.0, String::new());
    for y in 0..a.height() {
        for x in 0..a.width() {
            let actual = premul(a.get_color((x, y)));
            let wanted = premul(b.get_color((x, y)));
            let difference = actual
                .into_iter()
                .zip(wanted)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            if difference > maximum.0 {
                maximum = (difference, format!("{x},{y}: {actual:?} != {wanted:?}"));
            }
        }
    }
    assert!(
        maximum.0 <= tolerance,
        "maximum difference {}: {}",
        maximum.0,
        maximum.1
    );
}

#[test]
fn every_blend_mode_and_nested_opacity_match_the_existing_paint_path() {
    for mode in render::BLEND_MODES {
        let fill = json!({"type":"mesh_gradient","columns":2,"rows":2,"colors":["#C0406080","#C0406080","#C0406080","#C0406080"],"blendMode":mode,"opacity":0.5});
        let reference = json!({"type":"gradient","colors":[{"position":0,"color":"#C0406080"},{"position":1,"color":"#C0406080"}],"blendMode":mode,"opacity":0.5});
        let children = |paint| json!([{"id":"frame","type":"frame","layout":"none","width":48,"height":32,"fill":"#3070C080","opacity":0.6,"children":[{"id":"tile","type":"rectangle","width":48,"height":32,"fill":paint,"opacity":0.75}]}]);
        let actual = output(children(fill), &Default::default()).unwrap();
        let wanted = output(children(reference), &Default::default()).unwrap();
        compare_images(&actual, &wanted, 2.0);
    }
}

#[test]
fn mesh_fills_share_shape_clipping_text_masks_and_all_stroke_alignments() {
    for kind in ["ellipse", "path", "text", "rectangle"] {
        for alignment in ["inner", "center", "outer"] {
            let children = |fill| json!([{"id":"page","type":"frame","layout":"none","width":180,"height":100,"children":[{"id":"tile","type":kind,"x":20,"y":20,"width":130,"height":60,"cornerRadius":12,"geometry":"M0 0 L100 0 L50 60 Z","viewBox":[0,0,100,60],"textGrowth":"fixed-width-height","content":"A 日本語","fontSize":24,"underline":true,"fill":["#FF0000",fill],"stroke":fill,"strokeWidth":4,"strokeAlignment":alignment}]}]);
            let actual = output(
                children(mesh(json!(vec!["#0000FF80"; 4]))),
                &Default::default(),
            )
            .unwrap();
            let wanted = output(children(json!("#0000FF80")), &Default::default()).unwrap();
            // Image-shader AA and consecutive fill/stroke composites can round differently
            // from a solid paint; compare premultiplied channels within three 8-bit steps.
            compare_images(&actual, &wanted, 3.0);
        }
    }
    let mut fill = mesh(json!(["#FF0000", "#0000FF", "#FF0000", "#0000FF"]));
    fill["points"] = json!([[0, 0], [1, 0], [0, 1], [1, 1]]);
    let rendered = output(json!([{"id":"tile","type":"rectangle","x":8,"y":8,"width":40,"height":20,"stroke":fill,"strokeWidth":8,"strokeAlignment":"outer"}]), &RenderOptions{region:Some(Rect::new(0.0,0.0,56.0,36.0)),..Default::default()}).unwrap();
    let image = raster(&rendered);
    let pixels = image.peek_pixels().unwrap();
    assert!(
        pixels.get_color((2, 18)).r() > 245,
        "Outer stroke clamps the field at the left edge"
    );
    assert!(
        pixels.get_color((53, 18)).b() > 245,
        "Outer stroke clamps the field at the right edge"
    );
}

#[test]
fn cropped_rotated_scaled_meshes_keep_the_same_scene_and_effect_context() {
    let fill = json!({"type":"mesh_gradient","columns":3,"rows":3,"colors":["#FF004080","#0020FF80","#80FF0080","#4080FF80","#FFFFFF80","#FF204080","#FF80FF80","#10FF8080","#FF800080"],"points":[[0,0],[0.5,0],[1,0],[0,0.5],[0.68,0.38],[1,0.5],[0,1],[0.5,1],[1,1]]});
    for rotation in [0, 90, 37] {
        for scale in [1.0, 2.0] {
            let (_temp, store) = fixture(
                json!([{"id":"back","type":"rectangle","x":-80,"y":-80,"width":240,"height":200,"fill":"#10206080"},{"id":"tile","type":"rectangle","x":20,"y":20,"rotation":rotation,"width":90,"height":60,"cornerRadius":8,"fill":fill,"effect":[{"type":"background_blur","radius":2},{"type":"shadow","offset":{"x":3,"y":4},"blur":3,"color":"#C0409080","blendMode":"multiply"}]}]),
            );
            let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
            let full = render::png(
                &project,
                &snapshot,
                &RenderOptions {
                    scale,
                    ..Default::default()
                },
            )
            .unwrap();
            let crop = render::png(
                &project,
                &snapshot,
                &RenderOptions {
                    scale,
                    region: Some(Rect::new(0.0, 0.0, 96.0, 80.0)),
                    ..Default::default()
                },
            )
            .unwrap();
            let full_image = raster(&full);
            let crop_image = raster(&crop);
            let full_pixels = full_image.peek_pixels().unwrap();
            let crop_pixels = crop_image.peek_pixels().unwrap();
            for y in 0..crop.height as i32 {
                for x in 0..crop.width as i32 {
                    close(
                        crop_pixels.get_color((x, y)),
                        premul(
                            full_pixels
                                .get_color((x + (80.0 * scale) as i32, y + (80.0 * scale) as i32)),
                        ),
                        2.0,
                        &format!("{rotation}/{scale}: {x},{y}"),
                    );
                }
            }
        }
    }
}

#[test]
fn malformed_meshes_fail_with_source_context_and_disabled_paints_are_not_evaluated() {
    let valid = mesh(json!(vec!["#FF0000"; 4]));
    let mut cases = vec![json!({"type":"mesh_gradient"})];
    for (name, value) in [
        ("columns", json!(1)),
        ("rows", json!(65)),
        ("rows", json!(2.5)),
        ("columns", json!("2")),
        ("rows", Value::Null),
        ("colors", json!(["#F00"])),
        ("colors", json!(["#F00", false, "#F00", "#F00"])),
        ("points", json!([])),
        ("points", Value::Null),
        ("points", json!([[0, 0, 0], [1, 0], [0, 1], [1, 1]])),
        ("points", json!([[0, 0], [1, 0], [0, 1], [1.01, 1]])),
        (
            "points",
            json!([{"position":[0,0],"rightHandle":[0.2]},[1,0],[0,1],[1,1]]),
        ),
        ("points", json!([{}, [1, 0], [0, 1], [1, 1]])),
        ("opacity", json!(-0.1)),
    ] {
        let mut bad = valid.clone();
        bad[name] = value;
        cases.push(bad);
    }
    for mut fill in cases {
        for property in ["fill", "stroke"] {
            let mut children = scene(json!("#000"), 32, 24);
            children[0][property] = fill.clone();
            children[0]["strokeWidth"] = json!(2);
            let error = output(children, &Default::default())
                .err()
                .expect("A malformed mesh must fail");
            assert_eq!(error.code, "INVALID_DOCUMENT", "{error:?}");
            let details = error.details.unwrap();
            assert_eq!(details["node_id"], "tile");
            assert_eq!(details["property"], property);
            assert!(
                details["source_path"]
                    .as_str()
                    .unwrap()
                    .ends_with("mesh.pen")
            );
        }
        fill["enabled"] = json!(false);
        output(scene(json!(["#123456", fill]), 32, 24), &Default::default()).unwrap();
    }
}

#[test]
fn extreme_handles_exceed_the_budget_before_allocating_a_large_mesh() {
    for offset in [1.0e308, 1.0e100, 0.5] {
        let count = if offset > 1.0 { 2 } else { 64 };
        let points:Vec<_>=(0..count*count).map(|i| json!({"position":[(i%count) as f64/(count-1) as f64,(i/count) as f64/(count-1) as f64],"rightHandle":[offset,offset],"bottomHandle":[offset,offset]})).collect();
        let fill = json!({"type":"mesh_gradient","columns":count,"rows":count,"colors":vec!["#F00";count*count],"points":points});
        let error = output(scene(fill, 100, 100), &Default::default())
            .err()
            .expect("Excessive subdivision must fail");
        assert_eq!(error.code, "RESOURCE_LIMIT", "{error:?}");
        assert_eq!(error.details.unwrap()["property"], "fill");
    }
    // A failed reservation/mesh must not contaminate a later independent request.
    output(
        scene(mesh(json!(vec!["#F00"; 4])), 32, 24),
        &Default::default(),
    )
    .unwrap();
}

#[test]
fn cli_mesh_exports_and_node_previews_match_the_common_renderer() {
    use std::process::Command;
    let children = json!([{"id":"page","type":"frame","layout":"none","width":80,"height":60,"fill":"#102040","children":[{"id":"tile","type":"rectangle","x":16,"y":12,"width":40,"height":24,"fill":mesh(json!(["#FF0000","#0000FF","#FFFF00","#00FF00"]))}]}]);
    let (temp, store) = fixture(children);
    let exported = temp.path().join("mesh.png");
    let result = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(temp.path().join("cli-data"))
        .args(["--json", "render"])
        .arg(store.path())
        .args(["--scale", "2", "--node", "tile", "--output"])
        .arg(&exported)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["width"], 80);
    assert_eq!(report["height"], 48);
    assert!(report["dynamic_artifacts"].as_array().unwrap().is_empty());
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let common = render::png(
        &project,
        &snapshot,
        &RenderOptions {
            scale: 2.0,
            node: Some("tile".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(fs::read(&exported).unwrap(), common.png);
    for (fill, code) in [
        (
            json!({"type":"mesh_gradient","columns":1}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"type":"mesh_gradient","columns":2,"rows":2,"colors":vec!["#F00";4],"points":[{"position":[0,0],"rightHandle":[1.0e308,1.0e308]},[1,0],[0,1],[1,1]]}),
            "RESOURCE_LIMIT",
        ),
    ] {
        fs::write(
            store.path(),
            json!({"version":"2.21","children":scene(fill,32,24)}).to_string(),
        )
        .unwrap();
        let bad = temp.path().join("invalid.png");
        let result = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "render"])
            .arg(store.path())
            .arg("--output")
            .arg(&bad)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(4));
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["error"]["code"], code);
        assert!(!bad.exists());
    }
}

#[test]
fn bundled_mesh_study_renders_native_text_shapes_and_borders_without_a_runtime() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/mesh-study.pen");
    let temp = TempDir::new().unwrap();
    let store = Store::open(&file, Some(temp.path().join("data"))).unwrap();
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    assert!(
        snapshot
            .items
            .iter()
            .all(|item| item.record.diagnostics.is_empty())
    );
    let rendered = render::png(&project, &snapshot, &Default::default()).unwrap();
    assert_eq!((rendered.width, rendered.height), (1040, 900));
    assert!(rendered.dynamic_artifacts.is_empty());
    assert!(rendered.assets.is_empty());
}

#[test]
fn a_curved_mesh_spanning_multiple_index_strips_keeps_alpha_continuous() {
    let fill = json!({"type":"mesh_gradient","columns":2,"rows":2,"colors":vec!["#4080C080";4],"points":[
        {"position":[0,0],"rightHandle":[0.3333333333333333,1]},
        {"position":[1,0],"leftHandle":[-0.3333333333333333,1]},
        {"position":[0,1],"rightHandle":[0.3333333333333333,1]},
        {"position":[1,1],"leftHandle":[-0.3333333333333333,1]}
    ]});
    let rendered = output(scene(fill, 32, 8192), &Default::default()).unwrap();
    let image = raster(&rendered);
    let pixels = image.peek_pixels().unwrap();
    // x=u; y=v+3*u*(1-u). Probe inside the field, including index-strip joins.
    // Edge scan conversion on this extreme aspect ratio is not a point-sample oracle.
    for x in 0..32 {
        let u = (x as f64 + 0.5) / 32.0;
        let boundary = 3.0 * u * (1.0 - u) * 8192.0;
        for y in (0..8192).step_by(31) {
            let v = (y as f64 + 0.5 - boundary) / 8192.0;
            if !(0.15..=0.95).contains(&v) {
                continue;
            }
            assert!(
                (pixels.get_color((x, y)).a() as i32 - 128).abs() <= 1,
                "{x},{y}: {boundary}"
            );
        }
    }
}

#[test]
fn a_real_script_can_generate_the_same_mesh_as_an_editable_node() {
    let fill = json!({"type":"mesh_gradient","columns":3,"rows":3,"colors":["#FF000080","#00F","#0F0","#FF0","#0FF","#F0F","#F00","#00F","#0F0"],"points":[[0,0],[0.5,0],[1,0],[0,0.5],[0.65,0.35],[1,0.5],[0,1],[0.5,1],[1,1]]});
    let rectangle = json!({"type":"rectangle","width":96,"height":64,"cornerRadius":8,"fill":fill});
    let mut stored = rectangle.clone();
    stored["id"] = json!("tile");
    let baseline = output(json!([stored]), &Default::default()).unwrap();
    let (temp, store) = fixture(
        json!([{"id":"generated","type":"script","scriptUri":"mesh.js","width":96,"height":64}]),
    );
    fs::write(
        temp.path().join("mesh.js"),
        format!("/**\n * @schema 2.21\n */\nreturn [{}];", rectangle),
    )
    .unwrap();
    let original = fs::read(store.path()).unwrap();
    let mut runtime = lazypencil::script::ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil"));
    let (project, snapshot) =
        service::layout_snapshot_with_runtime(&store, &[], None, None, &mut runtime).unwrap();
    assert_eq!(snapshot.script_executions.len(), 1);
    assert!(
        snapshot
            .items
            .iter()
            .any(|item| item.record.generated_by.is_some())
    );
    let generated = render::png(&project, &snapshot, &Default::default()).unwrap();
    assert_eq!(generated.png, baseline.png);
    assert_eq!(fs::read(store.path()).unwrap(), original);
}

#[test]
#[ignore = "Requires verified headless Chromium, software WebGL, and its native sandbox"]
fn real_webgl_effects_preserve_native_mesh_pixels_and_alpha() {
    let executable = std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
        .expect("Set LAZYPENCIL_TEST_CHROMIUM to a verified Chromium executable");
    let fill = json!({"type":"mesh_gradient","columns":3,"rows":3,"colors":["#FF004080","#0020FF80","#80FF0080","#4080FF80","#FFFFFF80","#FF204080","#FF80FF80","#10FF8080","#FF800080"],"points":[[0,0],[0.5,0],[1,0],[0,0.5],[0.68,0.38],[1,0.5],[0,1],[0.5,1],[1,1]],"opacity":0.7});
    let (temp, store) = fixture(json!([]));
    fs::write(temp.path().join("copy.glsl"),"precision highp float;\n/** @resolution */ uniform vec2 size;\nuniform sampler2D object;\nvoid main(){gl_FragColor=texture2D(object,gl_FragCoord.xy/size);}").unwrap();
    let mut runtime = lazypencil::dynamic::DynamicRuntime::new(store.data_dir())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable);
    for shader_first in [false, true] {
        let effects = json!([{"type":"background_blur","radius":2},{"type":"shadow","offset":{"x":3,"y":4},"blur":3,"color":"#C0409080","blendMode":"multiply"}]);
        let mut children = json!([{"id":"back","type":"rectangle","width":80,"height":60,"fill":"#10206080"},{"id":"tile","type":"rectangle","x":12,"y":12,"width":48,"height":32,"cornerRadius":6,"fill":fill,"opacity":0.75,"effect":effects}]);
        fs::write(
            store.path(),
            json!({"version":"2.21","children":children}).to_string(),
        )
        .unwrap();
        let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let baseline = render::png(&project, &snapshot, &Default::default()).unwrap();
        let mut effects = effects.as_array().unwrap().clone();
        let length = effects.len();
        effects.insert(
            if shader_first { 0 } else { length },
            json!({"type":"shader","url":"copy.glsl","uniforms":{"object":"pencil:object"}}),
        );
        children[1]["effect"] = json!(effects);
        fs::write(
            store.path(),
            json!({"version":"2.21","children":children}).to_string(),
        )
        .unwrap();
        let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let actual =
            render::png_with_runtime(&project, &snapshot, &Default::default(), &mut runtime, None)
                .unwrap();
        assert!(!actual.dynamic_artifacts.is_empty());
        compare_images(&actual, &baseline, 2.0);
    }
}
