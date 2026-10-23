#![cfg(feature = "rendering")]

use lazypencil::{
    document::hash,
    text::{DEFAULT_FONT_BYTES, DEFAULT_FONT_FAMILY, Fonts, TextBlock, TextSpec},
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .join(name)
}
fn spec(content: &str, family: &str) -> TextSpec {
    TextSpec {
        content: content.into(),
        family: family.into(),
        size: 48.0,
        ..Default::default()
    }
}
fn paint(block: &TextBlock) -> Vec<u8> {
    let mut surface = skia_safe::surfaces::raster_n32_premul((500, 130)).unwrap();
    surface.canvas().clear(skia_safe::Color::WHITE);
    block.paragraph.paint(surface.canvas(), (10.0, 10.0));
    skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
        .unwrap()
        .as_bytes()
        .to_vec()
}

#[test]
fn original_font_fixtures_have_pinned_contents_and_load_every_collection_face() {
    let manifest: Value =
        serde_json::from_slice(&fs::read(fixture("manifest.json")).unwrap()).unwrap();
    for (name, information) in manifest.as_object().unwrap() {
        let bytes = fs::read(fixture(name)).unwrap();
        assert_eq!(hash(&bytes), information["sha256"]);
        assert_eq!(bytes.len() as u64, information["bytes"].as_u64().unwrap());
        Fonts::new(&[fixture(name)]).unwrap();
    }
    let fonts = Fonts::new(&[fixture("faces.ttc")]).unwrap();
    let faces = fonts
        .inventory()
        .into_iter()
        .filter(|font| font.source.ends_with("faces.ttc"))
        .collect::<Vec<_>>();
    assert_eq!(faces.len(), 4);
    assert_eq!(
        faces
            .iter()
            .map(|font| font.face_index)
            .collect::<std::collections::BTreeSet<_>>(),
        [0, 1, 2, 3].into_iter().collect()
    );
    let digest = hash(&fs::read(fixture("faces.ttc")).unwrap());
    assert!(
        faces
            .iter()
            .all(|font| font.hash == digest && font.postscript_name.is_some())
    );
}

#[test]
fn static_styles_in_a_collection_and_separate_files_use_the_real_selected_face() {
    let mut outputs = Vec::new();
    for files in [
        vec![fixture("faces.ttc")],
        vec![
            fixture("regular.ttf"),
            fixture("bold.ttf"),
            fixture("italic.ttf"),
        ],
    ] {
        let mut fonts = Fonts::new(&files).unwrap();
        let mut images = Vec::new();
        for (weight, italic, index, file) in [
            (400, false, 0, "regular.ttf"),
            (700, false, 1, "bold.ttf"),
            (400, true, 2, "italic.ttf"),
        ] {
            let mut request = spec("A", "lp test sans");
            request.weight = weight;
            request.italic = italic;
            let block = fonts.layout(&request, None).unwrap();
            assert_eq!(block.metrics.fonts.len(), 1);
            let used = &block.metrics.fonts[0];
            assert_eq!(used.family, "LP Test Sans");
            assert_eq!(used.weight, weight);
            assert_eq!(used.slant, if italic { "italic" } else { "upright" });
            assert!(!used.synthetic_bold && !used.synthetic_italic);
            let name = if files.len() == 1 { "faces.ttc" } else { file };
            assert_eq!(used.hash, hash(&fs::read(fixture(name)).unwrap()));
            assert_eq!(used.face_hash, hash(&fs::read(fixture(file)).unwrap()));
            assert_eq!(used.face_index, if files.len() == 1 { index } else { 0 });
            images.push(paint(&block));
        }
        assert_ne!(images[0], images[1]);
        assert_ne!(images[0], images[2]);
        outputs.push(images);
    }
    assert_eq!(outputs[0], outputs[1]);
    let mut fonts = Fonts::new(&[fixture("regular.ttf")]).unwrap();
    let regular = paint(&fonts.layout(&spec("A", "LP Test Sans"), None).unwrap());
    for (weight, italic) in [(700, false), (400, true)] {
        let mut request = spec("A", "LP Test Sans");
        request.weight = weight;
        request.italic = italic;
        let block = fonts.layout(&request, None).unwrap();
        let face = &block.metrics.fonts[0];
        assert_eq!(face.hash, hash(&fs::read(fixture("regular.ttf")).unwrap()));
        assert_eq!(face.weight, 400);
        assert_eq!(face.synthetic_bold, weight == 700);
        assert_eq!(face.synthetic_italic, italic);
        assert_ne!(paint(&block), regular);
    }
}

#[test]
fn cff_fonts_and_mixed_outline_collections_measure_and_paint_the_selected_faces() {
    for name in ["cff.otf", "cff.otc"] {
        let mut fonts = Fonts::new(&[fixture(name)]).unwrap();
        let block = fonts.layout(&spec("A", "LP Test CFF"), None).unwrap();
        assert!((block.metrics.width - 43.2).abs() < 0.1);
        assert_eq!(block.metrics.fonts[0].face_index, 0);
        assert!(block.ink_bounds.width > 30.0);
        let png = paint(&block);
        assert!(png.len() > 100);
    }
    let mut fonts = Fonts::new(&[fixture("cff.otc")]).unwrap();
    let block = fonts
        .layout(&spec("A\u{f0000}", "LP Test CFF"), None)
        .unwrap();
    assert_eq!(
        block
            .metrics
            .fonts
            .iter()
            .map(|font| font.face_index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
}

#[test]
fn fallback_chains_cover_mixed_scripts_and_report_actual_shaped_fonts() {
    let mut fonts = Fonts::new(&[fixture("faces.ttc")]).unwrap();
    let block = fonts
        .layout(&spec("A日\u{f0000}", "LP Test Sans"), None)
        .unwrap();
    assert_eq!(
        block
            .metrics
            .fonts
            .iter()
            .map(|font| font.family.as_str())
            .collect::<Vec<_>>(),
        vec!["LP Test Sans", DEFAULT_FONT_FAMILY, "LP Test Supplement"]
    );
    assert_eq!(block.metrics.fonts[1].hash, hash(DEFAULT_FONT_BYTES));
    assert_eq!(block.metrics.fonts[2].face_index, 3);
    let combined = fonts
        .layout(&spec("A\u{0301}", "LP Test Sans"), None)
        .unwrap();
    assert!(
        combined
            .metrics
            .fonts
            .iter()
            .any(|font| font.family == DEFAULT_FONT_FAMILY)
    );
    fonts.set_fallback_families(&[]).unwrap();
    let error = fonts
        .layout(&spec("A日\u{f0000}", "LP Test Sans"), None)
        .err()
        .unwrap();
    assert_eq!(error.code, "ASSET_NOT_FOUND");
    assert_eq!(
        error.details.as_ref().unwrap()["missing_character_count"],
        2
    );
    fonts
        .set_fallback_families(&[DEFAULT_FONT_FAMILY.into()])
        .unwrap();
    fonts.layout(&spec("A日", "LP Test Sans"), None).unwrap();
    assert!(
        fonts
            .layout(&spec("\u{f0000}", "LP Test Sans"), None)
            .is_err()
    );
    fonts
        .set_fallback_families(&["LP Test Supplement".into(), DEFAULT_FONT_FAMILY.into()])
        .unwrap();
    fonts
        .layout(&spec("A日\u{f0000}", "LP Test Sans"), None)
        .unwrap();
    let before = fonts.fallback_families().to_vec();
    assert!(
        fonts
            .set_fallback_families(&["LP Font Missing 8492".into()])
            .is_err()
    );
    assert_eq!(fonts.fallback_families(), before);
    assert_eq!(
        fonts
            .layout(&spec("A", "LP Font Missing 8492"), None)
            .err()
            .unwrap()
            .code,
        "ASSET_NOT_FOUND"
    );
}

#[test]
fn variable_axes_and_bounded_missing_glyph_diagnostics_come_from_shaping() {
    let mut fonts = Fonts::new(&[]).unwrap();
    let mut request = spec("日本語 A", DEFAULT_FONT_FAMILY);
    request.weight = 700;
    let block = fonts.layout(&request, None).unwrap();
    assert_eq!(block.metrics.fonts.len(), 1);
    let face = &block.metrics.fonts[0];
    let axis = face.axes.iter().find(|axis| axis.tag == "wght").unwrap();
    assert_eq!((axis.min, axis.max, axis.value), (100.0, 900.0, 700.0));
    assert!(!face.synthetic_bold);
    let content = (0xf1000..0xf1080)
        .map(|value| char::from_u32(value).unwrap())
        .collect::<String>();
    let error = fonts
        .layout(&spec(&content, DEFAULT_FONT_FAMILY), None)
        .err()
        .unwrap();
    let details = error.details.unwrap();
    assert_eq!(details["missing_character_count"], 128);
    assert_eq!(details["missing_characters"].as_array().unwrap().len(), 64);
    assert_eq!(details["missing_characters_truncated"], true);
    for family in [String::new(), "X".repeat(1024 * 1024)] {
        let error = fonts.layout(&spec("A", &family), None).err().unwrap();
        assert_eq!(error.code, "INVALID_ARGUMENT");
        assert!(error.message.len() < 256);
    }
}

#[test]
fn malformed_and_oversized_collection_headers_fail_without_a_partial_font_set() {
    let original = fs::read(fixture("faces.ttc")).unwrap();
    let mut inputs = vec![b"ttcf".to_vec(), original[..16].to_vec()];
    for (offset, number) in [
        (4, 0x00030000u32),
        (8, 0),
        (8, 257),
        (12, u32::MAX),
        (12, 1),
    ] {
        let mut bytes = original.clone();
        bytes[offset..offset + 4].copy_from_slice(&number.to_be_bytes());
        inputs.push(bytes);
    }
    let first = u32::from_be_bytes(original[12..16].try_into().unwrap()) as usize;
    for (offset, number) in [
        (first + 12 + 8, u32::MAX),
        (first + 12 + 12, u32::MAX),
        (first + 4, 0),
        (first + 4, 0xffff0000),
    ] {
        let mut bytes = original.clone();
        bytes[offset..offset + 4].copy_from_slice(&number.to_be_bytes());
        inputs.push(bytes);
    }
    let mut signature_only = original.clone();
    signature_only[first + 4..first + 6].copy_from_slice(&1u16.to_be_bytes());
    signature_only[first + 12..first + 16].copy_from_slice(b"DSIG");
    inputs.push(signature_only);
    for bytes in inputs {
        let result = Fonts::from_sources(&[(PathBuf::from("captured.ttc"), Arc::from(bytes))]);
        assert_eq!(result.err().unwrap().code, "INVALID_ARGUMENT");
    }
}

#[test]
fn captured_font_inputs_enforce_the_aggregate_byte_budget_before_loading() {
    let bytes = Arc::<[u8]>::from(DEFAULT_FONT_BYTES);
    let sources = (0..15)
        .map(|index| (PathBuf::from(format!("font-{index}.ttf")), bytes.clone()))
        .collect::<Vec<_>>();
    let error = Fonts::from_sources(&sources).err().unwrap();
    assert_eq!(error.code, "INVALID_ARGUMENT");
    assert!(error.message.contains("128 MiB"));
}

#[test]
#[ignore = "Requires an explicitly named installed system font family"]
fn installed_system_styles_use_captured_bytes_and_report_the_selected_faces() {
    let family =
        std::env::var("LAZYPENCIL_TEST_FONT_FAMILY").expect("Set LAZYPENCIL_TEST_FONT_FAMILY");
    let system = skia_safe::FontMgr::new();
    let mut fonts = Fonts::new(&[]).unwrap();
    let mut images = Vec::new();
    for (weight, italic, style) in [
        (400, false, skia_safe::FontStyle::normal()),
        (700, false, skia_safe::FontStyle::bold()),
        (400, true, skia_safe::FontStyle::italic()),
    ] {
        let face = system
            .match_family_style(&family, style)
            .expect("Installed font face");
        let (bytes, index) = face.to_font_bytes().unwrap();
        let mut request = spec("Hello World", &family);
        request.weight = weight;
        request.italic = italic;
        let block = fonts.layout(&request, None).unwrap();
        assert_eq!(block.metrics.fonts.len(), 1);
        let actual = &block.metrics.fonts[0];
        assert_eq!(actual.source, "system:font-data");
        assert_eq!(actual.hash, hash(&bytes));
        assert_eq!(actual.face_index, index);
        assert_eq!(actual.postscript_name, face.post_script_name());
        images.push(paint(&block));
    }
    assert_ne!(images[0], images[1]);
    assert_ne!(images[0], images[2]);
}

#[test]
fn font_cli_lists_collection_metadata_and_embedded_license_without_installation() {
    let temp = tempfile::TempDir::new().unwrap();
    let data = temp.path().join("unused");
    let list = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args(["--json", "fonts", "list", "--font"])
        .arg(fixture("faces.ttc"))
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stdout)
    );
    let value: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(value["fonts"].as_array().unwrap().len(), 5);
    assert_eq!(value["fallback_families"][0], DEFAULT_FONT_FAMILY);
    let license = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args(["--json", "fonts", "license"])
        .output()
        .unwrap();
    assert!(license.status.success());
    let value: Value = serde_json::from_slice(&license.stdout).unwrap();
    let text = value["text"].as_str().unwrap();
    assert!(text.contains("SIL OPEN FONT LICENSE"));
    assert_eq!(value["hash"], hash(text.as_bytes()));
    assert_eq!(value["license"], "OFL-1.1");
    assert!(!data.exists());
}

#[cfg(unix)]
#[test]
fn font_cli_rejects_fifo_sources_without_blocking() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("font.ttf");
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "fonts", "list", "--font"])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Font input blocked on a FIFO");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "INVALID_DOCUMENT");
}
