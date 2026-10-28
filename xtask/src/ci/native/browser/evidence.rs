//! Required browser artifacts, independent of runtime lane scheduling.

use super::*;
use crate::performance;

pub fn required_images(root: &Path, commit: &str) -> Result<()> {
    let report = |name: &str| -> Result<Value> {
        let v: Value = util::json(&root.join(name))
            .with_context(|| format!("Browser evidence report: {name}"))?;
        ensure!(v["source_commit"] == commit, "Wrong source: {name}");
        Ok(v)
    };
    let png = |name: &str, digest: &Value, w: u32, h: u32| -> Result<Vec<u8>> {
        ensure!(
            Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name)
                && name.starts_with("codec-"),
            "Invalid PNG path"
        );
        let bytes = performance::png(&root.join(name), &json!({"png_hash":digest}), w, h)?;
        ensure!(
            bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82"),
            "Truncated PNG: {name}"
        );
        Ok(bytes)
    };
    let dimensions = |row: &Value| -> Result<(u32, u32)> {
        let width = u32::try_from(row["width"].as_u64().context("Missing image width")?)?;
        let height = u32::try_from(row["height"].as_u64().context("Missing image height")?)?;
        ensure!(
            width > 0 && height > 0 && width <= 16384 && height <= 16384,
            "Invalid image dimensions"
        );
        Ok((width, height))
    };
    let structures = report("codec-shader-structures.json")?;
    let structures = structures["cases"]
        .as_array()
        .context("Missing shader structures")?;
    ensure!(structures.len() == 16, "Expected 16 shader structures");
    let expanded = report("codec-shader-preprocessed.json")?;
    let expanded = expanded["cases"]
        .as_array()
        .context("Missing preprocessed shaders")?;
    ensure!(expanded.len() == 64, "Expected 64 preprocessed shaders");
    for version in ["2.19", "2.21"] {
        for case in 0..8 {
            let rows: Vec<_> = structures
                .iter()
                .filter(|r| r["document_version"] == version && r["case"] == case)
                .collect();
            ensure!(rows.len() == 1, "Missing or duplicate shader structure");
            png(
                &format!("codec-shader-structure-{version}-{case}.png"),
                &rows[0]["png_hash"],
                32,
                16,
            )?;
            for effect in [false, true] {
                for scale in [1, 2] {
                    let rows: Vec<_> = expanded
                        .iter()
                        .filter(|r| {
                            r["document_version"] == version
                                && r["case"] == case
                                && r["effect"] == effect
                                && r["scale"] == scale
                        })
                        .collect();
                    ensure!(rows.len() == 1, "Missing or duplicate preprocessed shader");
                    let name =
                        format!("codec-shader-preprocessed-{version}-{case}-{effect}-{scale}.png");
                    ensure!(rows[0]["png"] == name, "Wrong preprocessed PNG name");
                    png(&name, &rows[0]["png_hash"], 32 * scale, 16 * scale)?;
                }
            }
        }
        let prefix = format!("codec-native-layout-cancellation-{version}");
        let r = report(&format!("{prefix}.json"))?;
        ensure!(
            r["document_version"] == version
                && r["worker_process_id"].is_null()
                && r["before_snapshot_id"] == r["recovered_snapshot_id"],
            "Native cancellation did not recover"
        );
        ensure!(
            png(
                &format!("{prefix}-before.png"),
                &r["before_png_hash"],
                320,
                80
            )? == png(
                &format!("{prefix}-recovered.png"),
                &r["recovered_png_hash"],
                320,
                80
            )?,
            "Recovered cancellation pixels differ"
        );
    }
    let sampler = report("codec-sampler-copy.json")?;
    let cases = sampler["cases"]
        .as_array()
        .context("Missing sampler copy images")?;
    ensure!(cases.len() == 4, "Expected four sampler copy conditions");
    for absolute in [false, true] {
        for theme in ["light", "dark"] {
            let rows: Vec<_> = cases
                .iter()
                .filter(|row| row["absolute"] == absolute && row["theme"] == theme)
                .collect();
            ensure!(
                rows.len() == 1,
                "Missing or duplicate sampler copy condition"
            );
            let row = rows[0];
            let (width, height) = dimensions(row)?;
            let before = format!("codec-sampler-copy-{absolute}-{theme}-before.png");
            let after = format!("codec-sampler-copy-{absolute}-{theme}-after.png");
            ensure!(
                row["before"] == before && row["after"] == after,
                "Wrong sampler image names"
            );
            ensure!(
                png(&before, &row["png_hash"], width, height)?
                    == png(&after, &row["png_hash"], width, height)?,
                "Copied sampler pixels differ"
            );
        }
    }
    let projection = report("codec-sampler-projection.json")?;
    let cases = projection["cases"]
        .as_array()
        .context("Missing sampler projection images")?;
    ensure!(
        cases.len() == 12,
        "Expected twelve sampler projection conditions"
    );
    for version in ["2.19", "2.21"] {
        for case in ["aliases", "file_directory", "case"] {
            for theme in ["light", "dark"] {
                let rows: Vec<_> = cases
                    .iter()
                    .filter(|row| {
                        row["document_version"] == version
                            && row["case"] == case
                            && row["theme"] == theme
                    })
                    .collect();
                ensure!(
                    rows.len() == 1,
                    "Missing or duplicate sampler projection condition"
                );
                let row = rows[0];
                let (width, height) = dimensions(row)?;
                let prefix = format!("codec-sampler-projection-{version}-{case}-{theme}");
                let before = format!("{prefix}-before.png");
                ensure!(
                    row["before"] == before,
                    "Wrong sampler projection image name"
                );
                let original = png(&before, &row["png_hash"], width, height)?;
                for stage in ["copied", "recopied"] {
                    let name = format!("{prefix}-{stage}.png");
                    ensure!(row[stage] == name, "Wrong sampler projection image name");
                    ensure!(
                        original == png(&name, &row["png_hash"], width, height)?,
                        "Sampler projection pixels differ after {stage}"
                    );
                }
            }
        }
    }
    let overrides = report("codec-script-overrides.json")?;
    let cases = overrides["cases"]
        .as_array()
        .context("Missing script override images")?;
    ensure!(
        cases.len() == 12,
        "Expected twelve script override conditions"
    );
    for version in ["2.19", "2.21"] {
        for mode in [
            "inputs", "code", "metadata", "combined", "literal", "nullable",
        ] {
            let rows: Vec<_> = cases
                .iter()
                .filter(|row| row["document_version"] == version && row["mode"] == mode)
                .collect();
            ensure!(
                rows.len() == 1,
                "Missing or duplicate script override condition"
            );
            let row = rows[0];
            let (width, height) = dimensions(row)?;
            ensure!(
                (width, height) == (36, 12),
                "Wrong script override image geometry"
            );
            let prefix = format!("codec-script-overrides-{version}-{mode}");
            let before = format!("{prefix}-before.png");
            ensure!(row["before"] == before, "Wrong script override image name");
            let original = png(&before, &row["png_hash"], width, height)?;
            for stage in ["copied", "recopied"] {
                let name = format!("{prefix}-{stage}.png");
                ensure!(row[stage] == name, "Wrong script override image name");
                ensure!(
                    original == png(&name, &row["png_hash"], width, height)?,
                    "Script override pixels differ after {stage}"
                );
            }
            for stage in ["explicit", "reset", "inherited_input", "inherited_code"] {
                let name = format!("{prefix}-{stage}.png");
                ensure!(row[stage] == name, "Wrong script input image name");
                png(&name, &row[format!("{stage}_png_hash")], width, height)?;
            }
        }
    }
    let local = report("codec-browser-local-files.json")?;
    ensure!(
        local["source_removed"] == true
            && local["managed_bundle_reloaded_after_source_removal"] == true
            && local["project_resource_revision"]
                .as_str()
                .is_some_and(|revision| revision.len() == 64
                    && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
            && local["location_preserved"] == true
            && local["unsafe_host_file"] == false
            && local["sandbox"] == "enabled"
            && local["stats"]["fulfilled"] == 8
            && local["stats"]["denied"] == 1
            && local["stats"]["attached_targets"]
                .as_u64()
                .is_some_and(|count| count >= 2),
        "Local browser resource isolation proof missing"
    );
    png("codec-browser-local-files.png", &local["png_hash"], 32, 32)?;
    let extensionless = &local["extensionless"];
    ensure!(
        extensionless["content_type"] == "text/plain"
            && extensionless["executed"] == false
            && extensionless["equivalent"] == true,
        "Missing native extensionless-file comparison"
    );
    let native = png(
        "codec-browser-local-files-native-plain.png",
        &extensionless["png_hash"],
        64,
        64,
    )?;
    ensure!(
        native
            == png(
                "codec-browser-local-files-captured-plain.png",
                &extensionless["png_hash"],
                64,
                64
            )?,
        "Extensionless file pixels differ from native loading"
    );
    let copied = report("codec-browser-copy.json")?;
    ensure!(
        copied["sandbox"] == "enabled",
        "Browser copy sandbox proof missing"
    );
    let rows = copied["cases"]
        .as_array()
        .context("Missing browser copy cases")?;
    ensure!(
        rows.len() == 32,
        "Expected both schemas, eight browser contexts and both themes"
    );
    for version in ["2.19", "2.21"] {
        for mode in [
            "inherited",
            "literal",
            "query",
            "metadata",
            "combined",
            "descendant-url",
            "descendant-metadata",
            "replacement",
        ] {
            for theme in ["light", "dark"] {
                let matching = rows
                    .iter()
                    .filter(|row| {
                        row["document_version"] == version
                            && row["mode"] == mode
                            && row["theme"] == theme
                    })
                    .collect::<Vec<_>>();
                ensure!(
                    matching.len() == 1,
                    "Missing or duplicate browser copy condition"
                );
                let row = matching[0];
                let width = match (
                    matches!(
                        mode,
                        "query" | "combined" | "descendant-url" | "replacement"
                    ),
                    theme == "dark",
                ) {
                    (false, false) => 32,
                    (false, true) => 40,
                    (true, false) => 48,
                    (true, true) => 56,
                };
                ensure!(
                    row["sources_removed"] == true && dimensions(row)? == (width, 16),
                    "Incomplete browser copy geometry/removal proof"
                );
                let stages = row["stages"]
                    .as_array()
                    .context("Missing browser copy stages")?;
                ensure!(
                    stages.len() == 4,
                    "Expected before, copied, recopied and detached browser images"
                );
                let mut original = None;
                let mut resource_revision = None;
                for stage in ["before", "copied", "recopied", "detached"] {
                    let matching = stages
                        .iter()
                        .filter(|row| row["stage"] == stage)
                        .collect::<Vec<_>>();
                    ensure!(
                        matching.len() == 1,
                        "Missing or duplicate browser copy stage"
                    );
                    let entry = matching[0];
                    let name = format!("codec-browser-copy-{version}-{mode}-{theme}-{stage}.png");
                    ensure!(entry["name"] == name, "Wrong browser copy image name");
                    let bytes = png(&name, &entry["png_hash"], width, 16)?;
                    let revision = entry["resource_revision"]
                        .as_str()
                        .context("Missing browser namespace revision")?;
                    ensure!(
                        revision.len() == 64
                            && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
                        "Invalid browser namespace revision"
                    );
                    if let Some(original) = &original {
                        ensure!(*original == bytes, "Browser pixels changed after {stage}");
                    } else {
                        original = Some(bytes);
                    }
                    if let Some(original) = resource_revision {
                        ensure!(
                            original == revision,
                            "Browser namespace changed after {stage}"
                        );
                    } else {
                        resource_revision = Some(revision);
                    }
                }
            }
        }
    }
    let generated = report("codec-generated-browser.json")?;
    ensure!(
        generated["sandbox"] == "enabled",
        "Generated browser sandbox proof missing"
    );
    let rows = generated["cases"]
        .as_array()
        .context("Missing generated browser cases")?;
    ensure!(
        rows.len() == 54,
        "Expected both schemas and all 27 generated browser contexts"
    );
    for version in ["2.19", "2.21"] {
        for style in [
            "direct",
            "reference",
            "descendant",
            "replacement",
            "children",
            "inherited",
            "nested",
            "scope-direct",
            "scope-reference",
            "scope-descendant",
            "scope-replacement",
            "scope-children",
            "scope-metadata",
            "scope-descendant-metadata",
            "scope-envelope",
            "scope-descendant-envelope",
            "scope-envelope-metadata",
            "scope-descendant-envelope-metadata",
            "scope-empty",
            "scope-qualified",
            "scope-nested",
            "scope-stale",
            "scope-stale-metadata",
            "scope-stale-descendant-metadata",
            "scope-stale-envelope-metadata",
            "scope-stale-descendant-envelope-metadata",
            "scope-default-shift-metadata",
        ] {
            let matching = rows
                .iter()
                .filter(|row| row["document_version"] == version && row["style"] == style)
                .collect::<Vec<_>>();
            ensure!(
                matching.len() == 1,
                "Missing or duplicated generated browser case"
            );
            let row = matching[0];
            let expected = match style {
                "inherited"
                | "scope-direct"
                | "scope-reference"
                | "scope-descendant"
                | "scope-replacement"
                | "scope-children"
                | "scope-envelope"
                | "scope-descendant-envelope"
                | "scope-nested"
                | "scope-stale-metadata"
                | "scope-stale-descendant-metadata"
                | "scope-stale-envelope-metadata"
                | "scope-stale-descendant-envelope-metadata"
                | "scope-default-shift-metadata" => json!([255, 0, 0, 255]),
                "scope-qualified" => json!([0, 0, 255, 255]),
                _ => json!([0, 255, 0, 255]),
            };
            ensure!(
                row["rgba"] == expected && row["sources_removed"] == true,
                "Generated browser source isolation proof missing"
            );
            let stages = row["stages"]
                .as_array()
                .context("Missing generated browser stages")?;
            ensure!(
                stages.len() == 3,
                "Expected original, copy and recopy generated pages"
            );
            let mut original = None;
            let mut revision = None;
            for stage in ["before", "copied", "recopied"] {
                let matching = stages
                    .iter()
                    .filter(|row| row["stage"] == stage)
                    .collect::<Vec<_>>();
                ensure!(
                    matching.len() == 1,
                    "Missing or duplicated generated browser stage"
                );
                let row = matching[0];
                let name = format!("codec-generated-browser-{version}-{style}-{stage}.png");
                ensure!(
                    row["name"] == name,
                    "Wrong generated browser evidence filename"
                );
                let bytes = png(&name, &row["png_hash"], 32, 16)?;
                let current = row["resource_revision"]
                    .as_str()
                    .context("Missing generated namespace revision")?;
                ensure!(current.len() == 64, "Invalid generated namespace revision");
                if let Some(original) = &original {
                    ensure!(
                        *original == bytes,
                        "Generated browser pixels changed after {stage}"
                    );
                } else {
                    original = Some(bytes);
                }
                if let Some(revision) = revision {
                    ensure!(
                        revision == current,
                        "Generated browser resource ownership changed after {stage}"
                    );
                } else {
                    revision = Some(current);
                }
            }
        }
    }
    let local = report("codec-local-pages.json")?;
    ensure!(
        local["sandbox"] == "enabled",
        "Local page sandbox proof missing"
    );
    let cases = local["cases"]
        .as_array()
        .context("Missing local page cases")?;
    let images = local["images"]
        .as_array()
        .context("Missing local page images")?;
    ensure!(
        cases.len() == 2 && images.len() == 14,
        "Expected both schemas and seven local page stages"
    );
    for version in ["2.19", "2.21"] {
        let cases = cases
            .iter()
            .filter(|row| row["document_version"] == version)
            .collect::<Vec<_>>();
        ensure!(cases.len() == 1, "Missing or duplicate local page schema");
        for proof in [
            "raster_cache_reused",
            "offscreen_resource_change_retired",
            "offscreen_denied_retired",
            "cancelled_process_reaped",
            "same_browser_after_offscreen_retirement",
        ] {
            ensure!(cases[0][proof] == true, "Missing local page proof: {proof}");
        }
        let mut states = std::collections::BTreeMap::new();
        let mut revisions = std::collections::BTreeMap::new();
        for (stage, width, rgba) in [
            ("first", 32, json!([0, 255, 0, 255])),
            ("changed", 32, json!([0, 0, 255, 255])),
            ("retained", 32, json!([0, 0, 255, 255])),
            ("resized", 40, json!([255, 255, 0, 255])),
            ("returned", 40, json!([255, 0, 0, 255])),
            ("restored", 40, json!([255, 0, 0, 255])),
            ("recovered", 40, json!([255, 0, 0, 255])),
        ] {
            let matching = images
                .iter()
                .filter(|row| row["document_version"] == version && row["stage"] == stage)
                .collect::<Vec<_>>();
            ensure!(matching.len() == 1, "Missing or duplicate local page stage");
            let row = matching[0];
            let name = format!("codec-local-page-{version}-{stage}.png");
            ensure!(
                row["name"] == name && dimensions(row)? == (width, 32) && row["rgba"] == rgba,
                "Wrong local page geometry/color"
            );
            png(&name, &row["png_hash"], width, 32)?;
            let acquisition = &row["acquisition"];
            let local_files = &acquisition["local_files"];
            ensure!(
                local_files["fulfilled"] == 5
                    && local_files["denied"] == 1
                    && local_files["attached_targets"]
                        .as_u64()
                        .is_some_and(|n| n >= 2),
                "Missing local page worker/denial proof"
            );
            let revision = acquisition["resources"]["revision"]
                .as_str()
                .context("Missing captured resource revision")?;
            ensure!(
                revision.len() == 64 && revision.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid captured revision"
            );
            revisions.insert(stage, revision);
            states.insert(stage, &acquisition["page_state"]);
        }
        ensure!(
            revisions["first"] != revisions["changed"]
                && revisions["changed"] == revisions["retained"]
                && revisions["retained"] == revisions["resized"]
                && revisions["returned"] != revisions["resized"]
                && revisions["returned"] == revisions["restored"]
                && revisions["restored"] == revisions["recovered"],
            "Stale local page resource revision"
        );
        for stage in ["retained", "resized", "returned", "restored", "recovered"] {
            ensure!(
                states[stage]["retained"] == true && states[stage]["id"].is_string(),
                "Missing retained page identity"
            );
        }
        ensure!(
            states["resized"]["reused"] == true
                && states["resized"]["id"] == states["retained"]["id"]
                && states["returned"]["id"] != states["retained"]["id"]
                && states["restored"]["id"] != states["returned"]["id"]
                && states["recovered"]["id"] != states["restored"]["id"],
            "Stale local page context"
        );
    }
    println!("Verified source identities, hashes and dimensions for 399 required PNGs");
    Ok(())
}

pub fn local_page_mcp_images(root: &Path, commit: &str) -> Result<()> {
    let report: Value = util::json(&root.join("codec-local-pages-mcp.json"))?;
    ensure!(
        report["source_commit"] == commit,
        "Wrong source for local page MCP images"
    );
    let images = report["images"]
        .as_array()
        .context("Missing local page MCP images")?;
    ensure!(
        images.len() == 8,
        "Expected both schemas and four MCP/CLI stages"
    );
    for version in ["2.19", "2.21"] {
        let mut changed = None;
        let mut revisions = std::collections::BTreeMap::new();
        for stage in ["first", "changed", "packed", "cli"] {
            let matching = images
                .iter()
                .filter(|row| row["document_version"] == version && row["stage"] == stage)
                .collect::<Vec<_>>();
            ensure!(
                matching.len() == 1,
                "Missing or duplicate local page MCP stage"
            );
            let row = matching[0];
            let name = format!("codec-local-page-{version}-mcp-{stage}.png");
            ensure!(
                row["name"] == name
                    && row["width"] == 32
                    && row["height"] == 32
                    && row["source_removed"] == matches!(stage, "packed" | "cli"),
                "Wrong local page MCP geometry/removal proof"
            );
            let bytes = performance::png(&root.join(&name), row, 32, 32)?;
            let artifacts = row["dynamic_artifacts"]
                .as_array()
                .context("Missing local page MCP acquisition")?;
            ensure!(
                artifacts.len() == 1
                    && artifacts[0]["acquisition"]["local_files"]["fulfilled"] == 2
                    && artifacts[0]["acquisition"]["local_files"]["denied"] == 0,
                "Incomplete local page MCP response proof"
            );
            let revision = artifacts[0]["acquisition"]["resources"]["revision"]
                .as_str()
                .context("Missing local page MCP revision")?;
            ensure!(
                revision.len() == 64 && revision.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid local page MCP revision"
            );
            revisions.insert(stage, revision);
            if stage == "changed" {
                changed = Some(bytes);
            } else if matches!(stage, "packed" | "cli") {
                ensure!(
                    changed.as_ref() == Some(&bytes),
                    "Packed MCP/CLI local page pixels changed"
                );
            }
        }
        ensure!(
            revisions["first"] != revisions["changed"]
                && revisions["changed"] == revisions["packed"]
                && revisions["packed"] == revisions["cli"],
            "Stale local page MCP namespace"
        );
    }
    println!("Verified source identities, hashes and dimensions for eight local page MCP/CLI PNGs");
    Ok(())
}
