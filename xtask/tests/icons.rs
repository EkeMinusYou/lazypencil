use lazypencil_xtask::{icons, util};
use serde_json::{Value, json};
use std::fs;

#[test]
fn catalogue_parser_accepts_only_data_and_preserves_escaped_unicode_strings() {
    let text = r#"const n = [{name:"demo",pascal_name:"Demo",tags:["a\"b","日本語"],categories:[e.SYSTEM],figma_category:a.SYSTEM,codepoint:123,published_in:2.1,alias:{name:"old",pascal_name:"Old"}}];
export { n as icons };"#;
    let value = icons::catalogue_literal(text).unwrap();
    assert_eq!(value[0]["tags"], json!(["a\"b", "日本語"]));
    assert_eq!(value[0]["alias"], json!({"name":"old","pascal_name":"Old"}));
    assert_eq!(value[0]["codepoint"], 123);
    for expression in [
        "process.exit(1)",
        "(()=>[])()",
        "import(\"x\")",
        "e.SYSTEM + run()",
        "{name:\"x\",name:\"y\"}",
    ] {
        assert!(
            icons::catalogue_literal(&format!(
                "const n = [{expression}];\nexport {{ n as icons }};"
            ))
            .is_err(),
            "{expression}"
        );
    }
    for text in [
        "const other = [];",
        "const n = [];\nrun();",
        "const n = [];\nexport { n }; const n = [];",
    ] {
        assert!(icons::catalogue_literal(text).is_err());
    }
}

#[test]
fn pinned_phosphor_archive_reproduces_original_geometry_index_license_and_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let project = util::root();
    fs::copy(
        project.join("tests/fixtures/icons/phosphor-core-2.1.1.tgz"),
        temp.path().join("phosphor-core-2.1.1.tgz"),
    )
    .unwrap();
    let output = temp.path().join("output");
    let (descriptor, provenance) = icons::phosphor(temp.path(), &output).unwrap();
    let catalogue: Vec<Value> =
        util::json(&project.join("resources/icons/catalogue.json")).unwrap();
    let expected = catalogue.iter().find(|v| v["id"] == "phosphor").unwrap();
    assert_eq!(&descriptor, expected);
    for name in ["LICENSE", "index.json"] {
        assert_eq!(
            fs::read(output.join("phosphor").join(name)).unwrap(),
            fs::read(project.join("resources/icons/phosphor").join(name)).unwrap()
        );
    }
    assert_eq!(
        util::file_digest(&temp.path().join("phosphor.icons.json")).unwrap(),
        expected["data_sha256"]
    );
    assert_eq!(provenance["catalogue_member"], "package/dist/index.mjs");
    assert_eq!(descriptor["icon_count"], 1512);
    assert_eq!(descriptor["alias_count"], 1548);
}

#[test]
fn unpinned_source_is_rejected_before_any_derived_resource_is_written() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("phosphor-core-2.1.1.tgz"),
        b"not original archive",
    )
    .unwrap();
    let output = temp.path().join("output");
    assert!(icons::phosphor(temp.path(), &output).is_err());
    assert!(!output.exists());
}
