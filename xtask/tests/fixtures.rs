use lazypencil_xtask::{fixtures, performance, util};
use serde_json::{Value, json};
use std::fs;

#[test]
fn upstream_emoji_pins_reject_mutable_sources_path_substitution_and_unbounded_assets() {
    let original: Value =
        util::json(&util::root().join("tests/fixtures/fonts/emoji-upstream.json")).unwrap();
    let valid: fixtures::emoji::Pin = serde_json::from_value(original.clone()).unwrap();
    valid.validate().unwrap();
    for (field, value) in [
        ("/revision", json!("main")),
        ("/repository", json!("other/repository")),
        ("/license/path", json!("../../LICENSE")),
        ("/fonts/0/asset/path", json!("2D/fonts/Other.ttf")),
        ("/fonts/1/format", json!("CBDT/CBLC")),
        ("/fonts/0/asset/sha256", json!("0".repeat(63))),
        ("/fonts/1/asset/bytes", json!(16 * 1024 * 1024 + 1)),
        ("/license/bytes", json!(0)),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(field).unwrap() = value;
        let pin: fixtures::emoji::Pin = serde_json::from_value(changed).unwrap();
        assert!(pin.validate().is_err(), "{field}");
    }
    let mut unknown = original;
    unknown["fonts"][0]["asset"]["url"] = json!("https://example.com/font.ttf");
    assert!(serde_json::from_value::<fixtures::emoji::Pin>(unknown).is_err());
}

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
    let tables = font_tables(&bytes);
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

fn font_tables(bytes: &[u8]) -> std::collections::BTreeMap<&[u8], &[u8]> {
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
        assert!(start + size <= bytes.len());
        assert!(
            tables
                .insert(&row[..4], &bytes[start..start + size])
                .is_none()
        );
    }
    tables
}

#[test]
fn original_sbix_font_has_valid_strike_offsets_duplicate_records_and_exact_rgba_bitmaps() {
    use std::io::Read;
    for silhouette in [false, true] {
        let bytes = fixtures::fonts::color::bitmap::generate_with_silhouette(silhouette).unwrap();
        assert!(bytes.len() < 8192);
        assert_eq!(
            bytes,
            fixtures::fonts::color::bitmap::generate_with_silhouette(silhouette).unwrap()
        );
        let tables = font_tables(&bytes);
        assert!(!tables.contains_key(b"COLR".as_slice()));
        assert!(!tables.contains_key(b"CPAL".as_slice()));
        assert_eq!(tables[b"maxp".as_slice()][4..6], 10u16.to_be_bytes());
        let sbix = tables[b"sbix".as_slice()];
        assert_eq!(&sbix[..12], &[0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 12]);
        let strike = &sbix[12..];
        assert_eq!(&strike[..4], &[0, 100, 0, 72]);
        let offsets = strike[4..48]
            .chunks_exact(4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize)
            .collect::<Vec<_>>();
        assert_eq!(offsets[0], 48);
        assert_eq!(offsets[10], strike.len());
        for (glyph, pair) in offsets.windows(2).enumerate() {
            assert!(pair[0] <= pair[1]);
            let record = &strike[pair[0]..pair[1]];
            if [4, 6, 7].contains(&glyph) {
                assert_eq!(record, &[0, 0, 0, 0, b'd', b'u', b'p', b'e', 0, 1]);
            } else if [1, 8].contains(&glyph) {
                assert_eq!(&record[..8], &[0, 0, 0, 0, b'p', b'n', b'g', b' ']);
                let png = &record[8..];
                assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
                let mut compressed = Vec::new();
                let mut cursor = 8;
                let mut tags = Vec::new();
                while cursor < png.len() {
                    let size =
                        u32::from_be_bytes(png[cursor..cursor + 4].try_into().unwrap()) as usize;
                    let tag = &png[cursor + 4..cursor + 8];
                    let data = &png[cursor + 8..cursor + 8 + size];
                    let mut crc = flate2::Crc::new();
                    crc.update(tag);
                    crc.update(data);
                    assert_eq!(
                        crc.sum().to_be_bytes(),
                        png[cursor + 8 + size..cursor + 12 + size]
                    );
                    tags.push(tag);
                    if tag == b"IHDR" {
                        assert_eq!(data, &[0, 0, 0, 64, 0, 0, 0, 70, 8, 6, 0, 0, 0]);
                    } else if tag == b"IDAT" {
                        compressed.extend(data);
                    } else {
                        assert_eq!(tag, b"IEND");
                        assert!(data.is_empty());
                    }
                    cursor += size + 12;
                }
                assert_eq!(
                    tags,
                    [b"IHDR".as_slice(), b"IDAT".as_slice(), b"IEND".as_slice()]
                );
                let mut raw = Vec::new();
                flate2::read::ZlibDecoder::new(compressed.as_slice())
                    .read_to_end(&mut raw)
                    .unwrap();
                assert_eq!(raw.len(), 70 * (64 * 4 + 1));
                for (y, row) in raw.chunks_exact(64 * 4 + 1).enumerate() {
                    assert_eq!(row[0], 0);
                    for (x, pixel) in row[1..].chunks_exact(4).enumerate() {
                        let mut expected = match (glyph, x < 32) {
                            (1, true) | (8, false) => [255, 0, 0, 255],
                            _ => [0, 255, 0, 255],
                        };
                        if silhouette {
                            let ring = (4..60).contains(&x)
                                && (4..54).contains(&y)
                                && !((16..48).contains(&x) && (16..42).contains(&y));
                            let island = (24..40).contains(&x) && (60..66).contains(&y);
                            expected[3] = if ring {
                                255
                            } else if island {
                                64
                            } else {
                                0
                            };
                        }
                        assert_eq!(pixel, expected);
                    }
                }
            } else {
                assert!(record.is_empty(), "Unexpected bitmap for glyph {glyph}");
            }
        }
    }
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
