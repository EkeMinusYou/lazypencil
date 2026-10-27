#[cfg(unix)]
use lazypencil_xtask::performance::resources::{self, Process};
use std::process::Command;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
#[test]
fn active_resource_trace_keeps_raw_owned_counters_and_excludes_latency_claims() {
    let root = std::process::id();
    assert!(resources::ActiveTrace::new(0).is_err());
    let mut trace = resources::ActiveTrace::new(root).unwrap();
    trace.observe_if_due().unwrap();
    let report = trace.finish().unwrap();
    let points = report["points"].as_array().unwrap();
    assert!(points.len() >= 2);
    assert_eq!(report["root_pid"], root);
    assert_eq!(report["included_in_latency_distributions"], false);
    assert_eq!(report["transient_kernel_peak_established"], false);
    let mut peak = 0;
    let mut previous = 0.0;
    let mut maximum_gap: f64 = 0.0;
    for (index, point) in points.iter().enumerate() {
        let time = point["elapsed_ms"].as_f64().unwrap();
        assert!(time.is_finite() && time >= previous);
        assert!(point["observation_ms"].as_f64().unwrap() >= 0.0);
        if index > 0 {
            maximum_gap = maximum_gap.max(time - previous);
        }
        previous = time;
        let processes = point["snapshot"]["processes"].as_object().unwrap();
        assert_eq!(processes[&root.to_string()]["pid"], root);
        let rss = processes
            .values()
            .map(|process| process["rss_bytes"].as_u64().unwrap())
            .sum::<u64>();
        peak = peak.max(rss);
    }
    assert_eq!(report["maximum_observed_process_tree_rss_bytes"], peak);
    assert_eq!(report["actual_maximum_sampling_gap_ms"], maximum_gap);
}

#[test]
fn direct_pixel_profiles_keep_legacy_reports_readable_and_require_the_new_phase_set() {
    use lazypencil_xtask::performance::tui::preview_profiles;
    use serde_json::json;
    let report = json!({"format_version":3,"generation":1,"worker_total_ms":22.0,"phases_ms":{
        "snapshot_ms":1.0,"render_raster_ms":2.0,"render_pixel_copy_ms":3.0,
        "verification_ms":4.0,"terminal_encode_ms":5.0,"metadata_ms":6.0
    }});
    let encode = |report: &serde_json::Value| format!("LAZYPENCIL_PREVIEW_TIMING {report}\n");
    assert_eq!(
        preview_profiles(encode(&report).as_bytes())
            .unwrap()
            .as_slice(),
        std::slice::from_ref(&report)
    );
    for version in [1, 2, 4] {
        let mut invalid = report.clone();
        invalid["format_version"] = json!(version);
        assert!(preview_profiles(encode(&invalid).as_bytes()).is_err());
    }
    for value in [json!(-1.0), json!("3"), json!(null)] {
        let mut invalid = report.clone();
        invalid["phases_ms"]["render_pixel_copy_ms"] = value;
        assert!(preview_profiles(encode(&invalid).as_bytes()).is_err());
    }
    for obsolete in ["render_png_encode_ms", "decode_png_ms"] {
        let mut invalid = report.clone();
        invalid["phases_ms"][obsolete] = json!(0.0);
        assert!(preview_profiles(encode(&invalid).as_bytes()).is_err());
    }
    let mut invalid = report;
    invalid["worker_total_ms"] = json!(20.0);
    assert!(preview_profiles(encode(&invalid).as_bytes()).is_err());
}

#[test]
fn split_raster_and_png_profiles_preserve_strict_phase_and_interval_validation() {
    use lazypencil_xtask::performance::tui::preview_profiles;
    use serde_json::json;
    let report = json!({"format_version":2,"generation":1,"worker_total_ms":22.0,"phases_ms":{
        "snapshot_ms":1.0,"render_raster_ms":1.5,"render_png_encode_ms":0.5,
        "verification_ms":3.0,"decode_png_ms":4.0,"terminal_encode_ms":5.0,"metadata_ms":6.0
    }});
    let encode = |report: &serde_json::Value| format!("LAZYPENCIL_PREVIEW_TIMING {report}\n");
    assert_eq!(
        preview_profiles(encode(&report).as_bytes())
            .unwrap()
            .as_slice(),
        std::slice::from_ref(&report)
    );
    for (field, value) in [
        ("format_version", json!(1)),
        ("format_version", json!(3)),
        ("worker_total_ms", json!(20.0)),
    ] {
        let mut invalid = report.clone();
        invalid[field] = value;
        assert!(preview_profiles(encode(&invalid).as_bytes()).is_err());
    }
    for phase in ["render_raster_ms", "render_png_encode_ms"] {
        let mut invalid = report.clone();
        invalid["phases_ms"][phase] = json!(-1.0);
        assert!(preview_profiles(encode(&invalid).as_bytes()).is_err());
        invalid["phases_ms"].as_object_mut().unwrap().remove(phase);
        invalid["phases_ms"]["render_png_ms"] = json!(0.0);
        assert!(preview_profiles(encode(&invalid).as_bytes()).is_err());
    }
}

#[test]
fn worker_profiles_require_complete_bounded_monotonic_intervals_without_extra_data() {
    use lazypencil_xtask::performance::tui::preview_profiles;
    use serde_json::{Value, json};
    let report = json!({"format_version":1,"generation":1,"worker_total_ms":22.0,"phases_ms":{
        "snapshot_ms":1.0,"render_png_ms":2.0,"verification_ms":3.0,
        "decode_png_ms":4.0,"terminal_encode_ms":5.0,"metadata_ms":6.0
    }});
    let encode = |report: &Value| format!("LAZYPENCIL_PREVIEW_TIMING {report}\n");
    let mut next = report.clone();
    next["generation"] = json!(3);
    let input = format!("unrelated stderr\n{}{}", encode(&report), encode(&next));
    assert_eq!(
        preview_profiles(input.as_bytes()).unwrap(),
        [report.clone(), next]
    );
    for invalid in [
        json!({}),
        {
            let mut v = report.clone();
            v["phases_ms"]
                .as_object_mut()
                .unwrap()
                .remove("decode_png_ms");
            v
        },
        {
            let mut v = report.clone();
            v["phases_ms"]["metadata_ms"] = json!(-1.0);
            v
        },
        {
            let mut v = report.clone();
            v["worker_total_ms"] = json!(20.0);
            v
        },
        {
            let mut v = report.clone();
            v["generation"] = json!(-1);
            v
        },
        {
            let mut v = report.clone();
            v["document"] = json!("must not enter the profile");
            v
        },
        {
            let mut v = report.clone();
            v["phases_ms"]["unexpected"] = json!(1.0);
            v
        },
    ] {
        assert!(
            preview_profiles(encode(&invalid).as_bytes()).is_err(),
            "{invalid}"
        );
    }
    assert!(
        preview_profiles(format!("{}{}", encode(&report), encode(&report)).as_bytes()).is_err()
    );
    assert!(preview_profiles(b"unrelated stderr\n").is_err());
    assert!(preview_profiles(b"LAZYPENCIL_PREVIEW_TIMING {broken}\n").is_err());
    let oversized = vec![b'x'; 64 * 1024 + 1];
    assert!(preview_profiles(&oversized).is_err());
    let mut excessive = String::new();
    for generation in 0..129 {
        let mut report = report.clone();
        report["generation"] = json!(generation);
        excessive.push_str(&encode(&report));
    }
    assert!(preview_profiles(excessive.as_bytes()).is_err());
}

#[test]
#[cfg(unix)]
fn kernel_counters_preserve_process_names_and_convert_ticks_and_pages() {
    let mut fields = ["0"; 22];
    fields[0] = "S";
    fields[1] = "9";
    fields[2] = "123";
    fields[11] = "25";
    fields[12] = "7";
    fields[19] = "8765";
    fields[21] = "13";
    let text = format!("123 (name ) with\nparentheses)) {}", fields.join(" "));
    let process = resources::linux_stat(&text, 4096, 100).unwrap();
    assert_eq!(process.pid, 123);
    assert_eq!(process.parent, 9);
    assert_eq!(process.group, 123);
    assert_eq!(process.start_ticks, Some(8765));
    assert_eq!(process.rss_bytes, 13 * 4096);
    assert_eq!(process.cpu_ms, 320.0);
    for invalid in ["", "123 name 0", "123 (name) S 9"] {
        assert!(resources::linux_stat(invalid, 4096, 100).is_err());
    }
    assert!(resources::linux_stat(&text, 0, 100).is_err());
    assert!(resources::linux_stat(&text, 4096, 0).is_err());
    fields[21] = "18446744073709551615";
    assert!(resources::linux_stat(&format!("123 (name) {}", fields.join(" ")), 4096, 100).is_err());
}

#[test]
#[cfg(unix)]
fn ps_clock_accounts_for_hours_days_precision_and_invalid_records() {
    for (clock, milliseconds) in [
        ("0:00.01", 10.0),
        ("01:02.34", 62340.0),
        ("2:03:04.50", 7384500.0),
        ("1-02:03:04.50", 93784500.0),
    ] {
        assert!((resources::cpu_time_ms(clock).unwrap() - milliseconds).abs() < 0.001);
    }
    for invalid in [
        "", "1", "NaN", "0:NaN", "0:-1", "0:60", "1:60:00", "1:2:3:4",
    ] {
        assert!(resources::cpu_time_ms(invalid).is_err(), "{invalid}");
    }
    let rows = resources::ps_table(" 0 0 0 0 0:00.00\n123 9 123 2048 1:02.34\n").unwrap();
    assert_eq!(rows[1].rss_bytes, 2 * 1024 * 1024);
    assert_eq!(rows[1].cpu_ms, 62340.0);
    assert!(resources::ps_table("123 9 123 2048").is_err());
    assert!(resources::ps_table("123 9 123 -1 0:00").is_err());
}

#[test]
#[cfg(unix)]
fn owned_tree_retains_reparented_group_members_and_bounds_idle_clock_uncertainty() {
    let row = |pid, parent, group| Process {
        pid,
        parent,
        group,
        start_ticks: Some(pid as u64),
        rss_bytes: 1024,
        cpu_ms: 10.0,
    };
    let before = resources::owned(
        100,
        vec![
            row(0, 0, 0),
            row(40, 100, 40),
            row(2, 40, 2),
            row(100, 9, 100),
            row(150, 1, 100),
            row(200, 9, 200),
        ],
        20.0,
        "fixture",
    )
    .unwrap();
    assert_eq!(
        before.processes.keys().copied().collect::<Vec<_>>(),
        [2, 40, 100, 150]
    );
    assert_eq!(before.rss_bytes().unwrap(), 4096);
    let mut after = before.clone();
    after.processes.get_mut(&100).unwrap().cpu_ms += 20.0;
    let percent = before
        .idle_cpu_upper_percent(&after, Duration::from_secs(30))
        .unwrap();
    assert!((percent - 100.0 / 300.0).abs() < 0.0001);
    after.processes.get_mut(&100).unwrap().start_ticks = Some(999);
    assert!(
        before
            .idle_cpu_upper_percent(&after, Duration::from_secs(30))
            .is_err()
    );
    after = before.clone();
    after.processes.remove(&40);
    assert!(
        before
            .idle_cpu_upper_percent(&after, Duration::from_secs(30))
            .is_err()
    );
    after = before.clone();
    after.processes.get_mut(&2).unwrap().cpu_ms = 0.0;
    assert!(
        before
            .idle_cpu_upper_percent(&after, Duration::from_secs(30))
            .is_err()
    );
    assert!(
        before
            .idle_cpu_upper_percent(&before, Duration::ZERO)
            .is_err()
    );
    assert!(resources::owned(100, vec![row(200, 9, 200)], 20.0, "fixture").is_err());
    assert!(
        resources::owned(
            100,
            vec![row(100, 9, 100), row(100, 9, 100)],
            20.0,
            "fixture"
        )
        .is_err()
    );
}

#[test]
fn tui_measurement_never_launches_a_candidate_outside_hosted_execution() {
    for (arguments, expected) in [
        (
            vec!["measure-tui", "--candidate", "unused", "--output", "unused"],
            1,
        ),
        (
            vec![
                "measure-tui",
                "--candidate",
                "unused",
                "--output",
                "unused",
                "--samples",
                "1",
            ],
            2,
        ),
        (
            vec![
                "measure-tui",
                "--candidate",
                "unused",
                "--output",
                "unused",
                "--idle-seconds",
                "0",
            ],
            2,
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .env_remove("GITHUB_ACTIONS")
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(expected));
    }
}
