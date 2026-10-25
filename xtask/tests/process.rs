use lazypencil_xtask::{
    ci::{cache, commands, setup},
    download, performance, process, util,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Cursor, Read},
    process::Command,
    time::{Duration, Instant},
};
#[path = "../src/test_support/install.rs"]
mod install;
fn helper() -> &'static str {
    env!("CARGO_BIN_EXE_lazypencil-test-helper")
}
#[test]
fn focused_scheduling_preserves_reproduction_order_and_excludes_only_repeated_conditions() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/focus-commands.json")).unwrap();
    for case in cases {
        let targets: Vec<String> = serde_json::from_value(case["targets"].clone()).unwrap();
        let plan =
            commands::plan(&util::root(), case["suite"].as_str().unwrap(), &targets).unwrap();
        assert_eq!(json!(plan.build), case["build"]);
        assert_eq!(json!(plan.checks), case["checks"], "{}", case["suite"]);
    }
    for (suite, targets) in [
        ("ci", vec!["mcp".into()]),
        ("shader", vec!["mcp".into()]),
        ("core", vec!["agent_workflow".into()]),
        ("core", vec!["not-a-target".into()]),
    ] {
        assert!(
            commands::plan(&util::root(), suite, &targets).is_err(),
            "{suite}/{targets:?}"
        );
    }
}
#[test]
fn bounded_commands_preserve_exit_codes_output_and_failure_logs() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("responses.json");
    fs::write(&config,br#"{"[\"ok\"]":{"stdout":"out","stderr":"err"},"[\"bad\"]":{"stdout":"partial","stderr":"failed","status":7}}"#).unwrap();
    let make = |mode: &str| {
        let mut c = Command::new(helper());
        c.arg("fixture-command").arg(&config).arg(mode);
        c
    };
    let output = process::output(&mut make("ok"), Duration::from_secs(5)).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"out");
    assert_eq!(output.stderr, b"err");
    let output = process::output(&mut make("bad"), Duration::from_secs(5)).unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"partial");
    let log = temp.path().join("failure.log");
    assert!(process::logged(&mut make("bad"), Duration::from_secs(5), &log, false).is_err());
    assert_eq!(fs::read(log).unwrap(), b"partialfailed");
}
#[cfg(unix)]
#[test]
fn deadlines_kill_a_term_ignoring_descendant_without_waiting_on_inherited_logs() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("pids.json");
    let started = Instant::now();
    let result = process::output(
        Command::new(helper()).arg("hold-tree").arg(&marker),
        Duration::from_millis(800),
    )
    .unwrap();
    assert!(result.timed_out);
    assert!(started.elapsed() < Duration::from_secs(6));
    let pids: Value = util::json(&marker).unwrap();
    for name in ["parent", "child"] {
        let pid = pids[name].as_i64().unwrap() as i32;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if unsafe { libc::kill(pid, 0) } == -1 {
                break;
            }
            #[cfg(target_os = "linux")]
            if fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
                s.split(')')
                    .nth(1)
                    .is_some_and(|s| s.trim_start().starts_with('Z'))
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Owned descendant {pid} survived deadline"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
#[test]
fn resource_measurements_use_fresh_native_children_and_enforce_failure_and_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("pixels.png");
    let (usage, raw) = performance::measure(
        Command::new(helper())
            .arg("render-fixture")
            .args(["--json", "render", "shadow.pen", "--output"])
            .arg(&out),
        5.0,
    )
    .unwrap();
    assert!(usage["peak_rss_bytes"].as_u64().unwrap() > 0);
    assert!(usage["user_cpu_ms"].as_f64().unwrap() >= 0.0);
    assert!(usage["system_cpu_ms"].as_f64().unwrap() >= 0.0);
    performance::validate_render(&raw, &out, 0).unwrap();
    assert!(performance::measure(Command::new(helper()).arg("unknown"), 5.0).is_err());
    let started = Instant::now();
    assert!(performance::measure(Command::new(helper()).arg("sleep"), 0.1).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}
#[test]
fn repeated_benchmark_observes_separate_first_invocations_and_exact_repeated_samples() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("renderer");
    install::install(&util::root(), &binary, &["render-fixture".into()]).unwrap();
    let report = performance::benchmark(&binary, 2, 5.0).unwrap();
    assert_eq!(report["binary_sha256"], util::file_digest(&binary).unwrap());
    for scene in report["scenes"].as_array().unwrap() {
        assert_eq!(scene["repeated"].as_array().unwrap().len(), 2);
        assert!(
            scene["first_invocation"]["peak_rss_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(scene["expanded_nodes"], 1000);
    }
}
#[test]
fn streamed_originals_enforce_exact_limit_hash_no_overwrite_and_transactional_cleanup() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("broken source"))
        }
    }
    for mutation in ["hash", "size", "read", "exact", "overwrite"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("original");
        let bytes = b"original pinned font";
        let sha = util::digest(bytes);
        let result = match mutation {
            "read" => download::publish(Broken, &sha, &path, bytes.len() as u64),
            "size" => download::publish(Cursor::new(bytes), &sha, &path, bytes.len() as u64 - 1),
            "hash" => download::publish(
                Cursor::new(bytes),
                &"0".repeat(64),
                &path,
                bytes.len() as u64,
            ),
            "overwrite" => {
                fs::write(&path, b"keep").unwrap();
                download::publish(Cursor::new(bytes), &sha, &path, bytes.len() as u64)
            }
            _ => download::publish(Cursor::new(bytes), &sha, &path, bytes.len() as u64),
        };
        assert_eq!(result.is_ok(), mutation == "exact", "{mutation}");
        if mutation == "overwrite" {
            assert_eq!(fs::read(path).unwrap(), b"keep");
        } else if mutation == "exact" {
            assert_eq!(
                download::verify_file(&path, &sha, bytes.len() as u64).unwrap(),
                bytes.len() as u64
            );
        } else {
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        }
    }
    let original = "https://raw.githubusercontent.com/google/pinned"
        .parse()
        .unwrap();
    for url in [
        "https://example.invalid/pinned",
        "http://raw.githubusercontent.com/google/pinned",
    ] {
        assert!(download::validate_redirect(&original, &url.parse().unwrap()).is_err());
    }
    download::validate_redirect(
        &original,
        &"https://raw.githubusercontent.com/google/same-host"
            .parse()
            .unwrap(),
    )
    .unwrap();
}
#[test]
fn cached_material_sources_revalidate_immutable_pins_without_network() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("fonts");
    fs::create_dir(&output).unwrap();
    let catalogue = temp.path().join("catalogue.json");
    let mut records = Vec::new();
    for style in ["Outlined", "Rounded", "Sharp"] {
        let name = format!("MaterialSymbols{style}[FILL,GRAD,opsz,wght].ttf");
        let bytes = format!("pinned-{style}");
        fs::write(output.join(&name), &bytes).unwrap();
        records.push(json!({"id":format!("Material Symbols {style}"),"version":"a".repeat(40),"source_url":format!("https://raw.githubusercontent.com/google/material-design-icons/{}/variablefont/MaterialSymbols{style}%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf","a".repeat(40)),"source_format":"ttf","source_sha256":util::digest(bytes)}));
    }
    fs::write(&catalogue, util::encode(&records).unwrap()).unwrap();
    let result = setup::material_fonts(&catalogue, &output).unwrap();
    assert!(
        result["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["reused"] == true)
    );
    for field in ["version", "source_url", "source_sha256", "source_format"] {
        let mut changed = records.clone();
        changed[0][field] = "invalid".into();
        fs::write(&catalogue, util::encode(&changed).unwrap()).unwrap();
        assert!(
            setup::material_fonts(&catalogue, &output).is_err(),
            "{field}"
        );
    }
    fs::write(&catalogue, util::encode(&records).unwrap()).unwrap();
    fs::write(
        output.join("MaterialSymbolsOutlined[FILL,GRAD,opsz,wght].ttf"),
        b"tampered",
    )
    .unwrap();
    assert!(setup::material_fonts(&catalogue, &output).is_err());
}
#[test]
fn retention_preserves_unrelated_download_dependency_tool_and_pull_request_caches() {
    let entries = vec![
        json!({"id":1,"key":"cargo-downloads-Linux","ref":"refs/heads/main"}),
        json!({"id":2,"key":"native-v2-ubuntu-24.04-image-hash","ref":"refs/heads/main"}),
        json!({"id":3,"key":"xtask-v1-tools","ref":"refs/heads/main"}),
        json!({"id":4,"key":"application-v2-ubuntu-24.04-hash","ref":"refs/pull/1/merge"}),
    ];
    assert!(cache::obsolete(&entries, None).unwrap().is_empty());
}
