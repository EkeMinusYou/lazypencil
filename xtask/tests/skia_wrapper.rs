use lazypencil_xtask::{
    notices::{self, skia_wrapper},
    util,
};
use serde_json::Value;
use std::{fs, path::Path};

const STDOUT: &[u8] =
    b"COMPILING BINDINGS: [\"src/bindings.cpp\", \"src/shaper.cpp\", \"src/paragraph.cpp\"]\n";

fn fixture() -> (tempfile::TempDir, Vec<String>) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let directory = tempfile::TempDir::new().unwrap();
    let review: Value = util::json(&source.join(skia_wrapper::REVIEW)).unwrap();
    let mut paths = vec![
        skia_wrapper::REVIEW.to_string(),
        "vendor/skia-bindings/Cargo.toml".into(),
    ];
    paths.extend(
        review["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pin| format!("vendor/skia-bindings/{}", pin["path"].as_str().unwrap())),
    );
    for path in paths {
        let target = directory.path().join(&path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(source.join(path), target).unwrap();
    }
    let features = serde_json::from_value(review["required_features"].clone()).unwrap();
    (directory, features)
}

#[test]
fn reviewed_wrapper_bundle_preserves_original_mit_and_observed_source_selection() {
    let (root, features) = fixture();
    let (index, files) = skia_wrapper::build(root.path(), &features, STDOUT).unwrap();
    assert_eq!(index["compiled_sources"].as_array().unwrap().len(), 3);
    let original = fs::read(root.path().join("vendor/skia-bindings/LICENSE")).unwrap();
    assert_eq!(files["LICENSE.original"], original);
    assert_eq!(files["build-output.log"].as_slice(), STDOUT);
    assert!(files["WRAPPER-NOTICES.txt"].ends_with(&original));
    let output = root.path().join("notices");
    skia_wrapper::collect(root.path(), &features, STDOUT, &output).unwrap();
    notices::verify_expected(&output, &files).unwrap();
    fs::write(
        output.join("WRAPPER-NOTICES.txt"),
        b"MIT without the original copyright and permission text",
    )
    .unwrap();
    assert!(notices::verify_expected(&output, &files).is_err());
}

#[test]
fn changed_wrapper_inputs_cannot_silently_reuse_attribution_review() {
    let (root, features) = fixture();
    let review: Value = util::json(&root.path().join(skia_wrapper::REVIEW)).unwrap();
    for pin in review["files"].as_array().unwrap() {
        let path = root
            .path()
            .join("vendor/skia-bindings")
            .join(pin["path"].as_str().unwrap());
        let original = fs::read(&path).unwrap();
        let mut changed = original.clone();
        changed.extend_from_slice(b"\n// Changed upstream input\n");
        fs::write(&path, changed).unwrap();
        assert!(
            skia_wrapper::build(root.path(), &features, STDOUT).is_err(),
            "{}",
            path.display()
        );
        fs::write(&path, original).unwrap();
    }
}

#[test]
fn wrapper_review_requires_known_features_and_one_exact_compilation_record() {
    let (root, features) = fixture();
    let mut embedded = features.clone();
    embedded.push("embed-freetype".into());
    skia_wrapper::build(root.path(), &embedded, STDOUT).unwrap();
    for unknown in ["gl", "svg", "skottie", "pdf"] {
        let mut selected = features.clone();
        selected.push(unknown.into());
        assert!(skia_wrapper::build(root.path(), &selected, STDOUT).is_err());
    }
    let mut missing = features.clone();
    missing.retain(|feature| feature != "textlayout");
    assert!(skia_wrapper::build(root.path(), &missing, STDOUT).is_err());
    for output in [
        Vec::new(),
        [STDOUT, STDOUT].concat(),
        b"COMPILING BINDINGS: [\"src/gl.cpp\"]\n".to_vec(),
    ] {
        assert!(skia_wrapper::build(root.path(), &features, &output).is_err());
    }
    let policy = root.path().join(skia_wrapper::REVIEW);
    let mut review: Value = util::json(&policy).unwrap();
    review["files"].as_array_mut().unwrap().pop();
    fs::write(policy, util::encode(&review).unwrap()).unwrap();
    assert!(skia_wrapper::build(root.path(), &features, STDOUT).is_err());
}
