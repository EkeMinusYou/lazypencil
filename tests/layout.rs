#![cfg(feature = "rendering")]

use lazypencil::{
    document::Document,
    layout::{self, LayoutSnapshot},
    project::Project,
    text::Fonts,
};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

fn compute(children: Value) -> lazypencil::Result<LayoutSnapshot> {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("test.pen");
    let source = json!({"version":"2.21","children":children}).to_string();
    fs::write(&path, &source).unwrap();
    let scene = Project::from_document(&path, Document::parse(source).unwrap(), &[])?.resolve()?;
    layout::compute(scene, &mut Fonts::new(&[]).unwrap())
}

fn rect(snapshot: &LayoutSnapshot, id: &str) -> lazypencil::geometry::Rect {
    snapshot
        .items
        .iter()
        .find(|i| i.record.id == id)
        .unwrap()
        .record
        .rect
}

#[test]
fn horizontal_fill_allocation_uses_padding_gaps_and_fixed_siblings() {
    let scene = compute(
        json!([{"id":"parent","type":"frame","width":320,"height":80,
        "padding":[10,16],"gap":8,"alignItems":"center","children":[
            {"id":"fixed","type":"rectangle","x":999,"width":40,"height":20},
            {"id":"a","type":"rectangle","width":"fill_container","height":"fill_container"},
            {"id":"b","type":"rectangle","width":"fill_container","height":40}
        ]}]),
    )
    .unwrap();
    assert_eq!(
        rect(&scene, "fixed"),
        lazypencil::geometry::Rect::new(16.0, 30.0, 40.0, 20.0)
    );
    assert_eq!(
        rect(&scene, "a"),
        lazypencil::geometry::Rect::new(64.0, 10.0, 116.0, 60.0)
    );
    assert_eq!(
        rect(&scene, "b"),
        lazypencil::geometry::Rect::new(188.0, 20.0, 116.0, 40.0)
    );
}

#[test]
fn vertical_content_sizes_exclude_absolute_and_disabled_children() {
    let scene=compute(json!([{"id":"parent","type":"frame","layout":"vertical","padding":[3,7,11,13],"gap":5,
        "children":[
            {"id":"a","type":"rectangle","width":80,"height":20},
            {"id":"b","type":"rectangle","width":40,"height":30},
            {"id":"absolute","type":"rectangle","layoutPosition":"absolute","x":200,"y":100,"width":900,"height":900},
            {"id":"disabled","type":"rectangle","enabled":false,"width":900,"height":900}
        ]}])).unwrap();
    assert_eq!(rect(&scene, "parent").width, 100.0);
    assert_eq!(rect(&scene, "parent").height, 69.0);
    assert_eq!(rect(&scene, "b").y, 28.0);
    assert_eq!(rect(&scene, "absolute").x, 200.0);
    assert!(
        scene
            .items
            .iter()
            .find(|i| i.record.id == "absolute")
            .unwrap()
            .record
            .diagnostics
            .iter()
            .any(|d| d.code == "LAYOUT_OVERFLOW")
    );
}

#[test]
fn wrapped_japanese_text_drives_the_content_height_of_its_parent() {
    let scene=compute(json!([{"id":"parent","type":"frame","layout":"vertical","width":180,"padding":12,"gap":8,"children":[
        {"id":"text","type":"text","textGrowth":"fixed-width","width":"fill_container","fontSize":18,"lineHeight":1.5,
         "content":"日本語と English が混在する文章を、同じ段落で計測して折り返します。"},
        {"id":"button","type":"rectangle","width":100,"height":44}
    ]}])).unwrap();
    assert_eq!(rect(&scene, "text").width, 156.0);
    let item = scene.items.iter().find(|i| i.record.id == "text").unwrap();
    assert!(item.record.text.as_ref().unwrap().line_count >= 3);
    assert_eq!(
        rect(&scene, "parent").height,
        24.0 + rect(&scene, "text").height + 8.0 + 44.0
    );
    let block = &scene.text_blocks[&1];
    assert_eq!(
        block.metrics.height,
        item.record.text.as_ref().unwrap().height
    );
    assert_eq!(block.paragraph.height(), rect(&scene, "text").height);
}

#[test]
fn content_fill_cycles_need_an_explicit_fallback() {
    let children = json!([{"id":"parent","type":"frame","children":[
        {"id":"child","type":"rectangle","width":"fill_container","height":20}
    ]}]);
    assert_eq!(
        compute(children.clone()).err().unwrap().code,
        "LAYOUT_UNRESOLVED"
    );
    let mut fallback = children;
    fallback[0]["children"][0]["width"] = json!("fill_container(120)");
    let scene = compute(fallback).unwrap();
    assert_eq!(rect(&scene, "parent").width, 120.0);
    assert_eq!(rect(&scene, "child").width, 120.0);
}

#[test]
fn absolute_fill_uses_its_fallback_and_auto_text_ignores_explicit_sizes() {
    let scene=compute(json!([{"id":"parent","type":"frame","layout":"none","width":300,"height":200,"children":[
        {"id":"absolute","type":"rectangle","width":"fill_container(44)","height":"fill_container(22)"},
        {"id":"text","type":"text","width":1,"height":1,"content":"Hello"}
    ]}])).unwrap();
    assert_eq!(rect(&scene, "absolute").width, 44.0);
    assert_eq!(rect(&scene, "absolute").height, 22.0);
    assert!(rect(&scene, "text").width > 1.0);
    assert!(rect(&scene, "text").height > 1.0);
}

#[test]
fn alignment_does_not_shrink_children_and_reports_text_overflow() {
    let scene=compute(json!([{"id":"parent","type":"frame","width":100,"height":40,"justifyContent":"end","children":[
        {"id":"text","type":"text","textGrowth":"fixed-width-height","width":180,"height":8,"fontSize":20,"content":"Overflow"}
    ]}])).unwrap();
    assert_eq!(rect(&scene, "text").x, -80.0);
    assert_eq!(rect(&scene, "text").width, 180.0);
    let diagnostics = &scene.items[1].record.diagnostics;
    assert!(diagnostics.iter().any(|d| d.code == "TEXT_OVERFLOW"));
    assert!(diagnostics.iter().any(|d| d.code == "LAYOUT_OVERFLOW"));
}

#[test]
fn rotations_and_flips_use_the_same_document_transform() {
    let scene=compute(json!([{"id":"parent","type":"frame","layout":"none","x":100,"y":200,"width":100,"height":100,
        "rotation":90,"children":[{"id":"child","type":"rectangle","x":10,"y":20,"width":30,"height":40,"flipX":true}]}])).unwrap();
    let parent = &scene.items[0].record;
    assert_eq!(parent.rect.x, 100.0);
    let point = scene.items[1].record.transform.point(0.0, 0.0);
    assert!((point.0 - 120.0).abs() < 0.001);
    assert!((point.1 - 160.0).abs() < 0.001);
    let opposite = scene.items[1].record.transform.point(30.0, 40.0);
    assert!((opposite.0 - 160.0).abs() < 0.001);
    assert!((opposite.1 - 190.0).abs() < 0.001);
}

#[test]
fn layout_records_include_font_hashes_and_weight_changes_in_the_snapshot() {
    let children = json!([{"id":"text","type":"text","content":"Snapshot","fontSize":20}]);
    let first = compute(children.clone()).unwrap();
    let mut changed = children;
    changed[0]["fontWeight"] = json!("700");
    assert_ne!(first.id, compute(changed).unwrap().id);
    assert!(
        !first.items[0].record.text.as_ref().unwrap().fonts[0]
            .hash
            .is_empty()
    );
}

#[test]
fn layout_include_stroke_reserves_the_inner_part_of_the_border() {
    let scene = compute(
        json!([{"id":"frame","type":"frame","width":100,"height":40,"layoutIncludeStroke":true,
            "stroke":"#000","strokeWidth":4,"strokeAlignment":"inner","children":[
            {"id":"child","type":"rectangle","width":"fill_container","height":"fill_container"}
        ]}]),
    )
    .unwrap();
    assert_eq!(
        rect(&scene, "child"),
        lazypencil::geometry::Rect::new(4.0, 4.0, 92.0, 32.0)
    );
}
