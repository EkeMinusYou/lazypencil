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
fn browser_lanes_preserve_every_existing_target_once_without_intra_target_parallelism() {
    use lazypencil_xtask::ci::native::browser::TEST_LANES;
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
    // Readiness owns browser-* outputs; its partner owns retained-* and
    // shader/sampler outputs. Ownership/destruction is checked after joining.
    assert_eq!(TEST_LANES[0], ["browser_readiness"]);
    assert!(!targets.contains(&"agent"));
}

#[test]
fn ci_lanes_overlap_join_every_completion_and_report_a_build_failure() {
    let (entered, arrivals) = mpsc::channel();
    let finished = Arc::new(AtomicUsize::new(0));
    let mut releases = Vec::new();
    let mut tasks: Vec<parallel::Task<'_>> = Vec::new();
    for (index, name) in ["release compilation", "lifecycle", "Material"]
        .into_iter()
        .enumerate()
    {
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
    let controller = std::thread::spawn(move || -> anyhow::Result<Vec<&'static str>> {
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(arrivals.recv_timeout(Duration::from_secs(5))?);
        }
        for release in releases {
            release.send(())?;
        }
        Ok(seen)
    });
    let result = parallel::run(tasks);
    let seen = controller.join().unwrap().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(finished.load(Ordering::SeqCst), 7);
    let error = format!("{:#}", result.unwrap_err());
    assert!(
        error.contains("release compilation") && error.contains("compiler fixture failed"),
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
