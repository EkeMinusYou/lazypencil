use super::helper;
use lazypencil::{
    cdp::{Browser, Budget, Purpose},
    runtime::InstalledBrowser,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn launch_fake(directory: &Path, budget: &Budget<'_>) -> lazypencil::Result<Browser> {
    let executable = directory.join("browser fixture 日本語");
    let mode = if directory.join("exit").exists() {
        "exit"
    } else if directory.join("detach").exists() {
        "detach"
    } else if directory.join("adopt").exists() {
        "adopt"
    } else {
        "group"
    };
    super::fixture_support::install(
        &executable,
        &[
            "browser-tree".into(),
            directory.to_str().unwrap().into(),
            mode.into(),
            "root".into(),
        ],
    );
    Browser::launch_with_supervisor(
        &InstalledBrowser {
            executable,
            version: "fixture".into(),
            platform: "fixture".into(),
        },
        Purpose::Page,
        budget,
        Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
}

fn alive(pid: u32) -> bool {
    // Signal zero only queries liveness; tests never signal a reported PID.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn pids(value: &Value) -> Vec<u32> {
    value["pids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pid| u32::try_from(pid.as_u64().unwrap()).unwrap())
        .collect()
}

fn fake_pids(directory: &Path) -> Vec<u32> {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = fs::read(directory.join("browser-tree.json")) {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            let pids: Vec<_> = ["root", "branch", "leaf", "guardian"]
                .map(|key| u32::try_from(value[key].as_u64().unwrap()).unwrap())
                .into();
            assert!(pids.iter().copied().all(alive));
            return pids;
        }
        assert!(
            Instant::now() < end,
            "Browser fixture tree did not become ready"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fake_profile(directory: &Path) -> PathBuf {
    let value: Value =
        serde_json::from_slice(&fs::read(directory.join("browser-profile.json")).unwrap()).unwrap();
    let path = PathBuf::from(value["path"].as_str().unwrap());
    assert_eq!(
        fs::read(path.join("fixture-cache")).unwrap(),
        b"owned-profile\n"
    );
    path
}

fn gone(pids: &[u32]) {
    let end = Instant::now() + Duration::from_secs(3);
    while pids.iter().copied().any(alive) {
        assert!(
            Instant::now() < end,
            "Owned browser processes survived: {:?}",
            pids.iter()
                .copied()
                .filter(|pid| alive(*pid))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn cancellation_and_startup_deadlines_reap_browser_groups() {
    for cancel in [false, true] {
        let directory = TempDir::new().unwrap();
        let token = CancellationToken::new();
        std::thread::scope(|scope| {
            let startup = scope.spawn(|| {
                launch_fake(
                    directory.path(),
                    &Budget::new(Duration::from_secs(8), Some(&token)),
                )
                .err()
                .expect("The fixture never opens CDP")
            });
            let pids = fake_pids(directory.path());
            let profile = fake_profile(directory.path());
            if cancel {
                token.cancel();
            }
            let error = startup.join().unwrap();
            assert_eq!(
                error.code,
                if cancel {
                    "CANCELLED"
                } else {
                    "RENDER_TIMEOUT"
                }
            );
            gone(&pids);
            assert!(
                !profile.exists(),
                "Cancelled startup left its browser profile"
            );
        });
    }
}

struct Owner(Child);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn owner(directory: &Path, real: bool) -> Owner {
    let log = fs::File::create(directory.join("owner.log")).unwrap();
    Owner(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "browser_ownership::fixture_host", "--nocapture"])
            .env("LAZYPENCIL_TEST_UNIX_BROWSER_HOST", directory)
            .env(
                "LAZYPENCIL_TEST_UNIX_BROWSER_REAL",
                if real { "1" } else { "0" },
            )
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    )
}

fn ready(owner: &mut Owner, directory: &Path, file: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(if file == "ready.json" { 40 } else { 8 });
    loop {
        if let Ok(bytes) = fs::read(directory.join(file)) {
            return serde_json::from_slice(&bytes).unwrap();
        }
        if let Some(status) = owner.0.try_wait().unwrap() {
            panic!(
                "Browser owner exited ({status}): {}",
                fs::read_to_string(directory.join("owner.log")).unwrap()
            );
        }
        assert!(Instant::now() < end, "Browser owner did not publish {file}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn terminate(owner: &mut Owner) {
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
}

#[test]
fn killed_owners_reap_browser_descendants_without_stopping_another_owner() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let mut first_owner = owner(first.path(), false);
    let mut second_owner = owner(second.path(), false);
    ready(&mut first_owner, first.path(), "browser-tree.json");
    ready(&mut second_owner, second.path(), "browser-tree.json");
    let first_tree = fake_pids(first.path());
    let second_tree = fake_pids(second.path());
    let first_profile = fake_profile(first.path());
    let second_profile = fake_profile(second.path());
    terminate(&mut first_owner);
    gone(&first_tree);
    assert!(
        !first_profile.exists(),
        "Killed owner left its browser profile"
    );
    assert!(second_profile.is_dir());
    assert!(second_tree.iter().copied().all(alive));
    terminate(&mut second_owner);
    gone(&second_tree);
    assert!(!second_profile.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_subreaper_collects_detached_descendants_after_owner_death() {
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("detach"), "").unwrap();
    let mut host = owner(directory.path(), false);
    ready(&mut host, directory.path(), "browser-tree.json");
    let pids = fake_pids(directory.path());
    let root_group = unsafe { libc::getpgid(pids[0] as libc::pid_t) };
    assert_eq!(root_group, pids[0] as libc::pid_t);
    for pid in &pids[1..3] {
        let group = unsafe { libc::getpgid(*pid as libc::pid_t) };
        assert!(
            group > 0 && group != root_group,
            "The fixture must really escape the browser group"
        );
    }
    terminate(&mut host);
    gone(&pids);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_reaps_exited_orphans_while_the_browser_root_is_still_live() {
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("adopt"), "").unwrap();
    let mut host = owner(directory.path(), false);
    let tree = ready(&mut host, directory.path(), "browser-tree.json");
    let adopted = ready(&mut host, directory.path(), "adopted.json");
    assert_eq!(adopted["parent"], tree["guardian"]);
    assert_eq!(adopted["pid"], tree["leaf"]);
    gone(&[u32::try_from(tree["leaf"].as_u64().unwrap()).unwrap()]);
    assert!(alive(
        u32::try_from(tree["root"].as_u64().unwrap()).unwrap()
    ));
    let ids: Vec<_> = ["root", "branch", "leaf", "guardian"]
        .map(|key| u32::try_from(tree[key].as_u64().unwrap()).unwrap())
        .into();
    terminate(&mut host);
    gone(&ids);
}

#[test]
fn a_worker_without_a_live_owner_never_launches_a_browser() {
    let directory = TempDir::new().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .current_dir(directory.path())
        .arg("__browser_process_worker")
        .arg("--")
        .arg(helper())
        .args(["mark", "must-not-exist"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!directory.path().join("must-not-exist").exists());
    let status: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(status["phase"], "owner_start");
}

#[test]
fn browser_worker_status_is_bounded_and_observes_deadlines_and_cancellation() {
    for (mode, cancel) in [
        ("partial", false),
        ("partial", true),
        ("oversized", false),
        ("invalid", false),
        ("eof", false),
    ] {
        let directory = TempDir::new().unwrap();
        let helper = directory.path().join("status helper 日本語");
        super::fixture_support::install(
            &helper,
            &[
                "browser-tree".into(),
                directory.path().to_str().unwrap().into(),
                mode.into(),
                "supervisor".into(),
            ],
        );
        let token = CancellationToken::new();
        let started = Instant::now();
        std::thread::scope(|scope| {
            let startup = scope.spawn(|| {
                Browser::launch_with_supervisor(
                    &InstalledBrowser {
                        executable: directory.path().join("must-not-be-launched"),
                        version: "fixture".into(),
                        platform: "fixture".into(),
                    },
                    Purpose::Page,
                    &Budget::new(
                        Duration::from_secs(if cancel { 5 } else { 1 }),
                        Some(&token),
                    ),
                    &helper,
                )
                .err()
                .expect("A fixture status must never launch a real browser")
            });
            let ready_end = Instant::now() + Duration::from_secs(2);
            while !directory.path().join("status-ready.json").is_file() {
                assert!(
                    Instant::now() < ready_end,
                    "Status fixture did not become ready: {mode}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            if cancel {
                token.cancel();
            }
            let error = startup.join().unwrap();
            assert_eq!(
                error.code,
                if cancel {
                    "CANCELLED"
                } else if mode == "partial" {
                    "RENDER_TIMEOUT"
                } else {
                    "RUNTIME_UNAVAILABLE"
                },
                "{mode}"
            );
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "Status response exceeded its bound: {mode}"
            );
        });
    }
}

#[test]
fn missing_browser_keeps_a_structured_startup_failure() {
    let directory = TempDir::new().unwrap();
    let error = Browser::launch_with_supervisor(
        &InstalledBrowser {
            executable: directory.path().join("missing-browser"),
            version: "fixture".into(),
            platform: "fixture".into(),
        },
        Purpose::Page,
        &Budget::new(Duration::from_secs(5), None),
        Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .err()
    .expect("A missing executable must fail before the startup deadline");
    assert_eq!(error.code, "RUNTIME_UNAVAILABLE");
    let details = error.details.unwrap();
    assert_eq!(details["supervisor_phase"], "browser_spawn");
    assert!(details["os_error"].is_number());
}

#[test]
fn a_browser_root_exit_reaps_descendants_despite_the_hosts_ignored_sigchld() {
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("exit"), "").unwrap();
    fs::write(directory.path().join("ignore-chld"), "").unwrap();
    let mut host = owner(directory.path(), false);
    let error = ready(&mut host, directory.path(), "exited.json");
    assert_eq!(error["code"], "RUNTIME_UNAVAILABLE");
    let tree: Value =
        serde_json::from_slice(&fs::read(directory.path().join("browser-tree.json")).unwrap())
            .unwrap();
    let ids: Vec<_> = ["root", "branch", "leaf", "guardian"]
        .map(|key| u32::try_from(tree[key].as_u64().unwrap()).unwrap())
        .into();
    gone(&ids);
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = host.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < end, "The fixture host did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn real_browser() -> Browser {
    Browser::launch_with_supervisor(
        &InstalledBrowser {
            executable: std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
                .expect("Set LAZYPENCIL_TEST_CHROMIUM")
                .into(),
            version: "external-unverified".into(),
            platform: lazypencil::runtime::platform().unwrap().into(),
        },
        Purpose::Page,
        &Budget::new(Duration::from_secs(30), None),
        Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap()
}

fn ps_output(args: &[&str]) -> Vec<u8> {
    let mut file = tempfile::tempfile().unwrap();
    let mut child = Owner(
        Command::new("ps")
            .args(args)
            .stdout(file.try_clone().unwrap())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            file.metadata().unwrap().len() <= 1024 * 1024,
            "Oversized ps output"
        );
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "ps exited with {status}");
            break;
        }
        assert!(
            Instant::now() < end,
            "ps did not finish within its deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 1024 * 1024);
    bytes
}

fn observed_descendants(guardian: u32) -> BTreeSet<u32> {
    let output = ps_output(&["-A", "-o", "pid=", "-o", "ppid="]);
    let parents: BTreeMap<u32, u32> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| {
            let mut fields = line.split_ascii_whitespace();
            let pid = fields.next().unwrap().parse().unwrap();
            let parent = fields.next().unwrap().parse().unwrap();
            assert!(fields.next().is_none());
            (pid, parent)
        })
        .collect();
    let mut owned = BTreeSet::from([guardian]);
    loop {
        let before = owned.len();
        for (pid, parent) in &parents {
            if owned.contains(parent) {
                owned.insert(*pid);
            }
        }
        if owned.len() == before {
            return owned;
        }
    }
}

fn real_pids(browser: &mut Browser) -> Vec<u32> {
    let budget = Budget::new(Duration::from_secs(15), None);
    browser
        .command(
            None,
            "Target.createTarget",
            json!({"url":"data:text/html,<p>owned renderer</p>"}),
            &budget,
        )
        .unwrap();
    let processes = browser
        .command(None, "SystemInfo.getProcessInfo", json!({}), &budget)
        .unwrap();
    let mut pids: Vec<_> = processes["processInfo"]
        .as_array()
        .unwrap()
        .iter()
        .map(|info| u32::try_from(info["id"].as_u64().unwrap()).unwrap())
        .collect();
    assert!(pids.len() >= 2 && pids.contains(&browser.process_id()));
    let parent = ps_output(&["-o", "ppid=", "-p", &browser.process_id().to_string()]);
    let guardian: u32 = String::from_utf8(parent).unwrap().trim().parse().unwrap();
    assert_ne!(guardian, std::process::id());
    // Include OS-observed children such as zygotes that CDP may omit. Only
    // query these PIDs; normal exits between observations are harmless.
    pids.extend(observed_descendants(guardian));
    pids.sort_unstable();
    pids.dedup();
    #[cfg(target_os = "macos")]
    for pid in pids.iter().filter(|pid| **pid != guardian) {
        let group = unsafe { libc::getpgid(*pid as libc::pid_t) };
        if group == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            continue;
        }
        // Verify every observed managed descendant, including non-CDP children.
        assert_eq!(group, browser.process_id() as libc::pid_t);
    }
    assert!(alive(browser.process_id()) && alive(guardian));
    assert!(
        pids.iter()
            .any(|pid| *pid != browser.process_id() && *pid != guardian && alive(*pid))
    );
    pids
}

#[test]
fn fixture_host() {
    let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_UNIX_BROWSER_HOST") else {
        return;
    };
    let directory = Path::new(&directory);
    if directory.join("ignore-chld").exists() {
        // Only this child fixture changes its disposition; the test runner
        // and other concurrent owners keep their original signal policies.
        assert_ne!(
            unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) },
            libc::SIG_ERR
        );
    }
    if std::env::var_os("LAZYPENCIL_TEST_UNIX_BROWSER_REAL").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        let mut browser = real_browser();
        let ids = real_pids(&mut browser);
        fs::write(
            directory.join("ready.tmp"),
            json!({"pids":ids,"root":browser.process_id(),"profile":browser.profile_path()})
                .to_string(),
        )
        .unwrap();
        fs::rename(directory.join("ready.tmp"), directory.join("ready.json")).unwrap();
        let end = Instant::now() + Duration::from_secs(60);
        while Instant::now() < end {
            if directory.join("probe").exists() && !directory.join("probe.json").exists() {
                let version = browser
                    .command(
                        None,
                        "Browser.getVersion",
                        json!({}),
                        &Budget::new(Duration::from_secs(5), None),
                    )
                    .unwrap();
                fs::write(directory.join("probe.tmp"), version.to_string()).unwrap();
                fs::rename(directory.join("probe.tmp"), directory.join("probe.json")).unwrap();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("The parent test must terminate this browser owner");
    }
    let error = launch_fake(directory, &Budget::new(Duration::from_secs(60), None)).err();
    if directory.join("exit").exists() {
        let error = error.expect("The browser root exits before CDP startup");
        fs::write(
            directory.join("exited.tmp"),
            serde_json::to_vec(&error).unwrap(),
        )
        .unwrap();
        fs::rename(directory.join("exited.tmp"), directory.join("exited.json")).unwrap();
        return;
    }
    panic!("The parent test must terminate this owner before startup finishes: {error:?}");
}

#[test]
#[ignore = "Requires verified sandboxed Chromium"]
fn real_browser_drop_and_owner_death_reap_the_tree_and_preserve_other_owners() {
    let mut browser = real_browser();
    let ids = real_pids(&mut browser);
    let profile = browser.profile_path().to_path_buf();
    assert!(profile.is_dir());
    drop(browser);
    gone(&ids);
    assert!(!profile.exists(), "Browser drop left its profile");
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let mut first_owner = owner(first.path(), true);
    let mut second_owner = owner(second.path(), true);
    let first_tree = ready(&mut first_owner, first.path(), "ready.json");
    let second_tree = ready(&mut second_owner, second.path(), "ready.json");
    terminate(&mut first_owner);
    gone(&pids(&first_tree));
    assert!(!Path::new(first_tree["profile"].as_str().unwrap()).exists());
    assert!(Path::new(second_tree["profile"].as_str().unwrap()).is_dir());
    assert!(alive(
        u32::try_from(second_tree["root"].as_u64().unwrap()).unwrap()
    ));
    fs::write(second.path().join("probe"), "").unwrap();
    assert!(
        ready(&mut second_owner, second.path(), "probe.json")["product"]
            .as_str()
            .unwrap()
            .starts_with("HeadlessChrome/")
    );
    terminate(&mut second_owner);
    gone(&pids(&second_tree));
    assert!(!Path::new(second_tree["profile"].as_str().unwrap()).exists());
}
