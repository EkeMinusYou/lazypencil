use lazypencil_xtask::ci::native::parallel;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

#[test]
fn browser_lanes_preserve_every_existing_target_once_with_bounded_parallelism() {
    use lazypencil_xtask::ci::native::browser::{TEST_LANES, test_threads};
    use std::collections::BTreeSet;
    let targets = TEST_LANES
        .into_iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let unique = targets.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), targets.len());
    assert_eq!(
        unique,
        BTreeSet::from([
            "browser_readiness",
            "dynamic_render",
            "imports",
            "sampler_sources",
            "script_scope",
            "shader_uniforms",
            "windows_agent",
        ])
    );
    // Readiness, retained pages and shaders own distinct artifact namespaces.
    // Ownership/destruction is checked after every lane has joined.
    assert_eq!(TEST_LANES[0], ["browser_readiness"]);
    assert_eq!(TEST_LANES[1], ["dynamic_render", "script_scope"]);
    assert_eq!(
        TEST_LANES[2],
        [
            "shader_uniforms",
            "sampler_sources",
            "imports",
            "windows_agent"
        ]
    );
    assert!(!targets.contains(&"agent"));
    for target in targets {
        assert_eq!(
            test_threads(target),
            if target == "browser_readiness" { 2 } else { 1 }
        );
    }
    let binary = std::env::temp_dir().join("verified-script-scope");
    let bindings = lazypencil_xtask::ci::native::browser::binding_commands(&binary);
    let reproduction = lazypencil_xtask::ci::native::browser::NATIVE_INPUT_GUARDS;
    assert_eq!(
        bindings
            .iter()
            .map(|command| command[1].as_str())
            .collect::<Vec<_>>(),
        [
            reproduction,
            "generated_browser_",
            "script_bindings::",
            "script_copy::"
        ],
        "Keep every original default-feature binding filter"
    );
    assert_eq!(bindings[0].last().unwrap(), "--exact");
    assert_eq!(&bindings[2][3..], ["--skip", reproduction]);
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests/support/script_generated_overrides.rs"),
    )
    .unwrap();
    assert!(
        source.contains(&format!(
            "fn {}(",
            reproduction.rsplit("::").next().unwrap()
        )),
        "The early exact filter must name an existing regression, never run zero tests"
    );
    for (index, command) in bindings.into_iter().enumerate() {
        assert_eq!(std::path::Path::new(&command[0]), binary.as_path());
        assert_eq!(
            command.len(),
            match index {
                0 => 4,
                2 => 5,
                _ => 3,
            }
        );
        assert_eq!(command[2], "--test-threads=1");
    }
}

#[test]
fn ci_lanes_overlap_join_every_completion_and_report_a_build_failure() {
    for names in [
        &["release compilation", "browser verification"][..],
        &["release compilation", "lifecycle", "Material"][..],
    ] {
        assert_lane_completion(names);
    }
}

fn assert_lane_completion(names: &[&'static str]) {
    let (entered, arrivals) = mpsc::channel();
    let finished = Arc::new(AtomicUsize::new(0));
    let mut releases = Vec::new();
    let mut tasks: Vec<parallel::Task<'_>> = Vec::new();
    for (index, &name) in names.iter().enumerate() {
        let (release, gate) = mpsc::channel();
        releases.push(release);
        let entered = entered.clone();
        let finished = finished.clone();
        tasks.push((
            name,
            Box::new(move || {
                entered.send(name)?;
                gate.recv_timeout(Duration::from_secs(5))?;
                finished.fetch_or(1 << index, Ordering::SeqCst);
                anyhow::ensure!(index != 0, "compiler fixture failed");
                Ok(())
            }),
        ));
    }
    drop(entered);
    // None of the lanes can finish until every lane has started. The bounded
    // channels make accidental serialization fail without an unbounded wait.
    let count = names.len();
    let controller = std::thread::spawn(move || -> anyhow::Result<Vec<&'static str>> {
        let mut seen = Vec::new();
        for _ in 0..count {
            seen.push(arrivals.recv_timeout(Duration::from_secs(5))?);
        }
        for release in releases {
            release.send(())?;
        }
        Ok(seen)
    });
    let result = parallel::run(tasks);
    let seen = controller.join().unwrap().unwrap();
    assert_eq!(seen.len(), count);
    assert_eq!(finished.load(Ordering::SeqCst), (1 << count) - 1);
    let error = format!("{:#}", result.unwrap_err());
    assert!(
        error.contains("release compilation") && error.contains("compiler fixture failed"),
        "{error}"
    );
}

#[test]
fn nested_runtime_groups_overlap_and_join_all_failures_before_ownership_cleanup() {
    use std::sync::{Condvar, Mutex};
    let (entered, arrivals) = mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let finished = Arc::new(AtomicUsize::new(0));
    let leaf = |name: &'static str, bit: usize| -> parallel::Task<'static> {
        let entered = entered.clone();
        let gate = gate.clone();
        let finished = finished.clone();
        (
            name,
            Box::new(move || {
                entered.send(name)?;
                let (lock, wake) = &*gate;
                let (_guard, deadline) = wake
                    .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |ready| {
                        !*ready
                    })
                    .unwrap();
                anyhow::ensure!(!deadline.timed_out(), "Runtime lanes were serialized");
                finished.fetch_or(bit, Ordering::SeqCst);
                anyhow::ensure!(
                    bit != 1 && bit != 16 && bit != 32,
                    "Expected {name} failure"
                );
                Ok(())
            }),
        )
    };
    let browser = vec![leaf("browser readiness", 1), leaf("dynamic pixels", 2)];
    let lifecycle = vec![leaf("idle lifecycle", 4), leaf("Material", 8)];
    let mcp = leaf("MCP", 16);
    let release = leaf("release compilation", 32);
    let controller = std::thread::spawn(move || -> anyhow::Result<()> {
        for _ in 0..6 {
            arrivals.recv_timeout(Duration::from_secs(5))?;
        }
        let (lock, wake) = &*gate;
        *lock.lock().unwrap() = true;
        wake.notify_all();
        Ok(())
    });
    let runtime: Vec<parallel::Task<'_>> = vec![
        ("browser", Box::new(|| parallel::run(browser))),
        mcp,
        (
            "lifecycle and Material",
            Box::new(|| parallel::run(lifecycle)),
        ),
    ];
    let result = parallel::run(vec![
        release,
        ("runtime suites", Box::new(|| parallel::run(runtime))),
    ]);
    controller.join().unwrap().unwrap();
    // Ownership/destruction and performance checks may begin only after every
    // already-started process owner has completed, including failed groups.
    assert_eq!(finished.load(Ordering::SeqCst), 63);
    let error = format!("{:#}", result.unwrap_err());
    assert!(
        error.contains("browser readiness")
            && error.contains("MCP")
            && error.contains("release compilation"),
        "{error}"
    );
}

#[test]
fn a_panicking_lane_cannot_hide_a_failed_lane_or_skip_another_completion() {
    let finished = AtomicUsize::new(0);
    let result = parallel::run(vec![
        ("panic fixture", Box::new(|| panic!("fixture panic"))),
        (
            "error fixture",
            Box::new(|| anyhow::bail!("second failure")),
        ),
        (
            "completion fixture",
            Box::new(|| {
                finished.store(1, Ordering::SeqCst);
                Ok(())
            }),
        ),
    ]);
    assert_eq!(finished.load(Ordering::SeqCst), 1);
    let error = format!("{:#}", result.unwrap_err());
    assert!(
        error.contains("panic fixture")
            && error.contains("task panicked")
            && error.contains("error fixture")
            && error.contains("second failure"),
        "{error}"
    );
    assert!(parallel::run(vec![]).is_err());
}
