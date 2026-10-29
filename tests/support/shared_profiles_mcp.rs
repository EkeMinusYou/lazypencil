//! Actual bound MCP clients coexist with the retained runtime used by previews.
use super::*;
use lazypencil::{
    cdp::{Browser, Budget},
    config::runtime::{PageNetworkPolicy, PageSelectorState, RuntimeSettings},
    dynamic::{DynamicRuntime, Raster},
    project::{Project, ResolvedNode},
    runtime::{InstalledBrowser, profiles::Profile},
    store::Store,
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[path = "page_wait_reference.rs"]
#[allow(
    dead_code,
    reason = "Shared HTTP fixture also serves other integration targets"
)]
mod reference;

struct Study {
    _temp: TempDir,
    file: std::path::PathBuf,
    data: std::path::PathBuf,
    user: std::path::PathBuf,
    browser: InstalledBrowser,
    profile: Profile,
    server: reference::Server,
    probe: Arc<AtomicBool>,
    node: ResolvedNode,
    settings: RuntimeSettings,
}

impl Study {
    fn new() -> Self {
        let (temp, file, data) = fixture();
        let probe = Arc::new(AtomicBool::new(false));
        let server = reference::Server::with_selector_probe(probe.clone());
        let url = format!("http://{}/profile-state", server.address);
        fs::write(&file,json!({"version":"2.21","children":[{"id":"page","type":"browser","width":32,"height":32,"url":url}]}).to_string()).unwrap();
        let browser = InstalledBrowser {
            executable: std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap().into(),
            version: "profile-sharing-fixture".into(),
            platform: lazypencil::runtime::platform().unwrap().into(),
        };
        let profile = Profile::create(&data, "shared-authentication-private").unwrap();
        let mut login = Browser::launch_profile(
            &browser,
            &profile,
            &Budget::new(Duration::from_secs(30), None),
            Some(Path::new(env!("CARGO_BIN_EXE_lazypencil"))),
            false,
        )
        .unwrap();
        let budget = Budget::new(Duration::from_secs(30), None);
        login.with_tab(&budget,|browser,session| {
            browser.command(Some(session),"Page.navigate",json!({"url":url}),&budget)?;
            let ready = format!("location.href==={}&&document.readyState==='complete'",json!(url));
            while browser.evaluate(session,&ready,&budget)? != true {
                std::thread::sleep(Duration::from_millis(20).min(budget.remaining()?));
            }
            let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()+3600;
            assert_eq!(browser.command(Some(session),"Network.setCookie",json!({"name":"lp-profile-cookie","value":"profile-fixture-secret","url":url,"httpOnly":true,"expires":expires}),&budget)?["success"],true);
            assert_eq!(browser.evaluate(session,reference::PROFILE_SEED,&budget)?,true);
            Ok(())
        }).unwrap();
        drop(login);
        let user = temp.path().join("user");
        fs::create_dir(&user).unwrap();
        fs::write(user.join("config.toml"),"[runtime.browser]\nprofile='shared-authentication-private'\nready_selector='html[data-profile-ready]'\nselector_state='attached'\ntimeout_ms=30000\n").unwrap();
        let mut settings = RuntimeSettings::default();
        settings.browser.profile = "shared-authentication-private".into();
        settings.browser.ready_selector = "html[data-profile-ready]".into();
        settings.browser.selector_state = PageSelectorState::Attached;
        settings.browser.timeout_ms = 30000;
        let node = Project::load(&Store::open(&file, Some(data.clone())).unwrap())
            .unwrap()
            .resolve()
            .unwrap()
            .roots
            .remove(0);
        Self {
            _temp: temp,
            file,
            data,
            user,
            browser,
            profile,
            server,
            probe,
            node,
            settings,
        }
    }
    fn runtime(&self) -> DynamicRuntime {
        let mut runtime = DynamicRuntime::new(&self.data)
            .with_executable(&self.browser.executable)
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_retained_pages(&self.file);
        runtime.configure(self.settings.clone()).unwrap();
        runtime
    }
    fn mcp(&self, file: &Path) -> Client {
        Client::start_with_user_config(
            file,
            &self.data,
            Some(&self.browser.executable),
            &[],
            None,
            Some(&self.user),
        )
    }
    fn capture(&self, runtime: &mut DynamicRuntime) -> Raster {
        runtime.begin_render(None, true);
        let result = runtime.page(&self.node, 32.0, 32.0, 1.0).unwrap();
        pixels(&result.png);
        assert!(
            !result
                .metadata
                .to_string()
                .contains("shared-authentication-private")
        );
        assert!(
            !result
                .metadata
                .to_string()
                .contains("profile-fixture-secret")
        );
        result
    }
    fn wait_for_selector(&self) {
        let end = Instant::now() + Duration::from_secs(30);
        while !self.probe.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < end,
                "The owned page did not observe its selector"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn pixels(png: &[u8]) {
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!((image.width(), image.height()), (32, 32));
    let raster = image.peek_pixels().unwrap();
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(raster.get_color((x, y)), skia_safe::Color::GREEN, "{x},{y}");
        }
    }
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn bound_mcp_and_retained_previews_share_authentication_without_cross_owner_eviction() {
    let study = Study::new();
    let original = fs::read(&study.file).unwrap();
    let mut preview = study.runtime();
    let first = study.capture(&mut preview);
    pixels(&first.png);
    let page = first.metadata["page_state"]["id"].clone();
    let pid = preview.process_ids();
    assert_eq!(pid.len(), 1);
    // An already running worker cannot silently serve a different engine or
    // Chromium selection. Rejection must leave its existing owner untouched.
    for engine in [false, true] {
        let mut incompatible = if engine {
            study
                .runtime()
                .with_supervisor_executable(std::env::current_exe().unwrap())
        } else {
            study
                .runtime()
                .with_executable(std::env::current_exe().unwrap())
        };
        incompatible.begin_render(None, true);
        let error = incompatible
            .page(&study.node, 32.0, 32.0, 1.0)
            .err()
            .unwrap();
        assert_eq!(error.details.unwrap()["stage"], "profile_context");
        assert!(incompatible.process_ids().is_empty());
    }
    let mut other = study.runtime();
    let second = study.capture(&mut other);
    assert_eq!(other.process_ids(), pid);
    assert_ne!(second.metadata["page_state"]["id"], page);
    // More than the global old limit must not evict another owner's page.
    for i in 0..9 {
        let mut node = study.node.clone();
        node.address.node_id = format!("owner-two-{i}");
        other.begin_render(None, true);
        pixels(&other.page(&node, 32.0, 32.0, 1.0).unwrap().png);
    }
    assert_eq!(other.retained_page_count(), 8);
    let mut mcp = study.mcp(&study.file);
    let document = mcp.data("get_document", json!({}));
    for refresh in [false, true] {
        let (png, report) = super::local_pages::capture(&mut mcp, &document, refresh);
        pixels(&png);
        assert!(!report.to_string().contains("profile-fixture-secret"));
    }
    mcp.finish();
    drop(other);
    let after = study.capture(&mut preview);
    pixels(&after.png);
    assert_eq!(after.metadata["page_state"]["id"], page);
    assert_eq!(after.metadata["page_state"]["reused"], true);
    assert_eq!(preview.process_ids(), pid);
    assert_eq!(fs::read(&study.file).unwrap(), original);
    drop(preview);
    // The last owner acknowledges teardown before a direct login can acquire
    // the original exclusive guardian lease again.
    drop(
        Browser::launch_profile(
            &study.browser,
            &study.profile,
            &Budget::new(Duration::from_secs(30), None),
            Some(Path::new(env!("CARGO_BIN_EXE_lazypencil"))),
            false,
        )
        .unwrap(),
    );
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn cancelled_and_killed_bound_mcp_preserve_another_previews_browser_and_page() {
    let study = Study::new();
    let mut preview = study.runtime();
    let first = study.capture(&mut preview);
    let pid = preview.process_ids();
    let busy_file = study._temp.path().join("busy.pen");
    Store::init(&busy_file, Some(study.data.clone())).unwrap();
    fs::write(&busy_file,json!({"version":"2.21","children":[{"id":"page","type":"browser","width":32,"height":32,"url":format!("http://{}/selector-cancel",study.server.address)}]}).to_string()).unwrap();
    let busy_user = study._temp.path().join("busy-user");
    fs::create_dir(&busy_user).unwrap();
    fs::write(busy_user.join("config.toml"),"[runtime.browser]\nprofile='shared-authentication-private'\nready_selector='#never'\ntimeout_ms=30000\n").unwrap();
    for kill in [false, true] {
        study.probe.store(false, Ordering::SeqCst);
        let mut mcp = Client::start_with_user_config(
            &busy_file,
            &study.data,
            Some(&study.browser.executable),
            &[],
            None,
            Some(&busy_user),
        );
        let document = mcp.data("get_document", json!({}));
        let request = mcp.start_request("tools/call",json!({"name":"render","arguments":{"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"refresh":true}}));
        study.wait_for_selector();
        if kill {
            mcp.child.kill().unwrap();
            mcp.child.wait().unwrap();
        } else {
            let cancelled = Instant::now();
            mcp.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request,"reason":"cancel owned profile page"}}));
            // RMCP drops a cancelled response. As in the existing cancellation
            // regressions, the next request proves that owned work was released.
            assert_eq!(
                mcp.data("get_document", json!({}))["revision"],
                document["revision"]
            );
            assert!(
                cancelled.elapsed() < Duration::from_secs(5),
                "Cancellation did not release the document worker before the original 30-second page deadline"
            );
            mcp.finish();
        }
        let recovered = study.capture(&mut preview);
        assert_eq!(
            preview.process_ids(),
            pid,
            "Another owner must not restart Chromium"
        );
        assert_eq!(
            recovered.metadata["page_state"]["id"],
            first.metadata["page_state"]["id"]
        );
        assert_eq!(recovered.metadata["page_state"]["reused"], true);
    }
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn shared_profile_policy_switches_preserve_storage_and_never_run_offline_input_online() {
    let study = Study::new();
    let mut online = study.runtime();
    study.capture(&mut online);
    let mut offline = study.runtime();
    let mut settings = study.settings.clone();
    settings.browser.network_policy = PageNetworkPolicy::Offline;
    settings.browser.additional_wait_ms = 1500;
    offline.configure(settings).unwrap();
    let mut node = study.node.clone();
    let html = format!(
        "<html data-profile-ready style='height:100%;background:#00ff00'><script>fetch('http://{}/network/fetch',{{mode:'no-cors'}}).catch(()=>{{}})</script></html>",
        study.server.address
    );
    use base64::Engine;
    node.properties.insert(
        "url".into(),
        json!(format!(
            "data:text/html;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(html)
        )),
    );
    let before = study.server.network_counts();
    for _ in 0..2 {
        offline.begin_render(None, true);
        let raster = offline.page(&node, 32.0, 32.0, 1.0).unwrap();
        pixels(&raster.png);
        assert_eq!(study.server.network_counts(), before);
        // Switching back must restore all four seeded storage classes; the
        // process relaunch intentionally resets DOM when launch policy changes.
        study.capture(&mut online);
    }
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn shared_profile_keeps_identical_file_urls_bound_to_each_projects_captured_resources() {
    let study = Study::new();
    let make_project = |name: &str, color: &str| {
        let root = study._temp.path().join(name);
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::create_dir(root.join("web")).unwrap();
        fs::write(root.join("web/index.html"),"<!doctype html><html data-profile-ready><link rel='icon' href='data:,'><link rel='stylesheet' href='style.css'></html>").unwrap();
        fs::write(
            root.join("web/style.css"),
            format!("html,body{{margin:0;height:100%;background:{color}}}"),
        )
        .unwrap();
        let file = root.join("local.pen");
        Store::init(&file, Some(study.data.clone())).unwrap();
        fs::write(&file,json!({"version":"2.21","metadata":{"lazypencil":{"browser_resources":{"version":1,"roots":[{"url":"file:///original/site/","path":"web"}]}}},"children":[{"id":"page","type":"browser","width":32,"height":32,"url":"file:///original/site/index.html"}]}).to_string()).unwrap();
        file
    };
    let green = make_project("green", "#00ff00");
    let blue = make_project("blue", "#0000ff");
    let project = Project::load(&Store::open(&green, Some(study.data.clone())).unwrap()).unwrap();
    let node = project.resolve().unwrap().roots.remove(0);
    let mut preview = study.runtime().with_retained_pages(&green);
    preview.begin_render(None, true);
    let first = preview
        .page_in_project(&project, &node, 32.0, 32.0, 1.0)
        .unwrap();
    pixels(&first.png);
    assert_eq!(first.metadata["local_files"]["fulfilled"], 2);
    let pid = preview.process_ids();
    let mut mcp = study.mcp(&blue);
    let document = mcp.data("get_document", json!({}));
    let (png, report) = super::local_pages::capture(&mut mcp, &document, true);
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!((image.width(), image.height()), (32, 32));
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(
                image.peek_pixels().unwrap().get_color((x, y)),
                skia_safe::Color::BLUE
            );
        }
    }
    assert_eq!(
        report["dynamic_artifacts"][0]["acquisition"]["local_files"]["fulfilled"],
        2
    );
    mcp.finish();
    preview.begin_render(None, true);
    let after = preview
        .page_in_project(&project, &node, 32.0, 32.0, 1.0)
        .unwrap();
    pixels(&after.png);
    assert_eq!(preview.process_ids(), pid);
    assert_eq!(
        after.metadata["page_state"]["id"],
        first.metadata["page_state"]["id"]
    );
    assert_eq!(after.metadata["page_state"]["reused"], true);
}
