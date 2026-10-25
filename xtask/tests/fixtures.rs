use lazypencil_xtask::{fixtures, performance, util};
use serde_json::{Value, json};
use std::fs;

#[test]
fn all_original_diagnostic_fonts_regenerate_byte_for_byte_in_rust() {
    let directory = util::root().join("tests/fixtures/fonts");
    let manifest: Value = util::json(&directory.join("manifest.json")).unwrap();
    let fonts = fixtures::fonts::generate();
    assert_eq!(fonts.len(), 7);
    for (name, bytes) in fonts {
        assert_eq!(manifest[name]["bytes"], bytes.len(), "{name}");
        assert_eq!(manifest[name]["sha256"], util::digest(&bytes), "{name}");
        assert_eq!(fs::read(directory.join(name)).unwrap(), bytes, "{name}");
    }
    let temporary = tempfile::tempdir().unwrap();
    let output = temporary.path().join("generated");
    fixtures::fonts::write(&output).unwrap();
    assert_eq!(
        fs::read(output.join("manifest.json")).unwrap(),
        fs::read(directory.join("manifest.json")).unwrap()
    );
}

#[test]
fn original_color_font_has_independent_sfnt_checksums_and_palette_tables() {
    let bytes = fixtures::fonts::color::generate();
    assert!(bytes.len() < 4096);
    assert_eq!(bytes.len() % 4, 0);
    let checksum = bytes.chunks_exact(4).fold(0u32, |sum, word| {
        sum.wrapping_add(u32::from_be_bytes(word.try_into().unwrap()))
    });
    assert_eq!(checksum, 0xb1b0afba);
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut tables = std::collections::BTreeMap::new();
    for row in bytes[12..12 + count * 16].chunks_exact(16) {
        let start = u32::from_be_bytes(row[8..12].try_into().unwrap()) as usize;
        let size = u32::from_be_bytes(row[12..16].try_into().unwrap()) as usize;
        assert_eq!(start % 4, 0);
        assert!(
            tables
                .insert(&row[..4], &bytes[start..start + size])
                .is_none()
        );
    }
    assert_eq!(tables[b"maxp".as_slice()][4..6], 10u16.to_be_bytes());
    assert_eq!(tables[b"hhea".as_slice()][34..36], 10u16.to_be_bytes());
    assert_eq!(tables[b"hmtx".as_slice()][14..16], 400u16.to_be_bytes());
    assert_eq!(
        tables[b"CPAL".as_slice()][14..],
        [0, 0, 255, 255, 0, 255, 0, 255]
    );
    assert_eq!(tables[b"COLR".as_slice()].len(), 84);
    assert_eq!(tables[b"GSUB".as_slice()].len(), 90);
    assert!(
        tables[b"name".as_slice()]
            .windows(8)
            .any(|w| w == b"\0L\0P\0 \0T")
    );
    assert_eq!(bytes, fixtures::fonts::color::generate());
}

#[test]
fn fixed_performance_inputs_keep_the_existing_source_and_asset_hashes() {
    let directory = util::root().join("tests/fixtures/performance");
    assert_eq!(
        util::file_digest(&directory.join("dashboard.pen")).unwrap(),
        "c55bcf8727afc04d31646ea1a259e17723fe9be03e3cb4fd2dccbf8512764da7"
    );
    assert_eq!(
        util::file_digest(&directory.join("avatar.png")).unwrap(),
        "adea05ca47573c3398f5e7d10aebcba7cd6da37abdeae104131e6085cd1e0937"
    );
    let shadow: Value = util::json(&directory.join("shadow.pen")).unwrap();
    assert_eq!(shadow["children"][0]["width"], 1920);
    assert_eq!(shadow["children"][0]["height"], 1080);
    assert_eq!(
        shadow["children"][0]["children"].as_array().unwrap().len(),
        999
    );
    let dashboard: Value = util::json(&directory.join("dashboard.pen")).unwrap();
    assert_eq!(dashboard["children"][0]["width"], 1920);
    assert_eq!(dashboard["children"][0]["height"], 1080);
    assert_eq!(
        dashboard["children"][0]["children"]
            .as_array()
            .unwrap()
            .len(),
        13
    );
}

#[test]
fn repeated_measurements_use_observed_nearest_rank_p95_and_keep_first_invocations_separate() {
    let samples = (1..=20)
        .map(|n| json!({"elapsed_ms":n,"peak_rss_bytes":n,"user_cpu_ms":n,"system_cpu_ms":n}))
        .collect::<Vec<_>>();
    assert_eq!(
        performance::summarize(&samples).unwrap()["elapsed_ms"],
        json!({"min":1.0,"median":10.5,"p95_nearest_rank":19.0,"max":20.0})
    );
    assert!(performance::summarize(&[]).is_err());
}

#[test]
fn performance_reports_bind_geometry_execution_scope_and_exact_output_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("output.png");
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend_from_slice(&1920u32.to_be_bytes());
    bytes.extend_from_slice(&1080u32.to_be_bytes());
    fs::write(&path, &bytes).unwrap();
    let report = json!({"width":1920,"height":1080,"node_count":1000,"diagnostics":[],"assets":[],"dynamic_artifacts":[],"script_executions":[],"png_hash":util::digest(&bytes)});
    assert!(performance::validate_render(&serde_json::to_vec(&report).unwrap(), &path, 0).is_ok());
    for (key, value) in [
        ("width", json!(100)),
        ("node_count", json!(999)),
        ("diagnostics", json!([{}])),
        ("dynamic_artifacts", json!([{}])),
        ("script_executions", json!([{}])),
        ("assets", json!([{}])),
        ("assets", Value::Null),
        ("png_hash", json!("forged")),
    ] {
        let mut invalid = report.clone();
        invalid[key] = value;
        assert!(
            performance::validate_render(&serde_json::to_vec(&invalid).unwrap(), &path, 0).is_err(),
            "{key}"
        );
    }
    fs::write(&path, b"not a png").unwrap();
    assert!(performance::validate_render(&serde_json::to_vec(&report).unwrap(), &path, 0).is_err());
}
