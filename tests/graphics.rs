#![cfg(feature = "rendering")]

#[allow(dead_code)]
#[path = "../xtask/src/fixtures/oracles/mod.rs"]
mod render_oracles;

use lazypencil::{
    layout,
    project::Project,
    render::{self, RenderOptions},
    store::Store,
    text::Fonts,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::fs;
use tempfile::TempDir;

struct Scene {
    directory: TempDir,
    children: Value,
}
impl Scene {
    fn new(children: Value) -> Self {
        Self {
            directory: TempDir::new().unwrap(),
            children,
        }
    }
    fn render(&self) -> lazypencil::Result<render::RenderedImage> {
        let file = self.directory.path().join("scene.pen");
        fs::write(
            &file,
            json!({"version":"2.21","children":self.children}).to_string(),
        )
        .unwrap();
        render_file(&file)
    }
    fn image(&self) {
        let mut surface = skia_safe::surfaces::raster_n32_premul((8, 4)).unwrap();
        surface.canvas().clear(Color::RED);
        let mut paint = skia_safe::Paint::default();
        paint.set_color(Color::BLUE);
        surface
            .canvas()
            .draw_rect(skia_safe::Rect::from_xywh(4.0, 0.0, 4.0, 4.0), &paint);
        fs::write(
            self.directory.path().join("image.png"),
            skia_safe::png_encoder::encode_image(
                None,
                &surface.image_snapshot(),
                &Default::default(),
            )
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
    }
}
fn render_file(file: &std::path::Path) -> lazypencil::Result<render::RenderedImage> {
    let store = Store::open(file, None).unwrap();
    let project = Project::load(&store)?;
    let snapshot = layout::compute(project.resolve()?, &mut Fonts::new(&[]).unwrap())?;
    render::png(&project, &snapshot, &RenderOptions::default())
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
fn text(properties: Value) -> Scene {
    let mut node = json!({"id":"text","type":"text","x":12,"y":12,"width":150,"height":80,"textGrowth":"fixed-width-height","fontSize":32,"content":"A A 日本語","fill":"#FF0000"});
    node.as_object_mut()
        .unwrap()
        .extend(properties.as_object().unwrap().clone());
    Scene::new(
        json!([{"id":"frame","type":"frame","layout":"none","width":180,"height":120,"children":[node]}]),
    )
}

#[test]
fn text_layer_bounds_preserve_native_alpha_at_fractional_scales_and_rotations() {
    use lazypencil::geometry::Affine;
    use skia_safe::{BlendMode, Matrix, Paint, Path, Rect, canvas::SaveLayerRec};
    for ((rotation, flip, decoration, parent_opacity), nested) in [
        (0.0, false, false, 1.0),
        (17.0, false, true, 1.0),
        (-43.0, true, true, 1.0),
        (0.0, false, false, 0.63),
        (17.0, false, true, 0.63),
        (-43.0, true, true, 0.63),
    ]
    .into_iter()
    .flat_map(|case| [(case, false), (case, true)])
    {
        let offset = if nested { [37.5, 44.25] } else { [0.0, 0.0] };
        let mut children = json!([{"id":"frame","type":"frame","layout":"none","x":offset[0],"y":offset[1],"width":300,"height":260,"clip":true,"opacity":parent_opacity,"children":[{
            "id":"text","type":"text","x":74.25,"y":70.5,"width":180,"height":120,
            "textGrowth":"fixed-width-height","content":"jÁg 日本語\nAV W","fontSize":26.25,
            "fontStyle":"italic","textAlign":"center","lineHeight":1.4,"fill":"#FF0000B0",
            "rotation":rotation,"flipX":flip,"underline":decoration,"strikethrough":decoration
        }]}]);
        if nested {
            children = json!([{"id":"canvas","type":"frame","layout":"none","width":580,"height":440,"clip":true,"children":children}]);
        }
        let scene = Scene::new(children);
        let file = scene.directory.path().join("scene.pen");
        fs::write(
            &file,
            json!({"version":"2.21","children":scene.children}).to_string(),
        )
        .unwrap();
        let store = Store::open(&file, None).unwrap();
        let project = Project::load(&store).unwrap();
        let snapshot =
            layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap()).unwrap();
        let index = snapshot
            .items
            .iter()
            .position(|item| item.record.id == "text")
            .unwrap();
        let block = &snapshot.text_blocks[&index];
        for scale in [0.25_f32, 0.75, 1.5] {
            let output = render::png(
                &project,
                &snapshot,
                &RenderOptions {
                    scale,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut reference =
                skia_safe::surfaces::raster_n32_premul((output.width as i32, output.height as i32))
                    .unwrap();
            let canvas = reference.canvas();
            canvas.clear(Color::TRANSPARENT);
            let viewport = Affine::scale(scale, scale)
                .then(Affine::translation(-output.region.x, -output.region.y));
            let parent = snapshot
                .items
                .iter()
                .find(|item| item.record.id == "frame")
                .unwrap();
            canvas.concat(&Matrix::from_affine(
                &viewport.then(parent.record.transform).0,
            ));
            if parent_opacity < 1.0 {
                let mut parent = Paint::default();
                parent.set_alpha_f(parent_opacity);
                canvas.save_layer(&SaveLayerRec::default().paint(&parent));
            }
            canvas.clip_rect(Rect::from_xywh(0.0, 0.0, 300.0, 260.0), None, true);
            canvas.reset_matrix();
            canvas.concat(&Matrix::from_affine(
                &viewport.then(snapshot.items[index].record.transform).0,
            ));
            // Preserve the original unbounded compositor, including its
            // antialiased fill geometry. An infinite draw_paint would remove
            // the original coverage multiplication along the fill boundary.
            canvas.save_layer(&SaveLayerRec::default());
            let mut fill = Paint::default();
            fill.set_color(Color::from_argb(176, 255, 0, 0));
            fill.set_anti_alias(true);
            let ink_bounds = block.ink_bounds;
            let fill_bounds = Rect::new(
                0.0_f32.min(ink_bounds.x),
                0.0_f32.min(ink_bounds.y),
                snapshot.items[index]
                    .record
                    .rect
                    .width
                    .max(block.metrics.longest_line)
                    .max(ink_bounds.right()),
                block.metrics.height.max(ink_bounds.bottom()),
            );
            canvas.draw_path(&Path::rect(fill_bounds, None), &fill);
            let mut mask = Paint::default();
            mask.set_blend_mode(BlendMode::DstIn);
            canvas.save_layer(&SaveLayerRec::default().paint(&mask));
            block.paragraph.paint(canvas, (0.0, 0.0));
            canvas.restore();
            canvas.restore();
            if parent_opacity < 1.0 {
                canvas.restore();
            }
            let expected = reference.image_snapshot();
            let expected = expected.peek_pixels().unwrap();
            let image = raster(&output);
            let actual = image.peek_pixels().unwrap();
            let mut ink = 0;
            for y in 0..output.height as i32 {
                for x in 0..output.width as i32 {
                    let color = actual.get_color((x, y));
                    ink += usize::from(color.a() != 0);
                    assert_eq!(
                        color,
                        expected.get_color((x, y)),
                        "scale={scale} rotation={rotation} flip={flip} parent_opacity={parent_opacity} nested={nested} at {x},{y}"
                    );
                }
            }
            assert!(ink > 20);
        }
    }
    // Original lazypencil output, not a Pencil reference. The c355f19 hosted
    // artifacts from run 40281460362 establish each platform's unchanged
    // shadow/mesh/text compositor. Hash decoded RGBA rather than PNG encoding.
    // This caught a one-channel rounding regression hidden by isolated text.
    let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "ff94788fec42537e375b5841f906bd4917d42b66deac893f7b6f080f0714fc13",
        ("linux", "aarch64") => "c9bbe604900f14172516e644d6dd4a578df16e89092abf9bef6f45a4e4b6bf21",
        ("macos", "aarch64") => "bae66e1b870d9953e08fc5600054b34aabadbb6b383671f53256f18911313c4e",
        // Retain the existing Windows/Intel implementation without promising
        // an original-pixel baseline outside the initial release platforms.
        _ => return,
    };
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("mesh-shadow.pen");
    fs::write(&path, include_bytes!("../examples/mesh-study.pen")).unwrap();
    let output = render_file(&path).unwrap();
    assert_eq!((output.width, output.height), (1040, 900));
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    let mut rgba = Vec::with_capacity(1040 * 900 * 4);
    for y in 0..900 {
        for x in 0..1040 {
            let color = pixels.get_color((x, y));
            rgba.extend_from_slice(&[color.r(), color.g(), color.b(), color.a()]);
        }
    }
    assert_eq!(lazypencil::document::hash(&rgba), expected);
}

#[test]
fn decorations_add_ink_to_shaped_text_without_changing_line_layout() {
    let plain = text(json!({})).render().unwrap();
    let base = raster(&plain);
    let base_pixels = base.peek_pixels().unwrap();
    for properties in [
        json!({"underline":true}),
        json!({"strikethrough":true}),
        json!({"underline":true,"strikethrough":true}),
    ] {
        let decorated = text(properties).render().unwrap();
        assert_eq!(
            (decorated.width, decorated.height),
            (plain.width, plain.height)
        );
        let image = raster(&decorated);
        let pixels = image.peek_pixels().unwrap();
        let added = (0..120)
            .flat_map(|y| (0..180).map(move |x| (x, y)))
            .filter(|point| {
                base_pixels.get_color(*point).a() == 0 && pixels.get_color(*point).a() > 200
            })
            .count();
        assert!(
            added > 30,
            "Decoration produced too little new ink: {added}"
        );
        assert_eq!(pixels.get_color((0, 0)).a(), 0);
    }
    assert_eq!(
        text(json!({"underline":"yes"}))
            .render()
            .err()
            .unwrap()
            .code,
        "INVALID_DOCUMENT"
    );
}

#[test]
fn text_strokes_follow_glyph_outlines_and_respect_alignment() {
    let plain = text(json!({})).render().unwrap();
    let base = raster(&plain);
    let base_pixels = base.peek_pixels().unwrap();
    for alignment in ["inner", "center", "outer"] {
        let stroked =
            text(json!({"fill":[],"stroke":"#0000FF","strokeWidth":2,"strokeAlignment":alignment}))
                .render()
                .unwrap();
        let image = raster(&stroked);
        let pixels = image.peek_pixels().unwrap();
        let mut ink = 0;
        let mut outside = 0;
        for y in 0..120 {
            for x in 0..180 {
                let color = pixels.get_color((x, y));
                if color.a() > 200 {
                    ink += 1;
                    assert_eq!(color.b(), 255);
                    if base_pixels.get_color((x, y)).a() == 0 {
                        outside += 1;
                    }
                }
            }
        }
        assert!(ink > 100);
        if alignment == "inner" {
            assert_eq!(outside, 0);
        } else {
            assert!(outside > 50, "{alignment}: {outside}");
        }
        assert_eq!(
            pixels.get_color((12, 12)).a(),
            0,
            "A stroke must not outline the text box"
        );
    }
}

#[test]
fn text_strokes_include_underline_and_strikethrough_geometry() {
    let plain = text(json!({"content":"A A", "fill":[], "stroke":"#0000FF", "strokeWidth":2}))
        .render()
        .unwrap();
    for decorations in [
        json!({"underline":true}),
        json!({"strikethrough":true}),
        json!({"underline":true,"strikethrough":true}),
    ] {
        let mut properties =
            json!({"content":"A A", "fill":[], "stroke":"#0000FF", "strokeWidth":2});
        properties
            .as_object_mut()
            .unwrap()
            .extend(decorations.as_object().unwrap().clone());
        let decorated = text(properties).render().unwrap();
        assert_ne!(
            plain.png, decorated.png,
            "Decorations must contribute to stroke-only text: {decorations}"
        );
    }
}

#[test]
fn decoration_strokes_share_the_native_fill_mask_and_do_not_accumulate_alpha() {
    for decorated in [false, true] {
        let properties = json!({"content":"A g 日本語\nA A", "fontSize":31.5,"lineHeight":1.7,"letterSpacing":1.25,"underline":decorated,"strikethrough":decorated});
        let mut native = text(properties.clone());
        native.children[0]["height"] = json!(240);
        let native = native.render().unwrap();
        let native = raster(&native);
        let mask = native.peek_pixels().unwrap();
        for (alignment, color) in [
            ("inner", "#0000FF"),
            ("center", "#0000FF80"),
            ("outer", "#0000FF80"),
        ] {
            let mut properties = properties.clone();
            properties["fill"] = json!([]);
            properties["stroke"] = json!(color);
            properties["strokeWidth"] = json!(2);
            properties["strokeAlignment"] = json!(alignment);
            let mut scene = text(properties);
            scene.children[0]["height"] = json!(240);
            let rendered = scene.render().unwrap();
            let rendered = raster(&rendered);
            let actual = rendered.peek_pixels().unwrap();
            let mut ink = 0;
            for y in 0..240 {
                for x in 0..180 {
                    let color = actual.get_color((x, y));
                    if color.a() > 0 {
                        ink += 1;
                        assert_eq!(color.b(), 255);
                    }
                    if alignment == "inner" && color.a() > 200 {
                        assert!(
                            mask.get_color((x, y)).a() > 0,
                            "Inner stroke outside native mask, decorated={decorated}, at {x},{y}"
                        );
                    }
                    if alignment != "inner" {
                        assert!(
                            color.a() <= 128,
                            "Overlapping glyph and decoration contours must be painted once"
                        );
                    }
                }
            }
            assert!(ink > 100);
        }
    }
}

#[test]
fn decoration_strokes_accept_gradient_and_image_paints() {
    for paint in [
        json!({"type":"gradient","gradientType":"linear","rotation":90,"colors":[{"color":"#FF0000","position":0},{"color":"#0000FF","position":1}]}),
        json!({"type":"image","url":"image.png","mode":"cover"}),
    ] {
        let mut images = Vec::new();
        for decorated in [false, true] {
            let scene = text(
                json!({"content":"A A 日本語", "fill":[], "stroke":paint, "strokeWidth":2, "underline":decorated, "strikethrough":decorated}),
            );
            scene.image();
            images.push(raster(&scene.render().unwrap()));
        }
        let before = images[0].peek_pixels().unwrap();
        let after = images[1].peek_pixels().unwrap();
        let mut added = 0;
        for y in 0..120 {
            for x in 0..180 {
                if before.get_color((x, y)).a() == 0 && after.get_color((x, y)).a() > 200 {
                    added += 1;
                    let color = after.get_color((x, y));
                    assert!(
                        color.r() > 0 || color.b() > 0,
                        "A decoration must receive its stroke paint"
                    );
                    assert_eq!(color.g(), 0);
                }
            }
        }
        assert!(added > 50, "Missing gradient/image decoration ink: {paint}");
    }
}

#[test]
fn outlined_text_keeps_wrapped_runs_alignment_and_variable_font_weight() {
    for alignment in ["left", "center", "right"] {
        for weight in ["400", "700"] {
            let properties =
                json!({"content":"A A\n日本語 と A","fontWeight":weight,"textAlign":alignment});
            let mut scene = text(properties.clone());
            scene.children[0]["height"] = json!(220);
            let plain = scene.render().unwrap();
            let image = raster(&plain);
            let base = image.peek_pixels().unwrap();
            let mut properties = properties;
            properties["fill"] = json!([]);
            properties["stroke"] = json!("#0000FF");
            properties["strokeWidth"] = json!(1);
            properties["strokeAlignment"] = json!("inner");
            let mut scene = text(properties);
            scene.children[0]["height"] = json!(220);
            let stroked = scene.render().unwrap();
            let image = raster(&stroked);
            let pixels = image.peek_pixels().unwrap();
            let mut count = 0;
            for y in 0..220 {
                for x in 0..180 {
                    if pixels.get_color((x, y)).a() > 220 {
                        count += 1;
                        assert!(
                            base.get_color((x, y)).a() > 0,
                            "{alignment}/{weight}: {x},{y}"
                        );
                    }
                }
            }
            assert!(count > 100);
        }
    }
    let empty = text(json!({"content":"   \n   ","fill":[],"stroke":"#0000FF","strokeWidth":2}))
        .render()
        .unwrap();
    let image = raster(&empty);
    let pixels = image.peek_pixels().unwrap();
    assert!(
        (0..empty.height as i32)
            .all(|y| (0..empty.width as i32).all(|x| pixels.get_color((x, y)).a() == 0))
    );
}

#[test]
fn per_side_borders_have_independent_widths_and_matching_export_bounds() {
    for (alignment, outside) in [("inner", 0.0), ("center", 0.5), ("outer", 1.0)] {
        let output = Scene::new(json!([{"id":"border","type":"rectangle","width":100,"height":80,"stroke":"#0000FF","strokeWidth":{"top":2,"right":4,"bottom":8,"left":12},"strokeAlignment":alignment}])).render().unwrap();
        assert_eq!(output.region.x, -12.0 * outside);
        assert_eq!(output.region.y, -2.0 * outside);
        assert_eq!(output.width, (100.0 + 16.0 * outside) as u32);
        assert_eq!(output.height, (80.0 + 10.0 * outside) as u32);
        let px = |x: f32, y: f32| {
            pixel(
                &output,
                (x - output.region.x) as i32,
                (y - output.region.y) as i32,
            )
        };
        assert_eq!(px(50.0, 1.0 - 2.0 * outside), Color::BLUE);
        assert_eq!(px(1.0 - 12.0 * outside, 40.0), Color::BLUE);
        assert_eq!(px(97.0 + 4.0 * outside, 40.0), Color::BLUE);
        assert_eq!(px(50.0, 75.0 + 8.0 * outside), Color::BLUE);
        assert_eq!(px(13.0, 40.0).a(), 0);
        assert_eq!(px(50.0, 40.0).a(), 0);
    }
}

#[test]
fn nonrectangular_side_widths_match_top_width_pixels_and_export_bounds() {
    let shapes = [
        json!({"type":"ellipse"}),
        json!({"type":"polygon","polygonCount":3,"cornerRadius":6}),
        json!({"type":"path","geometry":"M0 40 L20 0 L40 40 Z","viewBox":[0,0,40,40]}),
        json!({"type":"path","geometry":"M0 40 L40 0","viewBox":[0,0,40,40]}),
        json!({"type":"text","content":"Aj","fontSize":14,"textGrowth":"fixed-width-height"}),
    ];
    let widths = [
        json!({"top":6,"right":0,"bottom":0,"left":0}),
        json!({"right":6}),
        json!({"bottom":6,"left":6}),
        json!({"top":2,"right":4,"bottom":8,"left":12}),
        json!({}),
    ];
    let mut actual = Vec::new();
    let mut scalar = Vec::new();
    for (alignment_index, alignment) in ["inner", "center", "outer"].iter().enumerate() {
        for (row, shape) in shapes.iter().enumerate() {
            for (column, width) in widths.iter().enumerate() {
                let mut node = json!({"id":format!("border-{alignment_index}-{row}-{column}"),
                    "x":column*80+20,"y":(alignment_index*shapes.len()+row)*80+20,
                    "width":40,"height":40,"fill":"#ff0000","stroke":"#0000ff",
                    "strokeWidth":width,"strokeAlignment":alignment});
                node.as_object_mut()
                    .unwrap()
                    .extend(shape.as_object().unwrap().clone());
                actual.push(node.clone());
                node["strokeWidth"] = width.get("top").cloned().unwrap_or(json!(0));
                scalar.push(node);
            }
        }
    }
    let canvas = |children| {
        json!([{"id":"canvas","type":"frame","width":400,"height":1200,
        "layout":"none","clip":true,"fill":"#ffffff","children":children}])
    };
    let scene = Scene::new(canvas(actual));
    let output = scene.render().unwrap();
    let reference = Scene::new(canvas(scalar)).render().unwrap();
    assert_eq!(output.region, reference.region);
    assert_eq!(output.png, reference.png);
    let saved: Value =
        serde_json::from_slice(&fs::read(scene.directory.path().join("scene.pen")).unwrap())
            .unwrap();
    assert_eq!(saved["children"], scene.children);
}

#[test]
fn acute_path_miters_are_included_in_export_bounds() {
    let output=Scene::new(json!([{"id":"triangle","type":"path","width":40,"height":40,"viewBox":[0,0,40,40],"geometry":"M0 40 L20 0 L40 40 Z","stroke":"#0000FF","strokeWidth":6,"strokeAlignment":"outer"}])).render().unwrap();
    assert!(output.region.y < -12.0, "{:?}", output.region);
    let x = (20.0 - output.region.x) as i32;
    assert!(pixel(&output, x, 2).b() > 200);
    assert!(pixel(&output, x, 2).a() > 0);
}

#[test]
fn variable_borders_keep_rounded_corners_and_explicit_corner_joins() {
    for join in ["miter", "round", "bevel"] {
        let scene = Scene::new(
            json!([{"id":"border","type":"rectangle","width":60,"height":40,"stroke":"#0000FF","strokeWidth":{"top":8,"left":8,"right":2,"bottom":4},"strokeAlignment":"outer","strokeLinejoin":join}]),
        );
        let output = scene.render().unwrap();
        assert_eq!(
            pixel(&output, 0, 0).a(),
            if join == "miter" { 255 } else { 0 }
        );
        assert_eq!(pixel(&output, 30, 2), Color::BLUE);
    }
    let output=Scene::new(json!([{"id":"border","type":"rectangle","width":80,"height":60,"cornerRadius":16,"stroke":"#0000FF","strokeWidth":{"top":2,"left":10,"right":4,"bottom":8},"strokeAlignment":"inner"}])).render().unwrap();
    assert_eq!(pixel(&output, 0, 0).a(), 0);
    assert_eq!(pixel(&output, 40, 0), Color::BLUE);
    assert_eq!(pixel(&output, 2, 30), Color::BLUE);
    assert_eq!(pixel(&output, 40, 30).a(), 0);
}

#[test]
fn normalized_image_crops_preserve_the_requested_half_and_sizing_mode() {
    for mode in ["cover", "contain", "stretch"] {
        let scene = Scene::new(
            json!([{"id":"image","type":"rectangle","width":80,"height":40,"fill":{"type":"image","url":"image.png","mode":mode,"transform":[2,0,0,1,-1,0]}}]),
        );
        scene.image();
        let output = scene.render().unwrap();
        assert_eq!(pixel(&output, 40, 20), Color::BLUE);
        if mode == "contain" {
            assert_eq!(pixel(&output, 5, 20).a(), 0);
            assert_eq!(pixel(&output, 75, 20).a(), 0);
        } else {
            assert_eq!(pixel(&output, 10, 20), Color::BLUE);
            assert_eq!(pixel(&output, 70, 20), Color::BLUE);
        }
    }
    let scene = Scene::new(
        json!([{"id":"image","type":"rectangle","width":40,"height":40,"fill":{"type":"image","url":"image.png","transform":[2,0,0,1,0,0]}}]),
    );
    scene.image();
    assert_eq!(pixel(&scene.render().unwrap(), 20, 20), Color::RED);
}

#[test]
fn invalid_image_paints_report_the_supported_modes_and_complete_update_shape() {
    for mode in [json!("fit"), json!(7)] {
        let scene = Scene::new(
            json!([{"id":"image","type":"rectangle","width":80,"height":40,"fill":{"type":"image","url":"image.png","mode":mode}}]),
        );
        scene.image();
        let error = scene.render().err().expect("invalid image mode");
        assert_eq!(error.code, "INVALID_DOCUMENT");
        assert_eq!(
            error.details.unwrap()["allowed_modes"],
            json!(["cover", "contain", "stretch"])
        );
    }
    let scene = Scene::new(
        json!([{"id":"image","type":"rectangle","width":80,"height":40,"fill":{"url":"image.png","mode":"contain"}}]),
    );
    let error = scene.render().err().expect("a partial paint has no type");
    assert_eq!(error.code, "INVALID_DOCUMENT");
    assert!(error.message.contains("entire paint"));
    assert_eq!(error.details.unwrap()["example"]["type"], "image");
}

#[test]
fn rotated_crops_rotate_pixels_and_cover_keeps_translated_or_sheared_views_inside_the_image() {
    let mut scene = Scene::new(
        json!([{"id":"image","type":"rectangle","width":40,"height":40,"fill":{"type":"image","url":"image.png","mode":"stretch","transform":[0,1,-1,0,1,0]}}]),
    );
    scene.image();
    let output = scene.render().unwrap();
    assert_eq!(pixel(&output, 20, 8), Color::RED);
    assert_eq!(pixel(&output, 20, 32), Color::BLUE);
    for matrix in [
        json!([1, 0, 0, 1, 1, 0]),
        json!([1, 0.5, 0, 1, 0, 0]),
        json!([0.7, 0.7, -0.7, 0.7, 0.5, -0.2]),
    ] {
        scene.children[0]["fill"]["transform"] = matrix;
        scene.children[0]["fill"]["mode"] = json!("cover");
        let output = scene.render().unwrap();
        for point in [(0, 0), (39, 0), (0, 39), (39, 39), (20, 20)] {
            assert_eq!(pixel(&output, point.0, point.1).a(), 255, "{point:?}");
        }
    }
    scene.children[0]["fill"]["transform"] = json!([0, 0, 0, 0, 0, 0]);
    assert_eq!(scene.render().err().unwrap().code, "INVALID_DOCUMENT");
}

#[test]
fn inherited_image_strokes_keep_their_library_origin_and_cover_the_outer_border() {
    let scene = Scene::new(json!([]));
    scene.image();
    let root = scene.directory.path();
    fs::create_dir(root.join("lib")).unwrap();
    let mut surface = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
    surface.canvas().clear(Color::GREEN);
    fs::write(
        root.join("lib/image.png"),
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    fs::write(root.join("lib/ui.pen"),json!({"version":"2.21","children":[{"id":"card","type":"rectangle","reusable":true,"width":40,"height":40,"stroke":{"type":"image","url":"image.png"},"strokeWidth":4,"strokeAlignment":"outer"}]}).to_string()).unwrap();
    let file = root.join("scene.pen");
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"lib/ui.pen"},"children":[{"id":"card","type":"ref","ref":"ui:card","fill":{"type":"image","url":"image.png"}}]}).to_string()).unwrap();
    let output = render_file(&file).unwrap();
    assert_eq!(output.assets.len(), 2);
    assert_eq!(pixel(&output, 1, 24), Color::GREEN);
    assert_eq!(pixel(&output, 14, 24), Color::RED);
    assert_eq!(pixel(&output, 34, 24), Color::BLUE);
}

#[test]
fn inner_shadows_stay_inside_the_object_and_preserve_translucent_alpha() {
    for alpha in ["FF", "80"] {
        let output=Scene::new(json!([{"id":"shadow","type":"rectangle","width":40,"height":40,"fill":format!("#FF0000{alpha}"),"effect":{"type":"shadow","shadowType":"inner","offset":{"x":8,"y":0},"color":"#0000FF","blur":0}}])).render().unwrap();
        assert_eq!((output.width, output.height), (40, 40));
        let edge = pixel(&output, 3, 20);
        let center = pixel(&output, 20, 20);
        assert_eq!(edge.b(), 255);
        assert_eq!(edge.a(), center.a());
        assert_eq!(center.a(), if alpha == "FF" { 255 } else { 128 });
        if alpha == "FF" {
            assert_eq!(center, Color::RED);
        }
    }
    let output=Scene::new(json!([{"id":"shadow","type":"rectangle","width":60,"height":60,"cornerRadius":12,"fill":"#FF000080","effect":{"type":"shadow","shadowType":"inner","offset":{"x":3,"y":3},"color":"#0000FF","blur":8}}])).render().unwrap();
    assert_eq!(pixel(&output, 0, 0).a(), 0);
    assert!(pixel(&output, 5, 30).b() > pixel(&output, 30, 30).b());
    assert_eq!(pixel(&output, 30, 30).a(), 128);
}

#[test]
fn inactive_and_zero_width_strokes_do_not_reserve_space_or_execute_shaders() {
    for (enabled, width) in [(false, json!({"top":8,"left":12})), (true, json!(0))] {
        let scene = Scene::new(
            json!([{"id":"rect","type":"rectangle","width":40,"height":20,"fill":"#FF0000","stroke":{"type":"shader","enabled":enabled,"url":"unused.glsl"},"strokeWidth":width,"strokeAlignment":"outer"}]),
        );
        fs::write(
            scene.directory.path().join("unused.glsl"),
            "This is deliberately not valid GLSL.",
        )
        .unwrap();
        let output = scene.render().unwrap();
        assert_eq!((output.width, output.height), (40, 20));
        assert_eq!(pixel(&output, 20, 10), Color::RED);
        assert!(output.dynamic_artifacts.is_empty());
    }
}

#[test]
fn the_bundled_graphics_example_renders_without_layout_diagnostics() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/graphics-study.pen");
    let store = Store::open(&file, None).unwrap();
    let project = Project::load(&store).unwrap();
    let snapshot =
        layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap()).unwrap();
    assert!(
        snapshot.summary()["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let output = render::png(&project, &snapshot, &Default::default()).unwrap();
    assert_eq!((output.width, output.height), (1000, 760));
    assert_eq!(output.assets.len(), 1);
}

#[test]
fn tight_line_height_does_not_crop_glyphs_or_decorations_outside_the_line_box() {
    use lazypencil::text::TextSpec;
    let content = "日本語";
    let mut fonts = Fonts::new(&[]).unwrap();
    let reference = fonts
        .layout(
            &TextSpec {
                content: content.into(),
                size: 64.0,
                line_height: Some(0.5),
                underline: true,
                color: Color::WHITE,
                ..Default::default()
            },
            None,
        )
        .unwrap();
    let mut surface = skia_safe::surfaces::raster_n32_premul((260, 180)).unwrap();
    surface.canvas().clear(Color::TRANSPARENT);
    reference.paragraph.paint(surface.canvas(), (20.0, 70.0));
    let image = surface
        .image_snapshot()
        .make_raster_image(None, None)
        .unwrap();
    let expected = image.peek_pixels().unwrap();
    let output=Scene::new(json!([{"id":"label","type":"text","content":content,"fontSize":64,"lineHeight":0.5,"underline":true,"fill":"#FFFFFF"}])).render().unwrap();
    let image = raster(&output);
    let pixels = image.peek_pixels().unwrap();
    let mut ink = 0;
    for y in 0..180 {
        for x in 0..260 {
            if expected.get_color((x, y)).a() > 220 {
                ink += 1;
                let px = (x as f32 - 20.0 - output.region.x).floor() as i32;
                let py = (y as f32 - 70.0 - output.region.y).floor() as i32;
                assert!(
                    px >= 0 && py >= 0 && px < output.width as i32 && py < output.height as i32,
                    "Ink outside export: {x},{y}, {:?}",
                    output.region
                );
                assert!(
                    pixels.get_color((px, py)).a() > 0,
                    "Ink was clipped at {px},{py}"
                );
            }
        }
    }
    assert!(ink > 1000);
}

#[test]
fn independent_contract_triangle_matches_geometry_and_survives_an_edit_without_migration() {
    use lazypencil::{operations::EditRequest, service};
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("native.pen");
    let source = include_str!("fixtures/render-corpus/triangle.pen");
    fs::write(&file, source).unwrap();
    let store = Store::open(&file, Some(directory.path().join("data"))).unwrap();
    let output = render_file(&file).unwrap();
    assert_eq!((output.width, output.height), (144, 144));
    assert_native_pixels(
        &output,
        &render_oracles::shapes::raster(&serde_json::from_str(source).unwrap(), 1),
        150,
    );
    assert_eq!(pixel(&output, 66, 25), Color::RED);
    assert_eq!(pixel(&output, 66, 105), Color::WHITE);
    let metadata = service::document_metadata(&store).unwrap();
    let request: EditRequest = serde_json::from_value(json!({"request_id":"native-219-edit",
        "expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],
        "operations":[{"op":"update","target":"triangle","set":{"fill":"#0000ff"}}]})).unwrap();
    service::apply_request(&store, &request).unwrap();
    let saved = store.read().unwrap();
    assert_eq!(saved.version(), "2.19");
    assert_eq!(saved.text(), source.replace("#ff0000", "#0000ff"));
    let edited = render_file(&file).unwrap();
    assert_eq!(pixel(&edited, 66, 25), Color::BLUE);
    assert_eq!(pixel(&edited, 66, 105), Color::WHITE);
    if let Some(path) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        let path = std::path::PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("codec-triangle.png"), &output.png).unwrap();
        fs::write(path.join("codec-edited.pen"), saved.text()).unwrap();
        fs::write(path.join("codec-edited.png"), &edited.png).unwrap();
    }
}

fn assert_native_pixels(
    output: &render::RenderedImage,
    expected: &render_oracles::Raster,
    max_changed: usize,
) {
    assert_eq!(
        (output.width, output.height),
        (expected.width, expected.height)
    );
    let actual = raster(output);
    let actual_pixels = actual.peek_pixels().unwrap();
    let mut difference = 0u64;
    let mut changed_pixels = 0;
    for y in 0..expected.height as i32 {
        for x in 0..expected.width as i32 {
            let a = actual_pixels.get_color((x, y));
            let [er, eg, eb, ea] = expected.pixel(x as u32, y as u32);
            let b = Color::from_argb(ea, er, eg, eb);
            let channels = [
                a.a().abs_diff(b.a()),
                a.r().abs_diff(b.r()),
                a.g().abs_diff(b.g()),
                a.b().abs_diff(b.b()),
            ];
            difference += channels.iter().map(|value| u64::from(*value)).sum::<u64>();
            changed_pixels += usize::from(channels.iter().any(|value| *value > 20));
        }
    }
    eprintln!(
        "Native {}x{}: absolute channel difference={difference}, pixels over20={changed_pixels}",
        output.width, output.height
    );
    // Permit edge antialiasing differences, while rejecting shifted geometry.
    assert!(
        difference < u64::from(output.width) * u64::from(output.height) * 4 * 2,
        "Native pixel difference: {difference}"
    );
    assert!(
        changed_pixels < max_changed,
        "Native differing pixels: {changed_pixels}"
    );
}

#[test]
fn independent_contract_rounded_polygons_match_aspect_ratios_and_oversized_radii() {
    for (name, source, max_changed) in [
        (
            "rounded-polygons",
            include_str!("fixtures/render-corpus/rounded-polygons.pen"),
            600,
        ),
        (
            "radius-triangles",
            include_str!("fixtures/render-corpus/radius-triangles.pen"),
            900,
        ),
        (
            "rounded-large-aspects",
            include_str!("fixtures/render-corpus/rounded-large-aspects.pen"),
            600,
        ),
        (
            "wide-radius-triangles",
            include_str!("fixtures/render-corpus/wide-radius-triangles.pen"),
            750,
        ),
    ] {
        for version in ["2.19", "2.21"] {
            eprintln!("Reference {name}, schema {version}");
            let directory = TempDir::new().unwrap();
            let file = directory.path().join("native.pen");
            fs::write(&file, source.replace("2.19", version)).unwrap();
            let output = render_file(&file).unwrap();
            let expected =
                render_oracles::shapes::raster(&serde_json::from_str(source).unwrap(), 1);
            assert_native_pixels(&output, &expected, max_changed);
            if let Some(path) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                let path = std::path::PathBuf::from(path);
                fs::create_dir_all(&path).unwrap();
                fs::write(
                    path.join(format!("codec-{name}-{version}.png")),
                    &output.png,
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn independent_contract_nonrectangular_side_strokes_match_top_width_reference() {
    let source = include_str!("fixtures/render-corpus/nonrect-strokes.pen");
    let expected = render_oracles::shapes::raster(&serde_json::from_str(source).unwrap(), 2);
    assert_eq!((expected.width, expected.height), (1760, 480));
    for version in ["2.19", "2.21"] {
        let directory = TempDir::new().unwrap();
        let file = directory.path().join("native.pen");
        let source = source.replace("2.19", version);
        fs::write(&file, &source).unwrap();
        let store = Store::open(&file, None).unwrap();
        let project = Project::load(&store).unwrap();
        let snapshot =
            layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap()).unwrap();
        let diagnostics = snapshot.summary()["diagnostics"].clone();
        assert!(diagnostics.as_array().unwrap().is_empty());
        let output = render::png(
            &project,
            &snapshot,
            &RenderOptions {
                scale: 2.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((output.width, output.height), (1760, 480));
        let actual_image = raster(&output);
        let actual = actual_image.peek_pixels().unwrap();
        let mut cases = Vec::new();
        for row in 0..3 {
            for column in 0..11 {
                let mut absolute = 0u64;
                let mut changed = 0usize;
                let mut maximum = 0u8;
                for y in row * 160..(row + 1) * 160 {
                    for x in column * 160..(column + 1) * 160 {
                        let a = actual.get_color((x, y));
                        let [er, eg, eb, ea] = expected.pixel(x as u32, y as u32);
                        let b = Color::from_argb(ea, er, eg, eb);
                        let deltas = [
                            a.a().abs_diff(b.a()),
                            a.r().abs_diff(b.r()),
                            a.g().abs_diff(b.g()),
                            a.b().abs_diff(b.b()),
                        ];
                        absolute += deltas.iter().map(|value| u64::from(*value)).sum::<u64>();
                        changed += usize::from(deltas.iter().any(|value| *value > 20));
                        maximum = maximum.max(*deltas.iter().max().unwrap());
                    }
                }
                cases.push(json!({"id":format!("border-{row}-{column}"),"row":row,"column":column,
                    "absolute_channel_difference":absolute,"mean_channel_difference":absolute as f64/(160.0*160.0*4.0),
                    "maximum_channel_difference":maximum,"pixels_over20":changed}));
            }
        }
        if let Some(path) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
            let path = std::path::PathBuf::from(path);
            fs::create_dir_all(&path).unwrap();
            let name = format!("codec-nonrect-strokes-{version}");
            fs::write(path.join(format!("{name}.png")), &output.png).unwrap();
            fs::write(path.join(format!("{name}.json")), json!({"source_commit":std::env::var("GITHUB_SHA").ok(),
                "document_version":version,"width":output.width,"height":output.height,"diagnostics":diagnostics,
                "png_hash":lazypencil::document::hash(&output.png),"cases":cases,
                "oracle":"independent-contours-v1","oracle_rgba_sha256":lazypencil::document::hash(expected.pixels.as_flattened())}).to_string()).unwrap();
        }
        for case in cases {
            assert!(
                case["mean_channel_difference"].as_f64().unwrap() < 2.0,
                "Native stroke {version}: {case}"
            );
            assert!(
                case["pixels_over20"].as_u64().unwrap() < 400,
                "Native stroke {version}: {case}"
            );
        }
        assert_eq!(fs::read(&file).unwrap(), source.as_bytes());
    }
}

#[test]
fn polygon_radius_validation_and_degenerate_shapes_do_not_create_invalid_arcs() {
    for radius in [json!(-1), json!("large"), json!([10, 10, 10, 10])] {
        let scene = Scene::new(
            json!([{"id":"polygon","type":"polygon","width":100,"height":100,"cornerRadius":radius,"fill":"#ff0000"}]),
        );
        assert_eq!(scene.render().err().unwrap().code, "INVALID_DOCUMENT");
    }
    for (width, height) in [(0, 100), (100, 0), (0, 0)] {
        let scene = Scene::new(
            json!([{"id":"frame","type":"frame","width":144,"height":144,"layout":"none","fill":"#ffffff","children":[{"id":"polygon","type":"polygon","width":width,"height":height,"cornerRadius":100,"fill":"#ff0000"}]}]),
        );
        let output = scene.render().unwrap();
        assert_eq!(pixel(&output, 20, 20), Color::WHITE);
    }
}
