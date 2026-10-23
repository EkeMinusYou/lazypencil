#![cfg(feature = "rendering")]
use base64::Engine;
use lazypencil::{
    document::Document,
    dynamic::DynamicRuntime,
    project::Project,
    render::{self, RenderOptions},
    text::Fonts,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use tempfile::TempDir;

fn executable() -> PathBuf {
    std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
        .map(PathBuf::from)
        .expect("Set LAZYPENCIL_TEST_CHROMIUM to a verified headless Chromium executable")
}
fn scene(children: Value) -> (TempDir, Project, lazypencil::layout::LayoutSnapshot) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("scene.pen");
    let source = json!({"version":"2.21","children":children}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    (temp, project, layout)
}
fn raster(bytes: &[u8]) -> Image {
    Image::from_encoded(Data::new_copy(bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
}
fn page(html: &str) -> String {
    format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(html)
    )
}

#[test]
fn uniform_annotations_bind_to_the_correct_declaration() {
    let parsed=lazypencil::shader::uniforms("/* @resolution */ uniform highp vec2 size;\n/* @time */ uniform float clock;\nuniform vec4 color; /* @color */\n").unwrap();
    assert_eq!(parsed.len(), 3);
    assert!(parsed[0].resolution);
    assert!(!parsed[0].time);
    assert!(parsed[1].time);
    assert!(!parsed[2].time);
    assert!(lazypencil::shader::uniforms("/* @resolution */ uniform float broken;").is_err());
    let arrays = lazypencil::shader::uniforms("uniform vec2 array[4];").unwrap();
    assert!(arrays[0].array);
    assert_eq!(arrays[0].length, Some(4));
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn browser_pixels_are_captured_composited_and_reused_until_refresh() {
    let url = page(
        "<style>html,body{margin:0;background:#00f}div{width:20px;height:100vh;background:#f00}</style><div></div>",
    );
    let (temp, project, layout) = scene(
        json!([{"id":"page","type":"browser","url":url,"width":80,"height":40,"cornerRadius":4}]),
    );
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let a = render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
        .unwrap();
    let image = raster(&a.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((10, 20)), Color::RED);
    assert_eq!(pixels.get_color((50, 20)), Color::BLUE);
    assert!(pixels.get_color((0, 0)).a() < 255);
    let b = render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
        .unwrap();
    assert_eq!(
        a.dynamic_artifacts[0]["acquisition"]["acquisition_id"],
        b.dynamic_artifacts[0]["acquisition"]["acquisition_id"]
    );
    let c = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_ne!(
        a.dynamic_artifacts[0]["acquisition"]["acquisition_id"],
        c.dynamic_artifacts[0]["acquisition"]["acquisition_id"]
    );
}

#[test]
#[ignore = "Requires headless Chromium, software WebGL, and permission to launch its sandbox"]
fn page_and_shader_processes_expire_independently_preserve_rasters_and_restart() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("blue.glsl"),
        "precision highp float; void main(){gl_FragColor=vec4(0.,0.,1.,1.);}",
    )
    .unwrap();
    let source = json!({"version":"2.21","children":[
        {"id":"page","type":"browser","width":16,"height":16,"url":page("<style>html{background:#f00}</style>")},
        {"id":"shader","type":"rectangle","x":16,"width":16,"height":16,"fill":{"type":"shader","url":"blue.glsl"}}
    ]}).to_string();
    fs::write(temp.path().join("scene.pen"), &source).unwrap();
    let project = Project::from_document(
        &temp.path().join("scene.pen"),
        Document::parse(source).unwrap(),
        &[],
    )
    .unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    assert_eq!(runtime.idle_deadline(), None);
    let first =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let pids = runtime.process_ids();
    assert_eq!(pids.len(), 2);
    let deadline = runtime.idle_deadline().unwrap();
    runtime.expire_idle(deadline - Duration::from_nanos(1));
    assert_eq!(runtime.process_ids(), pids);
    let repeated = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_eq!(runtime.process_ids(), pids);
    assert_eq!(first.png, repeated.png);
    let first_deadline = runtime.idle_deadline().unwrap();
    runtime.expire_idle(first_deadline);
    assert_eq!(runtime.process_ids(), vec![pids[1]]);
    let last_deadline = runtime.idle_deadline().unwrap();
    assert!(last_deadline > first_deadline);
    runtime.expire_idle(last_deadline);
    assert!(runtime.process_ids().is_empty());
    assert_eq!(runtime.idle_deadline(), None);
    #[cfg(unix)]
    for pid in &pids {
        assert_eq!(
            unsafe { libc::waitpid(*pid as i32, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
    let cached =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert!(runtime.process_ids().is_empty());
    assert_eq!(cached.png, repeated.png);
    assert_eq!(cached.dynamic_artifacts, repeated.dynamic_artifacts);
    let refreshed = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_eq!(runtime.process_ids().len(), 2);
    assert!(runtime.process_ids().iter().all(|pid| !pids.contains(pid)));
    assert_eq!(refreshed.png, first.png);
    assert_ne!(refreshed.dynamic_artifacts, cached.dynamic_artifacts);
    let image = raster(&refreshed.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((8, 8)), Color::RED);
    assert_eq!(pixels.get_color((24, 8)), Color::BLUE);
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn software_webgl_pixels_follow_resolution_time_and_color_uniforms() {
    let temp = TempDir::new().unwrap();
    let shader = temp.path().join("fill.glsl");
    fs::write(&shader,"precision highp float;\n/* @resolution */ uniform vec2 size;\n/* @time */ uniform float clock;\nuniform vec4 tint;\nvoid main(){vec2 p=gl_FragCoord.xy/size;gl_FragColor=vec4(p.x,clock,tint.b,tint.a);}").unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","children":[{"id":"rect","type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":"fill.glsl","uniforms":{"tint":"#0000FFFF"}}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let output = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            time: 0.5,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    let image = raster(&output.png);
    let color = image.peek_pixels().unwrap().get_color((20, 10));
    assert!(color.r() > 120 && color.r() < 140);
    assert!((color.g() as i32 - 128).abs() < 2);
    assert_eq!(color.b(), 255);
    assert!(
        output.dynamic_artifacts[0]["acquisition"]["renderer"]
            .as_str()
            .unwrap()
            .contains("SwiftShader")
    );
    let changed = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            time: 0.0,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_ne!(changed.png, output.png);
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn an_infinite_page_script_can_be_cancelled_and_the_runtime_recovers() {
    let (temp, project, layout) = scene(
        json!([{"id":"page","type":"browser","url":page("<script>for(;;){}</script>"),"width":40,"height":20}]),
    );
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let cancel = tokio_util::sync::CancellationToken::new();
    let token = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        token.cancel();
    });
    let start = Instant::now();
    let error = render::png_with_runtime(
        &project,
        &layout,
        &Default::default(),
        &mut runtime,
        Some(cancel),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "CANCELLED");
    assert!(start.elapsed() < Duration::from_secs(3));
    thread.join().unwrap();
    let (_temp, project, layout) = scene(
        json!([{"id":"page","type":"browser","url":page("<style>body{background:#00f}</style>"),"width":40,"height":20}]),
    );
    render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None).unwrap();
}

fn effect_scene(
    effect: Value,
    children: Value,
) -> (TempDir, Project, lazypencil::layout::LayoutSnapshot) {
    let temp = TempDir::new().unwrap();
    let sample = "precision highp float;\n/* @resolution */ uniform vec2 size;\nuniform sampler2D src;\nvoid main(){gl_FragColor=texture2D(src,gl_FragCoord.xy/size).bgra;}";
    fs::write(temp.path().join("swap.glsl"), sample).unwrap();
    fs::write(
        temp.path().join("sample.glsl"),
        sample.replace(".bgra", ".rgba"),
    )
    .unwrap();
    let mut children = children;
    children[1]["effect"] = effect;
    let path = temp.path().join("scene.pen");
    let source = json!({"version":"2.21","children":children}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    (temp, project, layout)
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn shader_effects_receive_only_the_object_and_run_in_declared_order() {
    let children = json!([
        {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#00FF00"},
        {"id":"front","type":"rectangle","x":20,"y":10,"width":40,"height":20,"fill":"#FF0000"}
    ]);
    let effect = json!({"type":"shader","url":"swap.glsl","uniforms":{"src":"pencil:object"}});
    let (temp, project, layout) = effect_scene(effect.clone(), children.clone());
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((30, 20)), Color::BLUE);
    assert_eq!(pixels.get_color((5, 5)), Color::GREEN);
    let (temp, project, layout) = effect_scene(json!([effect.clone(), effect]), children);
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(
        raster(&output.png)
            .peek_pixels()
            .unwrap()
            .get_color((30, 20)),
        Color::RED
    );
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn background_sampling_includes_the_scene_and_parent_layers() {
    let children = json!([
        {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#0000FF"},
        {"id":"front","type":"frame","x":20,"y":0,"width":40,"height":40,"fill":"#FF0000","opacity":0.5,"children":[
            {"id":"child","type":"rectangle","width":40,"height":40,"fill":{"type":"shader","url":"sample.glsl","uniforms":{"src":"pencil:background"}}}
        ]}
    ]);
    let (temp, project, layout) = effect_scene(json!([]), children);
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    let color = pixels.get_color((30, 20));
    assert!((color.r() as i32 - 128).abs() < 2, "{color:?}");
    assert!((color.b() as i32 - 127).abs() < 2, "{color:?}");
    assert_eq!(color.g(), 0);
    assert_eq!(pixels.get_color((5, 5)), Color::BLUE);
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn browser_zoom_scroll_and_export_scale_change_the_capture() {
    let url = page(
        "<style>html,body{margin:0;width:200px;height:300px;background:#00f}div{width:20px;height:40px;background:#f00}</style><div></div>",
    );
    for (zoom, scroll, scale, point, expected) in [
        (2.0, 0.0, 1.0, (30, 20), Color::RED),
        (1.0, 60.0, 1.0, (10, 20), Color::BLUE),
        (1.0, 0.0, 2.0, (30, 20), Color::RED),
    ] {
        let (temp, project, layout) = scene(
            json!([{"id":"page","type":"browser","url":url,"width":80,"height":40,"zoom":zoom,"scrollY":scroll}]),
        );
        let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
        let output = render::png_with_runtime(
            &project,
            &layout,
            &RenderOptions {
                scale,
                ..Default::default()
            },
            &mut runtime,
            None,
        )
        .unwrap();
        assert_eq!(
            raster(&output.png).peek_pixels().unwrap().get_color(point),
            expected,
            "zoom={zoom} scroll={scroll} scale={scale}"
        );
    }
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn sampler_images_keep_their_orientation_and_fill_opacity() {
    let temp = TempDir::new().unwrap();
    let mut surface = skia_safe::surfaces::raster_n32_premul((8, 4)).unwrap();
    surface.canvas().clear(Color::RED);
    let mut paint = skia_safe::Paint::default();
    paint.set_color(Color::BLUE);
    surface
        .canvas()
        .draw_rect(skia_safe::Rect::from_xywh(0.0, 2.0, 8.0, 2.0), &paint);
    fs::write(
        temp.path().join("texture.png"),
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    fs::write(temp.path().join("sample.glsl"),"precision highp float;\n/* @resolution */ uniform vec2 size;\nuniform sampler2D image;\nvoid main(){gl_FragColor=texture2D(image,gl_FragCoord.xy/size);}").unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","children":[{"id":"rect","type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":"sample.glsl","opacity":0.5,"uniforms":{"image":"texture.png"}}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    let top = pixels.get_color((20, 3));
    let bottom = pixels.get_color((20, 17));
    assert_eq!(top.r(), 255);
    assert_eq!(bottom.b(), 255);
    assert!((top.a() as i32 - 128).abs() < 2);
    assert!((bottom.a() as i32 - 128).abs() < 2);
}

#[test]
fn uniform_names_that_match_other_literal_properties_still_resolve_variables() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("shade.glsl"),
        "precision highp float;uniform float name;void main(){gl_FragColor=vec4(name,0.,0.,1.);}",
    )
    .unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","variables":{"level":{"type":"number","value":0.75}},"children":[{"id":"rect","type":"rectangle","width":20,"height":20,"fill":{"type":"shader","url":"shade.glsl","uniforms":{"name":"$level"}}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let scene = project.resolve().unwrap();
    let fill = &scene.roots[0].properties["fill"];
    let request = lazypencil::shader::request(
        &project,
        &scene.roots[0],
        fill,
        "fill",
        20,
        20,
        0.0,
        None,
        None,
    )
    .unwrap();
    assert_eq!(request["uniforms"]["name"], 0.75);
}

#[test]
#[ignore = "Requires LAZYPENCIL_TEST_CHROMIUM and permission to launch Chromium"]
fn real_cli_uses_the_runtime_and_interrupts_without_writing_an_output() {
    use std::process::Command;
    #[cfg(unix)]
    use std::process::Stdio;
    let (temp, project, _layout) = scene(
        json!([{"id":"page","type":"browser","url":page("<style>body{margin:0;background:#00f}</style>"),"width":40,"height":20}]),
    );
    let file = temp.path().join("scene.pen");
    let output_path = temp.path().join("image.png");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .env("LAZYPENCIL_CHROMIUM", executable())
        .args(["--json", "render"])
        .arg(&file)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        raster(&fs::read(&output_path).unwrap())
            .peek_pixels()
            .unwrap()
            .get_color((10, 10)),
        Color::BLUE
    );
    assert!(project.verify_unchanged().is_ok());
    #[cfg(unix)]
    {
        fs::remove_file(&output_path).unwrap();
        let source = fs::read_to_string(&file).unwrap();
        let mut document: Value = serde_json::from_str(&source).unwrap();
        document["children"][0]["url"] = json!(page("<script>for(;;){}</script>"));
        fs::write(&file, document.to_string()).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .env("LAZYPENCIL_CHROMIUM", executable())
            .args(["--json", "render"])
            .arg(&file)
            .arg("--output")
            .arg(&output_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(130));
        assert!(!output_path.exists());
    }
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn derivatives_and_uv_varyings_link_with_fragment_precision() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("uv.glsl"), "#extension GL_OES_standard_derivatives : enable\nprecision mediump float;varying vec2 v_uv;void main(){gl_FragColor=vec4(v_uv.x,v_uv.y,step(0.01,fwidth(v_uv.x)),1.);}").unwrap();
    let path = temp.path().join("scene.pen");
    let source = json!({"version":"2.21","children":[{"id":"uv","type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":"uv.glsl"}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let color = image.peek_pixels().unwrap().get_color((20, 10));
    assert!((color.r() as i32 - 131).abs() < 3, "{color:?}");
    assert!((color.g() as i32 - 121).abs() < 3, "{color:?}");
    assert_eq!(color.b(), 255);
    assert!(
        output.dynamic_artifacts[0]["acquisition"]["extensions"]
            .as_array()
            .unwrap()
            .contains(&json!("OES_standard_derivatives"))
    );
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn shader_strokes_use_the_library_origin_and_paint_the_outer_border() {
    let temp = TempDir::new().unwrap();
    let library = temp.path().join("lib");
    fs::create_dir(&library).unwrap();
    fs::write(
        library.join("stroke.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(0.,0.,1.,1.);}",
    )
    .unwrap();
    fs::write(
        temp.path().join("stroke.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(0.,1.,0.,1.);}",
    )
    .unwrap();
    fs::write(library.join("ui.pen"),json!({"version":"2.21","children":[{"id":"card","type":"rectangle","reusable":true,"width":40,"height":20,"fill":"#FF0000","stroke":{"type":"shader","url":"stroke.glsl"},"strokeWidth":4,"strokeAlignment":"outer"}]}).to_string()).unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","imports":{"ui":"lib/ui.pen"},"children":[{"id":"card","type":"ref","ref":"ui:card"}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!((output.width, output.height), (48, 28));
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((1, 14)), Color::BLUE);
    assert_eq!(pixels.get_color((24, 14)), Color::RED);
    assert_eq!(output.dynamic_artifacts[0]["node_id"], "card");
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn background_and_shader_effect_order_preserves_replacement_coverage() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("quarter.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(1.,0.,0.,0.25);}",
    )
    .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable());
    for background_first in [true, false] {
        let mut effects = vec![
            json!({"type":"background_blur","radius":8}),
            json!({"type":"shader","url":"quarter.glsl"}),
        ];
        if !background_first {
            effects.reverse();
        }
        let path = temp.path().join("scene.pen");
        let source = json!({"version":"2.21","children":[
            {"id":"back","type":"rectangle","width":60,"height":40,"fill":"#0000FF80"},
            {"id":"glass","type":"rectangle","x":10,"y":10,"width":40,"height":20,"effect":effects}
        ]})
        .to_string();
        fs::write(&path, &source).unwrap();
        let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
        let layout =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        let output =
            render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
                .unwrap();
        let image = raster(&output.png);
        let color = image.peek_pixels().unwrap().get_color((30, 20));
        if background_first {
            assert!((color.a() as i32 - 64).abs() <= 1, "{color:?}");
            assert_eq!(color.r(), 255);
            assert_eq!(color.b(), 0);
        } else {
            assert!((color.a() as i32 - 160).abs() <= 2, "{color:?}");
            assert!((color.r() as i32 - 102).abs() <= 3, "{color:?}");
            assert!((color.b() as i32 - 153).abs() <= 3, "{color:?}");
        }
    }
}
