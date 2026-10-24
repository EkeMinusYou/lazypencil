#![cfg(feature = "rendering")]

use lazypencil::{
    document::Document,
    dynamic::DynamicRuntime,
    layout,
    project::Project,
    render::{self, RenderOptions},
    script::{self, ScriptRuntime},
    service,
    store::Store,
    text::{Fonts, TextSpec},
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn scene(version: &str) -> (TempDir, Store) {
    let root = TempDir::new().unwrap();
    let path = root.path().join("scene.pen");
    let source = json!({"version":version,"children":[{"id":"page","type":"frame","width":320,"height":"fit_content","layout":"vertical","gap":8,"fill":"#ffffff","children":[
        {"id":"title","type":"text","content":"日本語 Native cancellation","fontFamily":"Noto Sans JP","fontSize":18,"stroke":"#102030","strokeWidth":0.25,"underline":true},
        {"id":"box","type":"rectangle","width":100,"height":20,"fill":"#336699"}
    ]}]}).to_string();
    fs::write(&path, source).unwrap();
    let store = Store::open(path, Some(root.path().join("data"))).unwrap();
    (root, store)
}

#[test]
fn native_cancellation_static_runtime_computation_stops_and_recovers_without_a_worker() {
    for version in ["2.19", "2.21"] {
        let (root, store) = scene(version);
        let original = fs::read(store.path()).unwrap();
        let mut project = Project::from_document(
            store.path(),
            Document::parse(String::from_utf8(original.clone()).unwrap()).unwrap(),
            &[],
        )
        .unwrap();
        let mut fonts = Fonts::new(&[]).unwrap();
        let baseline = layout::compute(project.resolve().unwrap(), &mut fonts).unwrap();
        let options = RenderOptions {
            region: Some(lazypencil::geometry::Rect::new(0., 0., 320., 80.)),
            ..Default::default()
        };
        let baseline_image = render::png(&project, &baseline, &options).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut runtime = ScriptRuntime::new(root.path().join("must-not-launch"));
        runtime.set_cancellation(Some(cancel));
        let error = script::compute(&mut project, &mut fonts, &mut runtime)
            .err()
            .expect("A cancelled static calculation must stop without a JS evaluation");
        assert_eq!(error.code, "CANCELLED");
        assert!(runtime.process_id().is_none());
        runtime.set_cancellation(None);
        let recovered = script::compute(&mut project, &mut fonts, &mut runtime).unwrap();
        assert_eq!(recovered.id, baseline.id);
        let recovered_image = render::png(&project, &recovered, &options).unwrap();
        assert_eq!(recovered_image.png, baseline_image.png);
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
            fs::create_dir_all(&directory).unwrap();
            let directory = std::path::Path::new(&directory);
            let baseline_file = format!("codec-native-layout-cancellation-{version}-before.png");
            let recovered_file =
                format!("codec-native-layout-cancellation-{version}-recovered.png");
            fs::write(directory.join(&baseline_file), &baseline_image.png).unwrap();
            fs::write(directory.join(&recovered_file), &recovered_image.png).unwrap();
            fs::write(directory.join(format!("codec-native-layout-cancellation-{version}.json")),json!({
            "source_commit":std::env::var("GITHUB_SHA").ok(),"document_version":version,
            "width":baseline_image.width,"height":baseline_image.height,
            "before_png":baseline_file,"recovered_png":recovered_file,
            "before_png_hash":lazypencil::document::hash(&baseline_image.png),
            "recovered_png_hash":lazypencil::document::hash(&recovered_image.png),
            "before_snapshot_id":baseline.id,"recovered_snapshot_id":recovered.id,
            "worker_process_id":runtime.process_id(),
            "scope":"Native public-API cancellation/recovery with unchanged layout identity, PNG and document; no JS worker. Not a native Pencil oracle or a physical Ctrl+C/latency guarantee."
        }).to_string()).unwrap();
        }
        assert!(runtime.process_id().is_none());
        assert!(runtime.idle_deadline().is_none());
        assert_eq!(fs::read(store.path()).unwrap(), original);
        assert!(store.history(10, None).unwrap().is_empty());
    }
}

#[test]
fn native_cancellation_public_native_apis_precede_validation_and_preserve_reuse() {
    let (_root, store) = scene("2.21");
    let project = Project::load(&store).unwrap();
    let mut fonts = Fonts::new(&[]).unwrap();
    let baseline = layout::compute(project.resolve().unwrap(), &mut fonts).unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        layout::compute_with_cancellation(project.resolve().unwrap(), &mut fonts, Some(&cancel))
            .err()
            .unwrap()
            .code,
        "CANCELLED"
    );
    let invalid = TextSpec {
        size: 0.,
        content: "Text".into(),
        ..Default::default()
    };
    assert_eq!(
        fonts.layout(&invalid, None).err().unwrap().code,
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        fonts
            .layout_with_cancellation(&invalid, None, Some(&cancel))
            .err()
            .unwrap()
            .code,
        "CANCELLED"
    );
    let mut dynamic = DynamicRuntime::new(store.data_dir());
    let invalid = RenderOptions {
        scale: f32::NAN,
        ..Default::default()
    };
    assert_eq!(
        render::png_with_runtime(&project, &baseline, &invalid, &mut dynamic, Some(cancel))
            .err()
            .unwrap()
            .code,
        "CANCELLED"
    );
    assert!(dynamic.process_ids().is_empty());
    let recovered = render::png_with_runtime(
        &project,
        &baseline,
        &RenderOptions::default(),
        &mut dynamic,
        None,
    )
    .unwrap();
    assert_eq!(
        recovered.png,
        render::png(&project, &baseline, &RenderOptions::default())
            .unwrap()
            .png
    );
    assert!(dynamic.process_ids().is_empty());
    assert!(dynamic.idle_deadline().is_none());
}

#[test]
fn native_cancellation_service_snapshots_release_request_tokens_and_keep_documents_unchanged() {
    let (root, store) = scene("2.21");
    let original = fs::read(store.path()).unwrap();
    let mut runtime = ScriptRuntime::new(root.path().join("must-not-launch"));
    let cancel = CancellationToken::new();
    runtime.set_cancellation(Some(cancel.clone()));
    let (project, snapshot) =
        service::layout_snapshot_with_runtime(&store, &[], None, None, &mut runtime).unwrap();
    cancel.cancel();
    project.verify_unchanged().unwrap();
    assert_eq!(
        layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap()
            .id,
        snapshot.id
    );
    assert_eq!(
        service::layout_snapshot_with_runtime(&store, &[], None, None, &mut runtime)
            .err()
            .unwrap()
            .code,
        "CANCELLED"
    );
    runtime.set_cancellation(Some(CancellationToken::new()));
    let (_, recovered) =
        service::layout_snapshot_with_runtime(&store, &[], None, None, &mut runtime).unwrap();
    assert_eq!(recovered.id, snapshot.id);
    assert!(runtime.process_id().is_none());
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
}
