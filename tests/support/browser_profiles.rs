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
        assert_eq!(browser.evaluate(session, r#"(async()=>{localStorage.setItem('lp-auth','profile-fixture-secret');const db=await new Promise((resolve,reject)=>{const request=indexedDB.open('lp-auth-state',1);request.onupgradeneeded=()=>request.result.createObjectStore('state');request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error)});await new Promise((resolve,reject)=>{const tx=db.transaction('state','readwrite');tx.oncomplete=resolve;tx.onerror=()=>reject(tx.error);tx.objectStore('state').put('profile-fixture-secret','auth')});db.close();const cache=await caches.open('lp-auth-cache');await cache.put('/profile-cache-entry',new Response('profile-fixture-secret'));return true})()"#, &budget)?, true);
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
            let raster = runtime.page(node, 32.0, 32.0, 1.0).unwrap();
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
