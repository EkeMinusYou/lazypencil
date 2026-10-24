#![cfg(feature = "rendering")]

use lazypencil::{document::Document, project::Project, shader};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

fn project(root: &TempDir, shader: &str, overrides: Value) -> Project {
    fs::write(root.path().join("input.glsl"), shader).unwrap();
    let path = root.path().join("input.pen");
    let source = json!({"version":"2.21","children":[{"id":"shape","type":"rectangle","width":32,"height":16,
        "fill":{"type":"shader","url":"input.glsl","uniforms":overrides}}]}).to_string();
    fs::write(&path, &source).unwrap();
    Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap()
}

fn request(source: &str, overrides: Value) -> lazypencil::Result<Value> {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("input.glsl"), source).unwrap();
    let path = root.path().join("input.pen");
    let source = json!({"version":"2.21","children":[{"id":"shape","type":"rectangle","width":32,"height":16,
        "fill":{"type":"shader","url":"input.glsl","uniforms":overrides}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let scene = project.resolve().unwrap();
    let node = &scene.roots[0];
    shader::request(
        &project,
        node,
        &node.properties["fill"],
        "fill",
        32,
        16,
        0.25,
        None,
        None,
    )
}

#[test]
fn documentation_defaults_prepare_native_values_and_explicit_zero_wins() {
    let source = "precision highp float;\n/** @default 0.75 */ uniform float value;\n\
        /** @default 0.5 */ uniform float first,second;\n\
        /** @default 3 */ uniform int count;\n\
        /** @default -0.25 */ uniform float negative;\n\
        /** @default 2.5e-1 */ uniform float exponent;\n\
        /** @default 0x2 */ uniform float prefix;\n\
        /** @default 0.25, 0.75 */ uniform vec2 pair;\n\
        /**\n * @color\n * @default #3a7fc1\n */ uniform vec3 color;";
    let result = request(source, json!({"value":0})).unwrap();
    assert_eq!(result["uniforms"]["value"].as_f64(), Some(0.0));
    for name in ["first", "second"] {
        assert_eq!(result["uniforms"][name].as_f64(), Some(0.5));
    }
    assert_eq!(result["uniforms"]["count"], 3);
    assert_eq!(result["uniforms"]["negative"].as_f64(), Some(-0.25));
    assert_eq!(result["uniforms"]["exponent"].as_f64(), Some(0.25));
    assert_eq!(result["uniforms"]["prefix"].as_f64(), Some(0.0));
    assert_eq!(result["uniforms"]["pair"], json!([0.25, 0.75]));
    assert_eq!(
        result["uniforms"]["color"],
        json!([58.0 / 255.0, 127.0 / 255.0, 193.0 / 255.0])
    );
    let before = request("/** @default 0.75 */ uniform float value;", json!({})).unwrap();
    assert_eq!(before["uniforms"]["value"].as_f64(), Some(0.75));
}

#[test]
fn typed_boolean_and_matrix_defaults_keep_stored_false() {
    let source =
        "/** @default 1 */ uniform bool enabled;\n/** @default 1,0,0,1 */ uniform mat2 transform;";
    let defaults = request(source, json!({})).unwrap();
    assert_eq!(defaults["uniforms"]["enabled"], true);
    assert_eq!(
        defaults["uniforms"]["transform"],
        json!([1.0, 0.0, 0.0, 1.0])
    );
    let overridden = request(source, json!({"enabled":false})).unwrap();
    assert_eq!(overridden["uniforms"]["enabled"], false);
}

#[test]
fn default_annotations_ignore_ordinary_trailing_body_and_similarly_named_comments() {
    for source in [
        "/* @default 0.75 */ uniform float value;",
        "uniform float value; /** @default 0.75 */",
        "// @default 0.75\nuniform float value;",
        "/** @default_value 0.75 */ uniform float value;",
        "/** contact@default.example */ uniform float value;",
        "void helper(){/** @default 0.75 */ float unused;} uniform float value;",
    ] {
        let result = request(source, json!({})).unwrap();
        assert_eq!(result["uniforms"]["value"].as_f64(), Some(0.0), "{source}");
    }
    let result = request(
        "uniform float first;\n/** @default .5 */ uniform float second;",
        json!({}),
    )
    .unwrap();
    assert_eq!(result["uniforms"]["first"].as_f64(), Some(0.0));
    assert_eq!(result["uniforms"]["second"].as_f64(), Some(0.5));
}

#[test]
fn array_defaults_mark_one_element_and_keep_short_explicit_overrides_invalid() {
    for source in [
        "/** @default 0.25 */ uniform float value[2];",
        "const int N=2;\n/** @default 0.25 */ uniform float value[N];",
    ] {
        let result = request(source, json!({})).unwrap();
        assert_eq!(result["uniforms"]["value"], json!([0.25]));
        assert_eq!(result["uniform_array_defaults"], json!(["value"]));
        let explicit = request(source, json!({"value":[0.75,0.5]})).unwrap();
        assert_eq!(explicit["uniforms"]["value"], json!([0.75, 0.5]));
        assert!(explicit.get("uniform_array_defaults").is_none());
    }
    assert!(
        request(
            "/** @default 0.25 */ uniform float value[2];",
            json!({"value":[0.5]})
        )
        .is_err()
    );
}

#[test]
fn native_annotation_binding_cases_prepare_the_expected_inputs() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pencil-0.3.9/shader-annotation-binding");
    let active = [
        "doc-direct",
        "plain-doc",
        "doc-after-precision",
        "doc-after-uniform",
        "doc-blank-line",
        "doc-grouped",
        "default-direct",
        "default-after-precision",
        "default-after-uniform",
    ];
    let mut count = 0;
    for entry in fs::read_dir(fixtures).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "glsl") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let result = request(&fs::read_to_string(&path).unwrap(), json!({})).unwrap();
        let enabled = active.contains(&name);
        if name.starts_with("default-") {
            assert_eq!(
                result["uniforms"]["value"].as_f64(),
                Some(if enabled { 0.75 } else { 0.0 }),
                "{name}"
            );
        } else {
            let expected = if enabled {
                json!([32, 16])
            } else {
                json!([0.0, 0.0])
            };
            assert_eq!(result["uniforms"]["size"], expected, "{name}");
            if name == "doc-grouped" {
                assert_eq!(result["uniforms"]["other"], expected);
            }
        }
        count += 1;
    }
    assert_eq!(count, 20);
}

#[test]
fn time_inputs_reject_defaults_and_nonadjacent_metadata_does_not_leak() {
    let error = request(
        "/** @time\n @default 0.5 */ uniform float clock;",
        json!({}),
    )
    .unwrap_err();
    assert_eq!(error.code, "SHADER_COMPILE_FAILED");
    assert!(error.message.contains("@default"));
    for separator in [
        "/* explanation */",
        "/** explanation */",
        "// explanation\n",
        "#define UNUSED 1\n",
        "precision highp float;",
        "void helper(){}",
    ] {
        let source = format!("/** @time */ {separator} uniform float clock;");
        let result = request(&source, json!({})).unwrap();
        assert_eq!(result["uniforms"]["clock"].as_f64(), Some(0.0), "{source}");
        assert_eq!(result["time_dependent"], false, "{source}");
    }
    let result = request(
        "uniform float first; /** @default .5 */ uniform float second;",
        json!({}),
    )
    .unwrap();
    assert_eq!(result["uniforms"]["first"].as_f64(), Some(0.0));
    assert_eq!(result["uniforms"]["second"].as_f64(), Some(0.5));
}

#[test]
fn malformed_or_nonfinite_defaults_fail_before_browser_startup() {
    for source in [
        "/** @default */ uniform float value;",
        "/** @default invalid */ uniform float value;",
        "/** @default 1e400 */ uniform float value;",
        "/** @default 1e300 */ uniform float value;",
        "/** @default 1.5 */ uniform int value;",
        "/** @default 0.5 */ uniform vec3 value;",
        "/** @default 0.25,0.75 */ uniform float value[2];",
        "/** @default #3a7fc1 */ uniform vec3 value;",
        "/** @color\n @default #invalid */ uniform vec3 value;",
        "/** @default 0.25\n @default 0.75 */ uniform float value;",
    ] {
        let error = request(source, json!({})).unwrap_err();
        assert_eq!(error.code, "SHADER_COMPILE_FAILED", "{source}");
        assert!(error.message.contains("@default"), "{error:?}");
    }
    let large = format!(
        "/** @default {} */ uniform float value;",
        vec!["0"; 4097].join(",")
    );
    assert!(
        request(&large, json!({}))
            .unwrap_err()
            .message
            .contains("4096")
    );
}

#[test]
fn grouped_declarations_keep_individual_array_dimensions_and_shared_precision() {
    let uniforms = shader::uniforms("uniform highp vec3 first, second[4], third[2+1];").unwrap();
    assert_eq!(
        uniforms.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        ["first", "second", "third"]
    );
    assert!(!uniforms[0].array);
    assert_eq!(uniforms[1].length, Some(4));
    assert!(uniforms[2].array);
    assert_eq!(uniforms[2].length, None);
    assert!(uniforms.iter().all(|u| u.kind == "vec3"));
}

#[test]
fn exact_directives_use_the_nearest_documentation_comment_before_a_declaration() {
    let uniforms = shader::uniforms(
        "/* Unicode © annotation @time_step; contact@time.example */ uniform float ordinary;\n\
        /** @time */ uniform float first,second;\n\
        /** @resolution */ uniform vec2 size;\n uniform float third;\n",
    )
    .unwrap();
    assert!(!uniforms[0].time);
    assert!(uniforms[1].time && uniforms[2].time);
    assert!(uniforms[3].resolution);
    assert!(!uniforms[4].time && !uniforms[4].resolution);
    assert!(shader::uniforms("/** @resolution */ /** @time */ uniform vec2 invalid;").is_err());
    assert!(shader::uniforms("/** @time */ uniform float clock[2];").is_err());
}

#[test]
fn comments_inside_declarations_and_macro_bodies_do_not_supply_metadata() {
    let source = "#define DECL uniform float fake;\\\n uniform float also_fake;\n\
        // uniform vec4 ignored;\n\
        uniform /** @time */ highp /* precision */ float clock;\n\
        const int COUNT = 3; uniform vec2 values[COUNT];";
    let uniforms = shader::uniforms(source).unwrap();
    assert_eq!(uniforms.len(), 2);
    assert!(!uniforms[0].time);
    assert_eq!(uniforms[1].name, "values");
    assert_eq!(uniforms[1].length, None);
}

#[test]
fn malformed_or_excessive_declarations_fail_instead_of_hanging_or_truncating() {
    for source in [
        "uniform",
        "uniform float",
        "uniform float ;",
        "uniform float a,;",
        "uniform float a[;",
        "uniform float a[];",
        "uniform float a[0];",
        "uniform float a[257];",
        "uniform vec2 a[2][2];",
        "uniform float duplicate,duplicate;",
        "/* unterminated",
        "uniform float a = 2.0;",
        "uniform float a[999999999999999999999999];",
    ] {
        assert!(
            shader::uniforms(source).is_err(),
            "Accepted malformed declaration: {source}"
        );
    }
    let source = (0..257)
        .map(|i| format!("uniform float u{i};"))
        .collect::<String>();
    assert!(shader::uniforms(&source).is_err());
    assert!(shader::uniforms(&";".repeat(1024 * 1024 + 1)).is_err());
}

#[test]
fn numeric_array_values_are_flat_typed_and_checked_against_literal_dimensions() {
    let source = "uniform vec3 colors[2]; uniform int flags[2]; uniform bool bits[2]; uniform mat2 transforms[2];";
    let value = request(
        source,
        json!({"colors":[1,0,0,0,1,0],"flags":[-1,2],"bits":[0,1],"transforms":[1,0,0,1,2,0,0,2]}),
    )
    .unwrap();
    assert_eq!(value["uniform_arrays"]["colors"], 2);
    assert_eq!(
        value["uniforms"]["colors"],
        json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    );
    assert_eq!(value["uniforms"]["flags"], json!([-1, 2]));
    assert_eq!(value["uniforms"]["bits"], json!([0, 1]));
    for invalid in [
        json!({"colors":[1,2]}),
        json!({"flags":[0.5,1]}),
        json!({"bits":[0,2]}),
        json!({"bits":[true,false]}),
        json!({"transforms":[1,0,0,1]}),
        json!({"colors":["#FF0000","#00FF00"]}),
    ] {
        assert!(
            request(source, invalid.clone()).is_err(),
            "Accepted {invalid}"
        );
    }
}

#[test]
fn expression_arrays_defer_size_to_the_linker_and_defaults_stay_unstored() {
    let value = request(
        "const int N=3; uniform float entries[N]; uniform vec2 other[2];",
        json!({"entries":[1,2,3]}),
    )
    .unwrap();
    assert!(value["uniform_arrays"]["entries"].is_null());
    assert_eq!(value["uniform_arrays"]["other"], 2);
    assert!(value["uniforms"]["other"].is_null());
    assert!(request("uniform vec2 entries[2+1];", json!({"entries":[1,2,3]})).is_err());
}

#[test]
fn integer_boolean_vectors_and_column_major_matrices_use_numeric_components() {
    let value = request(
        "uniform ivec3 counts; uniform bvec2 enabled; uniform mat3 matrix;",
        json!({"counts":[-1,2,3],"enabled":[1,0],"matrix":[1,0,0,0,2,0,0,0,1]}),
    )
    .unwrap();
    assert_eq!(value["uniforms"]["counts"], json!([-1, 2, 3]));
    assert_eq!(value["uniforms"]["enabled"], json!([1, 0]));
    assert_eq!(
        value["uniforms"]["matrix"],
        json!([1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0])
    );
    assert!(
        request(
            "uniform ivec2 counts;",
            json!({"counts":[2147483648_i64,0]})
        )
        .is_err()
    );
    assert!(request("uniform float huge;", json!({"huge":1e300})).is_err());
}

#[test]
fn builtins_still_reject_stored_values_and_unknown_or_unsupported_bindings_fail() {
    let source = "/** @resolution */ uniform vec2 size; /** @time */\n uniform float ordinary;";
    // Documentation after a semicolon describes the next declaration.
    let next = request(source, json!({})).unwrap();
    assert_eq!(next["uniforms"]["size"], json!([32, 16]));
    assert_eq!(next["uniforms"]["ordinary"], 0.25);
    let source = "/** @resolution */ uniform vec2 size;\n/** @time */ uniform float clock;";
    let value = request(source, json!({})).unwrap();
    assert_eq!(value["uniforms"]["size"], json!([32, 16]));
    assert_eq!(value["uniforms"]["clock"], 0.25);
    assert_eq!(value["time_dependent"], true);
    assert_eq!(
        request("/** @resolution */ uniform vec2 size;", json!({})).unwrap()["time_dependent"],
        false
    );
    assert_eq!(
        request("/** @time_step */ uniform float clock;", json!({})).unwrap()["time_dependent"],
        false
    );
    assert!(request(source, json!({"clock":1})).is_err());
    assert!(request("uniform float declared;", json!({"unknown":1})).is_err());
    assert_eq!(
        request("uniform sampler2D images[2];", json!({})).unwrap()["uniforms"]["images"],
        Value::Null
    );
}

#[test]
fn sampler_arrays_validate_lengths_types_defaults_and_reserved_stage_inputs() {
    for value in [
        json!([]),
        json!(true),
        json!([1, 2]),
        json!([["pencil:object"], ["pencil:background"]]),
        json!(["pencil:object"]),
    ] {
        assert!(request("uniform sampler2D images[2];", json!({"images":value})).is_err());
    }
    assert!(request("uniform sampler2D images[257];", json!({})).is_err());
    assert!(
        request(
            "uniform sampler2D images[2];",
            json!({"images":["pencil:object","pencil:background"]})
        )
        .is_err()
    );
    assert_eq!(
        request("const int N=2;uniform sampler2D images[N];", json!({})).unwrap()["uniform_arrays"]
            ["images"],
        Value::Null
    );
}

#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn software_webgl_sampler_arrays_bind_images_defaults_active_tails_and_recover() {
    use skia_safe::{Color, Data, Image};
    let root = TempDir::new().unwrap();
    let mut runtime = lazypencil::dynamic::DynamicRuntime::new(root.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
        );
    let striped = "precision highp float;\n/** @resolution */ uniform vec2 size;\nuniform sampler2D single,images[3];void main(){vec2 uv=gl_FragCoord.xy/size;gl_FragColor=uv.x<0.5?texture2D(images[0],uv):(uv.y>0.5?texture2D(images[1],uv):texture2D(single,uv));}";
    let constant = "precision highp float;const int N=2;uniform sampler2D images[N];void main(){gl_FragColor=texture2D(images[1],vec2(0.5));}";
    for (source, values, success) in [
        (
            striped,
            json!({"single":"blue.png","images":["stripe.png","green.png","stripe.png"]}),
            true,
        ),
        (constant, json!({"images":["blue.png"]}), false),
        (constant, json!({"images":["blue.png","green.png"]}), true),
        (constant, json!({}), true),
    ] {
        let scene_root = TempDir::new().unwrap();
        for (name, color) in [
            ("blue.png", Color::BLUE),
            ("green.png", Color::GREEN),
            ("stripe.png", Color::RED),
        ] {
            let mut surface = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
            surface.canvas().clear(color);
            if name == "stripe.png" {
                let mut paint = skia_safe::Paint::default();
                paint.set_color(Color::BLUE);
                surface
                    .canvas()
                    .draw_rect(skia_safe::Rect::from_xywh(0.0, 2.0, 4.0, 2.0), &paint);
            }
            fs::write(
                scene_root.path().join(name),
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
        let unset = values == json!({});
        let scene = project(&scene_root, source, values);
        let before = fs::read(scene_root.path().join("input.pen")).unwrap();
        let layout = lazypencil::layout::compute(
            scene.resolve().unwrap(),
            &mut lazypencil::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let result = lazypencil::render::png_with_runtime(
            &scene,
            &layout,
            &Default::default(),
            &mut runtime,
            None,
        );
        if !success {
            let error = result
                .err()
                .expect("Expected a linked sampler length error");
            assert_eq!(error.code, "SHADER_COMPILE_FAILED");
            assert!(
                error.message.contains("Sampler array is shorter"),
                "{error:?}"
            );
            continue;
        }
        let output = result.unwrap();
        let image = Image::from_encoded(Data::new_copy(&output.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let pixels = image.peek_pixels().unwrap();
        if source == striped {
            assert_eq!(pixels.get_color((4, 2)), Color::RED);
            assert_eq!(pixels.get_color((4, 13)), Color::BLUE);
            assert_eq!(pixels.get_color((28, 2)), Color::GREEN);
            assert_eq!(pixels.get_color((28, 13)), Color::BLUE);
        } else if unset {
            assert_eq!(pixels.get_color((16, 8)).a(), 0);
        } else {
            assert_eq!(pixels.get_color((16, 8)), Color::GREEN);
        }
        assert_eq!(
            fs::read(scene_root.path().join("input.pen")).unwrap(),
            before
        );
    }
}

#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn software_webgl_binds_grouped_arrays_integer_boolean_and_matrix_values() {
    let root = TempDir::new().unwrap();
    let mut runtime = lazypencil::dynamic::DynamicRuntime::new(root.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
        );
    let cases = [
        (
            "precision highp float; uniform float red,green; uniform float blue[3]; void main(){gl_FragColor=vec4(red,green,blue[2],1.0);}",
            json!({"red":0.25,"green":0.5,"blue":[0.1,0.2,0.75]}),
            [64, 128, 191],
        ),
        (
            "precision highp float; uniform ivec2 i2; uniform ivec3 i3; uniform ivec4 i4;\n\
          uniform bvec2 b2; uniform bvec3 b3; uniform bvec4 b4;\n\
          uniform mat2 m2; uniform mat3 m3; uniform mat4 m4;\n\
          uniform int iarr[2]; uniform bool barr[2];\n\
          uniform vec2 v2[2]; uniform vec3 v3[2]; uniform vec4 v4[2]; uniform float zeros[3];\n\
          void main(){float r=float(i2.x+i3.y+i4.z+iarr[1])/20.0;\n\
          float g=(m2*vec2(0.0,1.0)).x+(m3*vec3(0.0,0.0,1.0)).y+(m4*vec4(1.0,0.0,0.0,0.0)).z;\n\
          float b=(all(b2)&&b3.y&&b4.z&&barr[1]&&!barr[0])?(v2[0].x+v3[0].y+v4[0].z+zeros[2]):0.0;\n\
          gl_FragColor=vec4(r,g,b,1.0);}",
            json!({"i2":[1,0],"i3":[0,2,0],"i4":[0,0,3,0],"iarr":[0,4],"b2":[1,1],"b3":[0,1,0],"b4":[0,0,1,0],"barr":[0,1],
          "m2":[0,0.9,0.1,0],"m3":[0,0,0,0,0,0.8,0,0.2,0],"m4":[0,0,0.3,0,0,0,0,0,0.7,0,0,0,0,0,0,0],
          "v2":[0.1,0,0,0],"v3":[0,0.2,0,0,0,0],"v4":[0,0,0.4,1,0,0,0,0]}),
            [128, 153, 179],
        ),
    ];
    // The first two independent color calculations exercise the real renderer.
    for (source, values, expected) in cases {
        let scene_root = TempDir::new().unwrap();
        let scene = project(&scene_root, source, values);
        let before = fs::read(scene_root.path().join("input.pen")).unwrap();
        let layout = lazypencil::layout::compute(
            scene.resolve().unwrap(),
            &mut lazypencil::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let rendered = lazypencil::render::png_with_runtime(
            &scene,
            &layout,
            &Default::default(),
            &mut runtime,
            None,
        )
        .unwrap();
        assert_eq!(rendered.dynamic_artifacts.len(), 1);
        let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&rendered.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let color = image.peek_pixels().unwrap().get_color((16, 8));
        assert_eq!(color.a(), 255);
        for (actual, expected) in [color.r(), color.g(), color.b()].into_iter().zip(expected) {
            assert!(
                (actual as i32 - expected).abs() <= 1,
                "Unexpected shader pixel: {color:?}"
            );
        }
        assert_eq!(
            fs::read(scene_root.path().join("input.pen")).unwrap(),
            before
        );
    }
}

#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn linked_constant_dimensions_validate_inputs_and_recover_after_an_error() {
    let root = TempDir::new().unwrap();
    let mut runtime = lazypencil::dynamic::DynamicRuntime::new(root.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
        );
    let source = "precision highp float;\n#define COUNT 3\nuniform float data[COUNT]; uniform mat2 unused_tail[2]; void main(){gl_FragColor=vec4(data[2]+(unused_tail[0]*vec2(0.0,1.0)).x,0.0,0.0,1.0);}";
    for (values, success) in [
        (json!({"data":[0.25]}), false),
        (
            json!({"data":[0.0,0.0,0.5],"unused_tail":[0,0.9,0.25,0,1,0,0,1]}),
            true,
        ),
    ] {
        let scene_root = TempDir::new().unwrap();
        let scene = project(&scene_root, source, values);
        let before = fs::read(scene_root.path().join("input.pen")).unwrap();
        let layout = lazypencil::layout::compute(
            scene.resolve().unwrap(),
            &mut lazypencil::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let result = lazypencil::render::png_with_runtime(
            &scene,
            &layout,
            &Default::default(),
            &mut runtime,
            None,
        );
        if success {
            let rendered = result.unwrap();
            let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&rendered.png))
                .unwrap()
                .make_raster_image(None, None)
                .unwrap();
            let color = image.peek_pixels().unwrap().get_color((16, 8));
            assert!(
                (color.r() as i32 - 191).abs() <= 1,
                "Unexpected recovered shader pixel: {color:?}"
            );
            assert_eq!((color.g(), color.b(), color.a()), (0, 0, 255));
        } else {
            let error = result.err().expect("Expected a linked uniform error");
            assert_eq!(error.code, "SHADER_COMPILE_FAILED");
            assert!(
                error
                    .message
                    .contains("shorter than its active linked elements"),
                "{error:?}"
            );
        }
        assert_eq!(
            fs::read(scene_root.path().join("input.pen")).unwrap(),
            before
        );
    }
}

#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn native_annotation_binding_matches_public_pencil_reference_pixels() {
    use lazypencil::{document::hash, render};
    use skia_safe::{Data, Image};
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pencil-0.3.9/shader-annotation-binding");
    let source =
        include_str!("fixtures/pencil-0.3.9/shader-annotation-binding/annotation-binding.pen");
    let reference_bytes =
        include_bytes!("fixtures/pencil-0.3.9/shader-annotation-binding/annotation-binding.png");
    let reference = Image::from_encoded(Data::new_copy(reference_bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!((reference.width(), reference.height()), (320, 240));
    let expected = reference.peek_pixels().unwrap();
    for version in ["2.19", "2.21"] {
        let root = TempDir::new().unwrap();
        for entry in fs::read_dir(&fixtures).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "glsl")
            {
                fs::copy(&path, root.path().join(path.file_name().unwrap())).unwrap();
            }
        }
        let source = source.replace("\"2.19\"", &format!("\"{version}\""));
        let path = root.path().join("design.pen");
        fs::write(&path, &source).unwrap();
        let project =
            Project::from_document(&path, Document::parse(source.clone()).unwrap(), &[]).unwrap();
        let layout = lazypencil::layout::compute(
            project.resolve().unwrap(),
            &mut lazypencil::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let mut runtime = lazypencil::dynamic::DynamicRuntime::new(root.path().join("runtime"))
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_executable(
                std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
            );
        let output = render::png_with_runtime(
            &project,
            &layout,
            &render::RenderOptions::default(),
            &mut runtime,
            None,
        )
        .unwrap();
        assert_eq!((output.width, output.height), (160, 120));
        assert_eq!(output.dynamic_artifacts.len(), 20);
        let diagnostics = layout.summary()["diagnostics"].clone();
        assert!(diagnostics.as_array().unwrap().is_empty());
        let image = Image::from_encoded(Data::new_copy(&output.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let actual = image.peek_pixels().unwrap();
        let mut maximum = 0u8;
        let mut difference = 0u64;
        // These original panels are constant colors at both native export pixels
        // per logical pixel. The 1x render tests metadata independently of the
        // separately tracked scaled-coordinate compatibility work.
        for y in 0..120 {
            for x in 0..160 {
                let a = actual.get_color((x, y));
                let b = expected.get_color((x * 2, y * 2));
                for delta in [
                    a.r().abs_diff(b.r()),
                    a.g().abs_diff(b.g()),
                    a.b().abs_diff(b.b()),
                    a.a().abs_diff(b.a()),
                ] {
                    maximum = maximum.max(delta);
                    difference += u64::from(delta);
                }
            }
        }
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_NATIVE_SHADER_DIR") {
            let directory = std::path::PathBuf::from(directory);
            fs::create_dir_all(&directory).unwrap();
            let stem = format!("chromium-native-annotation-binding-{version}");
            fs::write(directory.join(format!("{stem}.png")), &output.png).unwrap();
            fs::write(
                directory.join(format!("{stem}.json")),
                json!({
                    "source_commit":std::env::var("GITHUB_SHA").ok(), "document_version":version,
                    "width":output.width,"height":output.height,"scale":1,"diagnostics":diagnostics,
                    "png_hash":hash(&output.png),"reference_sha256":hash(reference_bytes),
                    "source_sha256":hash(source.as_bytes()),"native_producer":"@pen.dev/cli 0.3.9",
                    "native_document_version":"2.19","native_export_scale":2,"verified_cases":20,
                    "maximum_channel_difference":maximum,"absolute_channel_difference":difference
                })
                .to_string(),
            )
            .unwrap();
        }
        assert!(
            maximum <= 1,
            "Native annotation binding diverged ({version}): maximum={maximum}, difference={difference}"
        );
    }
}

#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn native_documentation_defaults_match_public_pencil_reference_pixels() {
    use lazypencil::{document::hash, render};
    use skia_safe::{Data, Image};
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pencil-0.3.9/shader-defaults");
    let source = include_str!("fixtures/pencil-0.3.9/shader-defaults/default-values.pen");
    let reference_bytes =
        include_bytes!("fixtures/pencil-0.3.9/shader-defaults/default-values.png");
    let reference = Image::from_encoded(Data::new_copy(reference_bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let expected = reference.peek_pixels().unwrap();
    assert_eq!((reference.width(), reference.height()), (560, 40));
    for variant in ["literal", "constant"] {
        let root = TempDir::new().unwrap();
        for entry in fs::read_dir(&fixtures).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "glsl")
            {
                let mut bytes = fs::read_to_string(&path).unwrap();
                if variant == "constant" && path.file_name().unwrap() == "array-default.glsl" {
                    bytes = bytes
                        .replace("/** @default", "const int COUNT=2;\n/** @default")
                        .replace("value[2]", "value[COUNT]");
                }
                fs::write(root.path().join(path.file_name().unwrap()), bytes).unwrap();
            }
        }
        let path = root.path().join("design.pen");
        fs::write(&path, source).unwrap();
        let project =
            Project::from_document(&path, Document::parse(source.to_owned()).unwrap(), &[])
                .unwrap();
        let layout = lazypencil::layout::compute(
            project.resolve().unwrap(),
            &mut lazypencil::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let mut runtime = lazypencil::dynamic::DynamicRuntime::new(root.path().join("runtime"))
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_executable(
                std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
            );
        let output = render::png_with_runtime(
            &project,
            &layout,
            &render::RenderOptions {
                scale: 2.0,
                ..Default::default()
            },
            &mut runtime,
            None,
        )
        .unwrap();
        assert_eq!((output.width, output.height), (560, 40));
        let diagnostics = layout.summary()["diagnostics"].clone();
        assert!(diagnostics.as_array().unwrap().is_empty());
        assert_eq!(output.dynamic_artifacts.len(), 14);
        let image = Image::from_encoded(Data::new_copy(&output.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let actual = image.peek_pixels().unwrap();
        let mut maximum = 0u8;
        let mut difference = 0u64;
        for y in 0..40 {
            for x in 0..560 {
                let a = actual.get_color((x, y));
                let b = expected.get_color((x, y));
                for delta in [
                    a.r().abs_diff(b.r()),
                    a.g().abs_diff(b.g()),
                    a.b().abs_diff(b.b()),
                    a.a().abs_diff(b.a()),
                ] {
                    maximum = maximum.max(delta);
                    difference += u64::from(delta);
                }
            }
        }
        assert!(
            maximum <= 1,
            "Native shader defaults diverged ({variant}): maximum={maximum}, difference={difference}"
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_NATIVE_SHADER_DIR") {
            let directory = std::path::PathBuf::from(directory);
            fs::create_dir_all(&directory).unwrap();
            let stem = format!("chromium-native-defaults-{variant}");
            fs::write(directory.join(format!("{stem}.png")), &output.png).unwrap();
            fs::write(
                directory.join(format!("{stem}.json")),
                json!({
                    "source_commit":std::env::var("GITHUB_SHA").ok(),"variant":variant,
                    "width":output.width,"height":output.height,"diagnostics":diagnostics,
                    "png_hash":hash(&output.png),"reference_sha256":hash(reference_bytes),
                    "source_sha256":hash(source.as_bytes()),"native_producer":"@pen.dev/cli 0.3.9",
                    "document_version":"2.19","verified_cases":14,
                    "maximum_channel_difference":maximum,"absolute_channel_difference":difference
                })
                .to_string(),
            )
            .unwrap();
        }
    }
}
