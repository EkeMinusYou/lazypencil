use super::*;
use lazypencil::{
    cdp::{Browser, Budget},
    config::runtime::{PageNetworkPolicy, PageSelectorState, RuntimeSettings},
    runtime::{InstalledBrowser, profiles::Profile},
};

const PROFILE: &str = "authentication-fixture-private";

#[path = "../../xtask/src/test_support/command_output.rs"]
#[allow(dead_code)]
mod command_output;

fn installed() -> InstalledBrowser {
    InstalledBrowser {
        executable: std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap().into(),
        version: "test-managed-runtime".into(),
        platform: lazypencil::runtime::platform().unwrap().into(),
    }
}

fn launch(profile: &Profile) -> lazypencil::Result<Browser> {
    Browser::launch_profile(
        &installed(),
        profile,
        &Budget::new(Duration::from_secs(30), None),
        Some(std::path::Path::new(env!("CARGO_BIN_EXE_lazypencil"))),
        false,
    )
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn dedicated_profiles_persist_full_storage_isolate_acquisitions_and_keep_secrets_out_of_metadata() {
    let server = reference::Server::new();
    let url = format!("http://{}/profile-state", server.address);
    let f = Fixture::new(&url);
    let profile = Profile::create(f.0.path(), PROFILE).unwrap();
    Profile::create(f.0.path(), "isolated").unwrap();
    let mut browser = launch(&profile).unwrap();
    let busy = launch(&profile)
        .err()
        .expect("A second owner must not acquire the same profile");
    assert_eq!(
        busy.details.as_ref().unwrap()["supervisor_phase"],
        "profile_lock"
    );
    let budget = Budget::new(Duration::from_secs(30), None);
    browser.with_tab(&budget, |browser, session| {
        browser.command(Some(session), "Page.navigate", json!({"url":url}), &budget)?;
        let expression = format!("location.href==={}&&document.readyState==='complete'", json!(url));
        while browser.evaluate(session, &expression, &budget)? != true {
            std::thread::sleep(Duration::from_millis(20).min(budget.remaining()?));
        }
        let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()+3600;
        assert_eq!(browser.command(Some(session), "Network.setCookie", json!({"name":"lp-profile-cookie","value":"profile-fixture-secret","url":url,"httpOnly":true,"expires":expires}), &budget)?["success"], true);
        assert_eq!(browser.evaluate(session, reference::PROFILE_SEED, &budget)?, true);
        Ok(())
    }).unwrap();
    drop(browser);
    assert!(profile.directory().is_dir());
    let project = f.project();
    let scene = project.resolve().unwrap();
    let node = &scene.roots[0];
    let mut evidence = Vec::new();
    for retained in [false, true] {
        let mut runtime = f.runtime();
        if retained {
            runtime = runtime.with_retained_pages(&f.0.path().join("scene.pen"));
        }
        let mut settings = RuntimeSettings::default();
        settings.browser.ready_selector = "html[data-profile-ready]".into();
        settings.browser.selector_state = PageSelectorState::Attached;
        for (stage, selected, rgba) in [
            ("restarted", PROFILE, [0, 255, 0, 255]),
            ("isolated", "isolated", [255, 0, 0, 255]),
            ("anonymous", "", [255, 0, 0, 255]),
            ("restored", PROFILE, [0, 255, 0, 255]),
        ] {
            settings.browser.profile = selected.into();
            runtime.configure(settings.clone()).unwrap();
            runtime.begin_render(None, true);
            let raster = runtime.page(node, 32.0, 32.0, 1.0).unwrap_or_else(|error| {
                panic!("Profile acquisition failed (retained={retained}, stage={stage}): {error:?}")
            });
            retain(&mut evidence, retained, stage, &raster, rgba);
        }
        settings.browser.network_policy = PageNetworkPolicy::Offline;
        runtime.configure(settings.clone()).unwrap();
        runtime.begin_render(None, true);
        assert_eq!(
            runtime.page(node, 32.0, 32.0, 1.0).err().unwrap().code,
            "PAGE_LOAD_FAILED"
        );
        assert!(runtime.process_ids().is_empty());
        let mut local = node.clone();
        local.properties.insert("url".into(), json!(page(&format!("<style>html{{height:100%;background:#00ff00}}</style><script>fetch('http://{}/network/fetch',{{mode:'no-cors'}}).catch(()=>{{}});document.documentElement.dataset.profileReady='yes';</script>",server.address))));
        let before_network = server.network_counts();
        settings.browser.additional_wait_ms = 1500;
        runtime.configure(settings.clone()).unwrap();
        runtime.begin_render(None, true);
        retain(
            &mut evidence,
            retained,
            "offline-local",
            &runtime.page(&local, 32.0, 32.0, 1.0).unwrap(),
            [0, 255, 0, 255],
        );
        assert_eq!(server.network_counts(), before_network);
        settings.browser.network_policy = PageNetworkPolicy::Online;
        settings.browser.additional_wait_ms = 0;
        runtime.configure(settings).unwrap();
        runtime.begin_render(None, true);
        retain(
            &mut evidence,
            retained,
            "online-restored",
            &runtime.page(node, 32.0, 32.0, 1.0).unwrap(),
            [0, 255, 0, 255],
        );
    }
    // Exercise the public user-configuration path with the actual CLI as well.
    let user = f.0.path().join("user");
    fs::create_dir(&user).unwrap();
    fs::write(user.join("config.toml"), format!("[runtime.browser]\nprofile={}\nready_selector='html[data-profile-ready]'\nselector_state='attached'\n",json!(PROFILE))).unwrap();
    f.policy("html[data-profile-ready]", "attached", 15000);
    let before = fs::read(f.0.path().join("scene.pen")).unwrap();
    let output = command_output::output(
        std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(f.0.path())
            .args(["render"])
            .arg(f.0.path().join("scene.pen"))
            .arg("--output")
            .arg(f.0.path().join("cli.png"))
            .env("LAZYPENCIL_CONFIG_DIR", &user)
            .env("LAZYPENCIL_CHROMIUM", installed().executable),
        Duration::from_secs(90),
        4 * 1024 * 1024,
    )
    .unwrap();
    assert!(!output.timed_out);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!report.to_string().contains(PROFILE));
    assert!(!report.to_string().contains("profile-fixture-secret"));
    assert_eq!(fs::read(f.0.path().join("scene.pen")).unwrap(), before);
    let png = fs::read(f.0.path().join("cli.png")).unwrap();
    pixels(&png, [0, 255, 0, 255]);
    let before_login = server.profile_requests();
    let login = command_output::output(
        std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(f.0.path())
            .args(["runtime", "profile", "login", PROFILE, "--browser"])
            .arg(&installed().executable)
            .arg("--url")
            .arg(&url)
            .args(["--headless", "--timeout-seconds", "10"])
            .env("LAZYPENCIL_CONFIG_DIR", &user)
            .env("LAZYPENCIL_CHROMIUM", installed().executable),
        Duration::from_secs(60),
        4 * 1024 * 1024,
    )
    .unwrap();
    assert!(!login.timed_out && !login.status.success());
    let login: Value = serde_json::from_slice(&login.stdout).unwrap();
    assert_eq!(login["error"]["code"], "RENDER_TIMEOUT");
    assert!(
        server.profile_requests() > before_login,
        "The real login browser must navigate before its deadline"
    );
    let after = command_output::output(
        std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(f.0.path())
            .arg("render")
            .arg(f.0.path().join("scene.pen"))
            .arg("--output")
            .arg(f.0.path().join("cli-after-login.png"))
            .env("LAZYPENCIL_CONFIG_DIR", &user)
            .env("LAZYPENCIL_CHROMIUM", installed().executable),
        Duration::from_secs(90),
        4 * 1024 * 1024,
    )
    .unwrap();
    assert!(!after.timed_out && after.status.success());
    assert_eq!(
        fs::read(f.0.path().join("cli-after-login.png")).unwrap(),
        png
    );
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        let directory = std::path::PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("codec-dedicated-profiles-cli.png"), &png).unwrap();
        fs::write(directory.join("codec-dedicated-profiles.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":evidence,"cli_png_hash":lazypencil::document::hash(&png),"storage":["http_only_cookie","local_storage","indexed_db","cache_storage"],"exclusive_owner":true,"login_navigation_before_deadline":true,"post_login_png_hash":lazypencil::document::hash(&png),"sandbox":"enabled"})).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "Requires a pinned full Chromium and an owned headed display"]
fn headed_login_closes_normally_and_preserves_storage_for_the_same_full_browser() {
    let executable = std::path::PathBuf::from(
        std::env::var_os("LAZYPENCIL_TEST_FULL_CHROMIUM").expect("Full browser fixture required"),
    );
    let server = reference::Server::new();
    let f = Fixture::new(&format!("http://{}/profile-state", server.address));
    let name = "headed-login-private";
    let profile = Profile::create(f.0.path(), name).unwrap();
    let user = f.0.path().join("user");
    fs::create_dir(&user).unwrap();
    fs::write(user.join("config.toml"),format!("[runtime.browser]\nprofile={}\nready_selector='html[data-profile-ready]'\nselector_state='attached'\n",json!(name))).unwrap();
    f.policy("html[data-profile-ready]", "attached", 15000);
    let before = fs::read(f.0.path().join("scene.pen")).unwrap();
    let mut command = if cfg!(target_os = "linux") {
        let mut command = std::process::Command::new("xvfb-run");
        // Only this CLI and its owned browser inherit the isolated display.
        command
            .args(["-a", "-s", "-screen 0 1280x960x24 -nolisten tcp -noreset"])
            .arg(env!("CARGO_BIN_EXE_lazypencil"));
        command
    } else {
        std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
    };
    command
        .args(["--json", "--data-dir"])
        .arg(f.0.path())
        .args(["runtime", "profile", "login", name, "--browser"])
        .arg(&executable)
        .arg("--url")
        .arg(format!("http://{}/headed-login", server.address))
        .args(["--timeout-seconds", "45"])
        .env("LAZYPENCIL_CONFIG_DIR", &user)
        .env("LAZYPENCIL_CHROMIUM", &executable);
    let login_url = format!("http://{}/headed-login", server.address);
    let (login, observed, close) = std::thread::scope(|threads| {
        let login = threads.spawn(move || {
            command_output::output(&mut command, Duration::from_secs(90), 4 * 1024 * 1024)
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut observed = None;
        while Instant::now() < deadline {
            if let Some(proof) = server.headed_login_ready() {
                observed = Some(proof);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Drive the owned browser's close operation, rather than relying on
        // page-script permissions to close a window. Always collect the CLI,
        // including when observation or the bounded close request fails.
        let close = observed
            .as_ref()
            .map(|_| close_owned_login_target(&profile, &login_url));
        (login.join().unwrap().unwrap(), observed, close)
    });
    assert!(
        !login.timed_out && login.status.success(),
        "observed={observed:?} close={close:?} stdout={} stderr={}",
        String::from_utf8_lossy(&login.stdout),
        String::from_utf8_lossy(&login.stderr)
    );
    assert_eq!(close, Some(Ok(())));
    let login: Value = serde_json::from_slice(&login.stdout).unwrap();
    assert_eq!(login["browser_closed"], true);
    let observed = observed.expect("The full browser must report its headed page before closing");
    assert!(observed["width"].as_u64().unwrap() > 0 && observed["height"].as_u64().unwrap() > 0);
    assert_eq!(observed["visible"], true);
    assert_eq!(observed["headless"], false);
    assert_eq!(observed["chrome"], true);
    assert!(profile.directory().is_dir());
    let output = command_output::output(
        std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(f.0.path())
            .arg("render")
            .arg(f.0.path().join("scene.pen"))
            .arg("--output")
            .arg(f.0.path().join("restored.png"))
            .env("LAZYPENCIL_CONFIG_DIR", &user)
            .env("LAZYPENCIL_CHROMIUM", &executable),
        Duration::from_secs(90),
        4 * 1024 * 1024,
    )
    .unwrap();
    assert!(
        !output.timed_out && output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!metadata.to_string().contains(name));
    assert!(!metadata.to_string().contains("profile-fixture-secret"));
    assert_eq!(fs::read(f.0.path().join("scene.pen")).unwrap(), before);
    let png = fs::read(f.0.path().join("restored.png")).unwrap();
    pixels(&png, [0, 255, 0, 255]);
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        let directory = std::path::PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("codec-headed-login.png"), &png).unwrap();
        fs::write(directory.join("codec-headed-login.json"),serde_json::to_vec_pretty(&json!({
            "source_commit":std::env::var("GITHUB_SHA").ok(),"headed_observation":observed,"browser_closed":true,
            "browser_version":std::env::var("LAZYPENCIL_TEST_FULL_CHROMIUM_VERSION").unwrap(),
            "archive_sha256":std::env::var("LAZYPENCIL_TEST_FULL_CHROMIUM_SHA256").unwrap(),
            "close_method":"owned_browser_discovery_target_close",
            "same_canonical_browser":true,"storage":["http_only_cookie","local_storage","indexed_db","cache_storage"],
            "png":"codec-headed-login.png","png_hash":lazypencil::document::hash(&png),"rgba":[0,255,0,255],
            "document_unchanged":true,"sandbox":"enabled"
        })).unwrap()).unwrap();
    }
}

fn close_owned_login_target(profile: &Profile, url: &str) -> Result<(), String> {
    use std::io::Read;

    let mut port_file = String::new();
    fs::File::open(profile.directory().join("DevToolsActivePort"))
        .and_then(|file| file.take(513).read_to_string(&mut port_file))
        .map_err(|error| format!("Owned browser endpoint: {error}"))?;
    let mut lines = port_file.lines();
    let port = lines
        .next()
        .and_then(|line| line.parse::<std::num::NonZeroU16>().ok())
        .ok_or("Invalid owned browser port")?;
    if port_file.len() > 512
        || !lines
            .next()
            .is_some_and(|line| line.starts_with("/devtools/browser/"))
    {
        return Err("Invalid owned browser endpoint".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into();
    let request = |path: &str| -> Result<Vec<u8>, String> {
        let mut response = agent
            .get(format!("http://127.0.0.1:{port}{path}"))
            .call()
            .map_err(|error| format!("Owned browser control: {error}"))?;
        if response.status() != 200 {
            return Err("Owned browser control returned a non-200 status".into());
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Owned browser control response: {error}"))?;
        if bytes.len() > 65536 {
            return Err("Owned browser control response exceeds its bound".into());
        }
        Ok(bytes)
    };
    let targets: Vec<Value> = serde_json::from_slice(&request("/json/list")?)
        .map_err(|error| format!("Owned browser target list: {error}"))?;
    let targets = targets
        .iter()
        .filter(|target| target["type"] == "page" && target["url"] == url)
        .collect::<Vec<_>>();
    if targets.len() != 1 {
        return Err("Expected exactly one owned login page".into());
    }
    let id = targets[0]["id"].as_str().ok_or("Missing login target ID")?;
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("Invalid login target ID".into());
    }
    // Chromium's discovery endpoint invokes DevToolsAgentHost::Close().
    // https://chromium.googlesource.com/chromium/src/+/main/content/browser/devtools/devtools_http_handler.cc
    let closed = request(&format!("/json/close/{id}"))?;
    if !String::from_utf8_lossy(&closed).contains("Target is closing") {
        return Err("Owned browser did not acknowledge target close".into());
    }
    Ok(())
}

fn pixels(png: &[u8], rgba: [u8; 4]) {
    let image = Image::from_encoded(Data::new_copy(png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!((image.width(), image.height()), (32, 32));
    let expected = Color::from_argb(rgba[3], rgba[0], rgba[1], rgba[2]);
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(image.peek_pixels().unwrap().get_color((x, y)), expected);
        }
    }
}

fn retain(
    cases: &mut Vec<Value>,
    retained: bool,
    stage: &str,
    raster: &lazypencil::dynamic::Raster,
    rgba: [u8; 4],
) {
    pixels(&raster.png, rgba);
    assert!(!raster.metadata.to_string().contains(PROFILE));
    assert!(
        !raster
            .metadata
            .to_string()
            .contains("profile-fixture-secret")
    );
    let name = format!("codec-dedicated-profiles-{retained}-{stage}.png");
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        let directory = std::path::PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(&name), &raster.png).unwrap();
    }
    cases.push(json!({"retained":retained,"stage":stage,"png":name,"png_hash":lazypencil::document::hash(&raster.png),"rgba":rgba,"metadata":raster.metadata}));
}
