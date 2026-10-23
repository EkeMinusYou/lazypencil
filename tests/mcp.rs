use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<Value>,
    next_id: u64,
}

impl Client {
    fn start(file: &std::path::Path, data: &std::path::Path) -> Self {
        Self::start_with_chromium(file, data, None)
    }
    fn start_with_chromium(
        file: &std::path::Path,
        data: &std::path::Path,
        chromium: Option<&std::path::Path>,
    ) -> Self {
        Self::start_configured(file, data, chromium, &[])
    }
    fn start_configured(
        file: &std::path::Path,
        data: &std::path::Path,
        chromium: Option<&std::path::Path>,
        fonts: &[std::path::PathBuf],
    ) -> Self {
        Self::start_with_context(file, data, chromium, fonts, None)
    }
    fn start_with_context(
        file: &std::path::Path,
        data: &std::path::Path,
        chromium: Option<&std::path::Path>,
        fonts: &[std::path::PathBuf],
        usage: Option<&std::path::Path>,
    ) -> Self {
        Self::start_with_user_config(file, data, chromium, fonts, usage, None)
    }
    fn start_with_user_config(
        file: &std::path::Path,
        data: &std::path::Path,
        chromium: Option<&std::path::Path>,
        fonts: &[std::path::PathBuf],
        usage: Option<&std::path::Path>,
        user: Option<&std::path::Path>,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        if let Some(path) = user {
            command.env("LAZYPENCIL_CONFIG_DIR", path);
        }
        if let Some(path) = chromium {
            command.env("LAZYPENCIL_CHROMIUM", path);
        }
        command
            .arg("--data-dir")
            .arg(data)
            .args(["mcp", "--file"])
            .arg(file);
        for path in fonts {
            command.arg("--font").arg(path);
        }
        if let Some(path) = usage {
            command.arg("--guidance-audit-manifest").arg(path);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let line = line.unwrap();
                let value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("Non-protocol stdout {line:?}: {e}"));
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            messages,
            next_id: 1,
        };
        let initialized = client.request(
            "initialize",
            json!({"protocolVersion":"2025-11-25",
            "capabilities":{},"clientInfo":{"name":"lazypencil-integration-test","version":"1"}}),
        );
        assert_eq!(
            initialized["result"]["protocolVersion"], "2025-11-25",
            "{initialized}"
        );
        assert_eq!(initialized["result"]["serverInfo"]["name"], "lazypencil");
        client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        client
    }

    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
    }
    fn start_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        id
    }
    fn response(&mut self, id: u64) -> Value {
        loop {
            let message = self
                .messages
                .recv_timeout(Duration::from_secs(10))
                .expect("MCP response timeout");
            if message.get("id").is_some() {
                assert_eq!(message["id"], id, "Unexpected response: {message}");
                return message;
            }
        }
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.start_request(method, params);
        self.response(id)
    }
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name":name,"arguments":arguments}));
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }
    fn data(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments);
        assert_ne!(result["isError"], true, "{result}");
        result["structuredContent"].clone()
    }
    fn finish(&mut self) {
        self.input.take();
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "MCP failed: {status}");
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "MCP server did not exit on stdin EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Ok(message) = self.messages.try_recv() {
            panic!("Unexpected trailing stdout: {message}");
        }
    }
}

#[cfg(all(unix, feature = "rendering"))]
#[test]
fn mcp_invalidates_icon_calculations_after_an_installation_path_is_retargeted() {
    use std::{os::unix::fs::symlink, path::Path};
    let (temp, file, data) = fixture();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icons/feather-icons-4.29.2.tgz");
    let setup = lazypencil::icons::setup(&data, "feather", Some(&source)).unwrap();
    let requested = Path::new(setup["path"].as_str().unwrap()).join("icons.json");
    let original = temp.path().join("original-icons.json");
    let replacement = temp.path().join("replacement-icons.json");
    fs::copy(&requested, &original).unwrap();
    fs::copy(&requested, &replacement).unwrap();
    fs::remove_file(&requested).unwrap();
    symlink(&original, &requested).unwrap();
    fs::write(&file, json!({"version":"2.21","children":[{"id":"icon","type":"icon","library":"feather","icon":"circle","width":24,"height":24,"fill":"#123456"}]}).to_string()).unwrap();
    let saved = fs::read(&file).unwrap();
    let mut client = Client::start(&file, &data);
    let before = client.data("get_document", json!({}));
    let calculation = client.data("get_layout", json!({"expected_revision":before["revision"],"expected_dependency_revision":before["dependency_revision"]}));
    fs::remove_file(&requested).unwrap();
    symlink(&replacement, &requested).unwrap();
    let stale = client.call("render", json!({"snapshot_id":calculation["snapshot_id"],"expected_revision":calculation["revision"],"expected_dependency_revision":calculation["dependency_revision"]}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    assert!(
        stale["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "image")
    );
    let after = client.data("get_document", json!({}));
    assert_eq!(after["revision"], before["revision"]);
    assert_ne!(after["dependency_revision"], before["dependency_revision"]);
    let recalculated = client.data("get_layout", json!({"expected_revision":after["revision"],"expected_dependency_revision":after["dependency_revision"]}));
    let rendered = client.call("render", json!({"snapshot_id":recalculated["snapshot_id"],"expected_revision":recalculated["revision"],"expected_dependency_revision":recalculated["dependency_revision"]}));
    assert_ne!(rendered["isError"], true);
    assert!(
        rendered["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "image")
    );
    assert_eq!(fs::read(&file).unwrap(), saved);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_script_limits_follow_captured_configuration_and_invalidate_old_calculations() {
    let (temp, file, data) = fixture();
    let user = temp.path().join("runtime-user");
    fs::create_dir(&user).unwrap();
    fs::write(
        user.join("config.toml"),
        "[runtime.javascript]\ntimeout_ms=100\nheap_mib=12\nmax_passes=6\n",
    )
    .unwrap();
    let configuration = temp.path().join("lazypencil.toml");
    fs::write(
        &configuration,
        "[runtime.javascript]\ntimeout_ms=2000\nmax_passes=4\n",
    )
    .unwrap();
    let code = temp.path().join("draw.js");
    fs::write(
        &code,
        "/** @schema 2.21 */ return [{type:'rectangle',width:20,height:10,fill:'#FF0000'}];",
    )
    .unwrap();
    let saved = json!({"version":"2.21","children":[{"id":"chart","type":"script","width":80,"height":40,"scriptUri":"draw.js"}]}).to_string();
    fs::write(&file, &saved).unwrap();
    let mut client = Client::start_with_user_config(&file, &data, None, &[], None, Some(&user));
    let current_arguments = |client: &mut Client| {
        let document = client.data("get_document", json!({}));
        json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]})
    };
    let arguments = current_arguments(&mut client);
    let before = client.data("get_layout", arguments);
    let limits = &before["script_executions"][0]["limits"];
    assert_eq!(limits["timeout_ms"], 2000);
    assert_eq!(limits["heap_mib"], 12);
    assert_eq!(limits["max_passes"], 4);
    fs::write(
        user.join("config.toml"),
        "[runtime.javascript]\ntimeout_ms=100\nheap_mib=16\nmax_passes=6\n",
    )
    .unwrap();
    let stale = client.call("render", json!({"snapshot_id":before["snapshot_id"],"expected_revision":before["revision"],"expected_dependency_revision":before["dependency_revision"]}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let arguments = current_arguments(&mut client);
    let after = client.data("get_layout", arguments);
    assert_eq!(after["script_executions"][0]["limits"]["heap_mib"], 16);
    assert_ne!(after["snapshot_id"], before["snapshot_id"]);
    assert_ne!(after["dependency_revision"], before["dependency_revision"]);
    fs::write(&code, "/** @schema 2.21 */ for (;;) {} ").unwrap();
    fs::write(&configuration, "[runtime.javascript]\ntimeout_ms=50\n").unwrap();
    let arguments = current_arguments(&mut client);
    let timeout = client.call("get_layout", arguments);
    assert_eq!(
        timeout["structuredContent"]["error"]["code"],
        "RENDER_TIMEOUT"
    );
    fs::write(
        &code,
        "/** @schema 2.21 */ return [{type:'rectangle',width:20,height:10,fill:'#FF0000'}];",
    )
    .unwrap();
    fs::write(&configuration, "[runtime.javascript]\ntimeout_ms=2000\n").unwrap();
    let arguments = current_arguments(&mut client);
    let recovered = client.data("get_layout", arguments);
    assert_eq!(
        recovered["script_executions"][0]["limits"]["timeout_ms"],
        2000
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), saved);
    client.finish();
}

#[test]
fn mcp_captures_user_settings_and_rejects_edits_from_stale_dependencies() {
    let (temp, file, data) = fixture();
    let user = temp.path().join("user settings");
    fs::create_dir(&user).unwrap();
    let config = user.join("config.toml");
    fs::write(&config, "[fonts]\nlocale='en-US'\n").unwrap();
    let saved = fs::read(&file).unwrap();
    let mut client = Client::start_with_user_config(&file, &data, None, &[], None, Some(&user));
    let before = client.data("get_document", json!({}));
    assert_eq!(
        before["configuration"]["settings"]["fonts"]["locale"],
        "en-US"
    );
    assert_eq!(before["configuration"]["font_origins"]["locale"], "user");
    let canonical = lazypencil::paths::canonical(&config).unwrap();
    assert!(
        before["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["path"] == json!(canonical) && item["kind"] == "configuration")
    );
    #[cfg(feature = "rendering")]
    let calculation = client.data("get_layout", json!({"expected_revision":before["revision"],"expected_dependency_revision":before["dependency_revision"]}));
    fs::write(&config, "[fonts]\nlocale='ja'\n").unwrap();
    let rejected = client.call(
        "apply_operations",
        json!({
            "request_id":"stale-user-settings", "expected_revision":before["revision"],
            "expected_dependency_revision":before["dependency_revision"],
            "operations":[{"op":"insert","node":{"type":"rectangle","width":20,"height":10}}]
        }),
    );
    assert_eq!(
        rejected["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    #[cfg(feature = "rendering")]
    {
        let stale = client.call("render", json!({"snapshot_id":calculation["snapshot_id"],"expected_revision":calculation["revision"],"expected_dependency_revision":calculation["dependency_revision"]}));
        assert_eq!(
            stale["structuredContent"]["error"]["code"],
            "REVISION_CONFLICT"
        );
    }
    let after = client.data("get_document", json!({}));
    assert_eq!(after["revision"], before["revision"]);
    assert_ne!(after["dependency_revision"], before["dependency_revision"]);
    assert_eq!(after["configuration"]["settings"]["fonts"]["locale"], "ja");
    assert_eq!(fs::read(&file).unwrap(), saved);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_uses_user_fonts_outside_the_document_root_without_cli_flags() {
    let (temp, file, data) = fixture();
    let user = temp.path().join("user settings");
    fs::create_dir_all(user.join("fonts")).unwrap();
    let font = user.join("fonts/faces.ttc");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/faces.ttc"),
        &font,
    )
    .unwrap();
    fs::write(
        user.join("config.toml"),
        "[fonts]\nfiles=['fonts/faces.ttc']\ndefault_family='LP Test Sans'\nlocale='en-US'\n",
    )
    .unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join(".git")).unwrap();
    let nested = project.join("design.pen");
    fs::rename(&file, &nested).unwrap();
    fs::write(&nested, json!({"version":"2.21","children":[{"id":"text","type":"text","content":"A","fontSize":32,"fill":"#111111"}]}).to_string()).unwrap();
    let mut client = Client::start_with_user_config(&nested, &data, None, &[], None, Some(&user));
    let document = client.data("get_document", json!({}));
    assert_eq!(
        document["configuration"]["root"],
        json!(lazypencil::paths::canonical(&project).unwrap())
    );
    let fonts = client.data("list_fonts", json!({}));
    assert_eq!(fonts["default_family"], "LP Test Sans");
    let layout = client.data("get_layout", json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
    assert_eq!(
        layout["nodes"][0]["text"]["fonts"][0]["hash"],
        lazypencil::document::hash(&fs::read(&font).unwrap())
    );
    assert_eq!(layout["nodes"][0]["text"]["locale"], "en-US");
    let rendered = client.call("render", json!({"snapshot_id":layout["snapshot_id"],"expected_revision":layout["revision"],"expected_dependency_revision":layout["dependency_revision"]}));
    assert_ne!(rendered["isError"], true, "{rendered}");
    assert!(
        rendered["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "image")
    );
    client.finish();
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fixture() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("design.pen");
    let data = temp.path().join("data");
    lazypencil::store::Store::init(&file, Some(data.clone())).unwrap();
    (temp, file, data)
}

#[test]
fn owned_mcp_records_exact_design_resource_metadata_and_rotates_active_turns() {
    use lazypencil::{guidance::usage::UsageGrant, store::Store};
    let (_temp, file, data) = fixture();
    let store = Store::open(&file, Some(data.clone())).unwrap();
    let conversation = uuid::Uuid::new_v4().to_string();
    let directory = store
        .document_data_dir()
        .join("conversations")
        .join(&conversation);
    lazypencil::store::create_private_directory(&directory).unwrap();
    let mut grant = UsageGrant::create(&store, &directory, &conversation).unwrap();
    let turn = uuid::Uuid::new_v4().to_string();
    grant.begin_turn(&turn).unwrap();
    let mut client = Client::start_with_context(&file, &data, None, &[], Some(grant.path()));
    let guide = client.data("get_guidance", json!({"id":"typography"}));
    let style = client.data("get_style", json!({"id":"standard-ui"}));
    let catalog = client.data("list_styles", json!({"limit":1}));
    assert_eq!(
        client.call("get_guidance", json!({"id":"unknown-guide"}))["isError"],
        true
    );
    let records = grant.finish_turn().unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["resources"][0]["hash"], guide["hash"]);
    assert_eq!(records[0]["resources"][0]["id"], "typography");
    assert_eq!(records[1]["resources"][0]["hash"], style["hash"]);
    assert_eq!(records[2]["resources"][0]["kind"], "style_catalog");
    assert_eq!(
        records[2]["resources"][0]["hash"],
        lazypencil::document::hash(catalog.to_string().as_bytes())
    );
    let metadata = serde_json::to_string(&records).unwrap();
    assert!(!metadata.contains("body"));
    assert!(!metadata.contains("document"));
    for record in &records {
        assert_eq!(record["turn"], turn);
    }
    assert_eq!(client.call("get_guidance", json!({}))["isError"], true);
    assert!(client.data("get_document", json!({}))["revision"].is_string());
    let next = uuid::Uuid::new_v4().to_string();
    grant.begin_turn(&next).unwrap();
    client.data("get_guidance", json!({"id":"color"}));
    let records = grant.finish_turn().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["turn"], next);
    assert_eq!(records[0]["resources"][0]["id"], "color");
    drop(grant);
    assert_eq!(
        client.call("get_style", json!({"id":"standard-ui"}))["isError"],
        true
    );
    client.finish();
    let mut standalone = Client::start(&file, &data);
    assert_eq!(
        standalone.data("get_guidance", json!({"id":"color"})),
        lazypencil::guidance::get_guidance(Some("color")).unwrap()
    );
    standalone.finish();
}

#[test]
fn project_design_catalog_uses_an_explicit_cli_grant_and_imports_through_the_bound_mcp_store() {
    let (temp, file, data) = fixture();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::write(
        temp.path().join("rules.txt"),
        "MCP_PROJECT_RULE: 日本語を維持する。",
    )
    .unwrap();
    fs::write(temp.path().join("components.pen"),json!({"version":"2.21","children":[{"id":"button","type":"frame","reusable":true,"width":40,"height":20,"fill":"#134E4A"}]}).to_string()).unwrap();
    fs::write(temp.path().join("brand.json"),json!({"id":"brand","name":"Brand","description":"Project style","tokens":{},"instructions":"Use the brand color.","example":"components.pen","use_cases":["dashboard"]}).to_string()).unwrap();
    fs::write(temp.path().join("lazypencil.toml"),"[design]\nstyles=['brand.json']\ndefault_style='project:brand'\n[[design.guides]]\nid='brand'\npath='rules.txt'\nlanguage='ja'\n").unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    assert_eq!(document["guidance"]["project_design"]["trusted"], false);
    assert!(!document.to_string().contains("MCP_PROJECT_RULE"));
    let rejected = client.call("get_guidance", json!({}));
    assert_eq!(
        rejected["structuredContent"]["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    let granted = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(&data)
        .args(["config", "trust"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        granted.status.success(),
        "{}",
        String::from_utf8_lossy(&granted.stdout)
    );
    let guidance = client.data("get_guidance", json!({}));
    assert_eq!(guidance["project_guides"][0]["language"], "ja");
    assert!(
        guidance["project_guides"][0]["body"]
            .as_str()
            .unwrap()
            .contains("MCP_PROJECT_RULE")
    );
    let style = client.data("get_style", json!({"id":"project:brand"}));
    assert_eq!(style["example"]["components"][0]["node_id"], "button");
    let applied=client.data("apply_operations",json!({"request_id":"mcp-project-style","expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"operations":[
        {"op":"import_component","source":{"style_id":"project:brand","node_id":"button"},"mode":"copy","client_ref":"primary"}
    ]}));
    let nodes = client.data(
        "read_nodes",
        json!({"node_id":applied["bindings"]["primary"],"expand_instances":true}),
    );
    assert_eq!(nodes["nodes"][0]["properties"]["fill"], "#134E4A");
    fs::write(temp.path().join("rules.txt"), "Changed instructions").unwrap();
    let rejected = client.call("get_guidance", json!({}));
    assert_eq!(
        rejected["structuredContent"]["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    client.finish();
}

#[test]
fn mcp_can_diagnose_and_repair_missing_assets_without_executing_missing_code() {
    let (_temp, file, data) = fixture();
    fs::write(&file,json!({"version":"2.21","children":[
        {"id":"image","type":"rectangle","width":10,"height":10,"fill":{"type":"image","url":"missing.png"}},
        {"id":"code","type":"script","width":10,"height":10,"scriptUri":"missing.js"}
    ]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let metadata = client.data("get_document", json!({}));
    assert_eq!(metadata["assets_complete"], false);
    assert_eq!(metadata["asset_diagnostics"].as_array().unwrap().len(), 2);
    let nodes = client.data("read_nodes", json!({"node_id":"image"}));
    assert_eq!(nodes["assets_complete"], false);
    #[cfg(feature = "rendering")]
    {
        let unavailable=client.call("get_layout",json!({"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"]}));
        assert_eq!(
            unavailable["structuredContent"]["error"]["code"],
            "ASSET_NOT_FOUND"
        );
    }
    let partial=client.data("apply_operations",json!({"request_id":"remove-missing-code","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":[{"op":"delete","target":"code"}]}));
    assert_eq!(partial["assets_complete"], false);
    assert_eq!(partial["asset_diagnostics"].as_array().unwrap().len(), 1);
    #[cfg(feature = "rendering")]
    {
        let asset=client.data("put_asset",json!({"kind":"image","source":{"data":"iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4/5/hPwgzwBgAbaQL9REU6k8AAAAASUVORK5CYII=","mime_type":"image/png"}}));
        let repaired=client.data("apply_operations",json!({"request_id":"replace-missing-image","expected_revision":partial["revision"],"expected_dependency_revision":partial["dependency_revision"],"operations":[{"op":"update","target":"image","set":{"fill":{"type":"image","url":asset["uri"]}}}]}));
        assert_eq!(repaired["assets_complete"], true);
        let rendered=client.call("render",json!({"expected_revision":repaired["revision"],"expected_dependency_revision":repaired["dependency_revision"]}));
        assert_ne!(rendered["isError"], true, "{rendered}");
        assert!(
            rendered["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|content| content["type"] == "image")
        );
    }
    #[cfg(not(feature = "rendering"))]
    {
        let repaired=client.data("apply_operations",json!({"request_id":"remove-missing-image","expected_revision":partial["revision"],"expected_dependency_revision":partial["dependency_revision"],"operations":[{"op":"delete","target":"image"}]}));
        assert_eq!(repaired["assets_complete"], true);
    }
    client.finish();
}

#[test]
fn mcp_variable_queries_expose_themed_values_sources_and_bound_pages() {
    let (temp, file, data) = fixture();
    fs::write(temp.path().join("tokens.pen"),json!({"version":"2.21","themes":{"mode":["light","dark"]},
        "variables":{"ink":{"type":"color","value":[{"value":"#FFFFFF"},{"theme":{"mode":"dark"},"value":"#111111"}]},"gap":{"type":"number","value":8}},
        "children":[]}).to_string()).unwrap();
    fs::write(
        &file,
        json!({"version":"2.21","imports":{"ui":"tokens.pen"},"themes":{"mode":["light","dark"]},
        "variables":{"ink":{"type":"color","value":"$ui:ink"}},"children":[]})
        .to_string(),
    )
    .unwrap();
    let mut client = Client::start(&file, &data);
    let tools = client.request("tools/list", json!({}));
    let tool = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "get_variables")
        .unwrap();
    assert!(tool["inputSchema"]["properties"]["theme"].is_object());
    let result = client.data("get_variables", json!({"theme":{"mode":"dark"}}));
    assert_eq!(result["variables"]["ink"]["value"], "$ui:ink");
    assert_eq!(result["resolved_values"]["ink"], "#111111");
    assert_eq!(
        result["sources"]["ink"]["value"],
        json!(lazypencil::paths::canonical(temp.path().join("tokens.pen")).unwrap())
    );
    let page = client.data("get_variables", json!({"library_alias":"ui","limit":1}));
    let next = client.data(
        "get_variables",
        json!({"library_alias":"ui","cursor":page["next_cursor"],"limit":500}),
    );
    assert_eq!(next["variables"].as_object().unwrap().len(), 1);
    let stale = client.call(
        "get_variables",
        json!({"library_alias":"ui","cursor":page["next_cursor"],"theme":{"mode":"dark"}}),
    );
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    #[cfg(not(feature = "rendering"))]
    {
        let unsupported = client.call("validate_document", json!({"scope":"layout"}));
        assert_eq!(
            unsupported["structuredContent"]["error"]["code"],
            "UNSUPPORTED_FEATURE"
        );
    }
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_layout_validation_retains_one_calculation_for_diagnostics_geometry_and_png() {
    use base64::Engine;
    let (temp, file, data) = fixture();
    fs::write(
        temp.path().join("draw.js"),
        "/**\n * @schema 2.21\n */\nreturn [{type:'rectangle',width:20,height:20,fill:'#2563EB'}];",
    )
    .unwrap();
    fs::write(&file,json!({"version":"2.21","children":[{"id":"code","type":"script","width":20,"height":20,"scriptUri":"draw.js"},
        {"id":"frame","type":"frame","x":30,"width":10,"height":10,"children":[{"id":"outside","type":"rectangle","width":20,"height":20}]}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let validation = client.data("validate_document", json!({"scope":"layout","limit":1}));
    assert_eq!(validation["valid"], true);
    assert_eq!(validation["script_executions"].as_array().unwrap().len(), 1);
    assert!(validation["warning_count"].as_u64().unwrap() > 0);
    let arguments = json!({"expected_revision":validation["revision"],"expected_dependency_revision":validation["dependency_revision"],"snapshot_id":validation["snapshot_id"]});
    let layout = client.data("get_layout", arguments.clone());
    assert_eq!(layout["snapshot_id"], validation["snapshot_id"]);
    assert_eq!(
        layout["runtime_dependency_revision"],
        validation["runtime_dependency_revision"]
    );
    let mut repeat = arguments.clone();
    repeat["scope"] = json!("layout");
    let checked = client.data("validate_document", repeat.clone());
    assert_eq!(checked["snapshot_id"], validation["snapshot_id"]);
    let rendered = client.call("render", arguments.clone());
    assert_ne!(rendered["isError"], true, "{rendered}");
    let image = rendered["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "image")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    let raster = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(
        raster.peek_pixels().unwrap().get_color((10, 10)),
        skia_safe::Color::from_rgb(0x25, 0x63, 0xeb)
    );
    fs::write(
        temp.path().join("draw.js"),
        "/**\n * @schema 2.21\n */\nreturn [];",
    )
    .unwrap();
    let stale = client.call("validate_document", repeat);
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let missing = client.call(
        "validate_document",
        json!({"scope":"layout","snapshot_id":"expired-or-unknown"}),
    );
    assert_eq!(
        missing["structuredContent"]["error"]["code"],
        "SNAPSHOT_UNAVAILABLE"
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_shadow_blends_follow_instance_themes_and_return_the_actual_composited_pixels() {
    use base64::Engine;
    let (_temp, file, data) = fixture();
    fs::write(&file,json!({"version":"2.21","themes":{"appearance":["light","dark"]},
        "variables":{"shadow.mode":{"type":"string","value":[{"value":"multiply"},{"theme":{"appearance":"dark"},"value":"screen"}]}},
        "children":[{"id":"definitions","type":"frame","enabled":false,"children":[{"id":"tile","type":"rectangle","reusable":true,"width":8,"height":8,"fill":"#203040",
            "effect":{"type":"shadow","offset":{"x":8,"y":0},"blur":0,"color":"#FF000080","blendMode":"$shadow.mode"}}]},
            {"id":"back","type":"rectangle","width":40,"height":20,"fill":"#0000FF80"},
            {"id":"use","type":"ref","ref":"tile","x":8,"y":4}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let metadata = client.data("get_document", json!({}));
    assert_eq!(
        metadata["capabilities"]["graphics"]["outer_shadow_blending"],
        true
    );
    assert_eq!(
        metadata["capabilities"]["graphics"]["blend_modes"]
            .as_array()
            .unwrap()
            .len(),
        18
    );
    let mut arguments = json!({"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"]});
    for (mode, expected) in [("multiply", 85_i32), ("screen", 170_i32)] {
        let values = client.data(
            "get_variables",
            json!({"name":"shadow.mode","node_id":"use"}),
        );
        assert_eq!(values["resolved_values"]["shadow.mode"], mode);
        let layout = client.data("get_layout", arguments.clone());
        let mut render_args = arguments.clone();
        render_args["snapshot_id"] = layout["snapshot_id"].clone();
        let output = client.call("render", render_args);
        assert_ne!(output["isError"], true, "{output}");
        let image = output["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["type"] == "image")
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().unwrap())
            .unwrap();
        let raster = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let pixel = raster.peek_pixels().unwrap().get_color((18, 8));
        assert!((pixel.a() as i32 - 192).abs() <= 1, "{mode}: {pixel:?}");
        assert!(
            (pixel.r() as i32 - expected).abs() <= 2,
            "{mode}: {pixel:?}"
        );
        assert!(
            (pixel.b() as i32 - expected).abs() <= 2,
            "{mode}: {pixel:?}"
        );
        if mode == "multiply" {
            let saved=client.data("apply_operations",json!({"request_id":"change-shadow-theme","expected_revision":arguments["expected_revision"],"expected_dependency_revision":arguments["expected_dependency_revision"],
                "operations":[{"op":"set_theme","target":"use","set":{"appearance":"dark"}}]}));
            arguments["expected_revision"] = saved["revision"].clone();
            arguments["expected_dependency_revision"] = saved["dependency_revision"].clone();
        }
    }
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_mesh_gradients_resolve_external_themes_and_render_the_retained_calculation() {
    use base64::Engine;
    let (temp, file, data) = fixture();
    fs::create_dir(temp.path().join("library")).unwrap();
    let library = temp.path().join("library/ui.lib.pen");
    let source=json!({"version":"2.21","themes":{"appearance":["light","dark"]},"variables":{
        "accent":{"type":"color","value":[{"value":"#FF000080"},{"value":"#00FF0080","theme":{"appearance":"dark"}}]}},
        "children":[{"id":"tile","type":"rectangle","reusable":true,"width":32,"height":16,
            "fill":{"type":"mesh_gradient","columns":2.0,"rows":2,"colors":["$accent","#0000FF80","$accent","#0000FF80"],"points":[[0,0],{"position":[1,0]},[0,1],[1,1]]}}]}).to_string();
    fs::write(&library, &source).unwrap();
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},"themes":{"appearance":["light","dark"]},
        "children":[{"id":"back","type":"rectangle","width":40,"height":24,"fill":"#20202080"},{"id":"use","type":"ref","ref":"ui:tile","x":4,"y":4}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let metadata = client.data("get_document", json!({}));
    assert_eq!(metadata["capabilities"]["graphics"]["mesh_gradient"], true);
    let mut conditions = json!({"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"]});
    for green in [false, true] {
        let calculation = client.data("get_layout", conditions.clone());
        let mut arguments = conditions.clone();
        arguments["snapshot_id"] = calculation["snapshot_id"].clone();
        let output = client.call("render", arguments);
        assert_ne!(output["isError"], true, "{output}");
        let block = output["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["type"] == "image")
            .unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(block["data"].as_str().unwrap())
            .unwrap();
        let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let pixel = image.peek_pixels().unwrap().get_color((12, 8));
        let a = 128.0 / 255.0;
        let alpha = a + a * (1.0 - a);
        assert!((pixel.a() as f64 - alpha * 255.0).abs() <= 1.0);
        let u = 8.5 / 32.0;
        let expected = if green {
            [0.0, 255.0 * (1.0 - u), 255.0 * u]
        } else {
            [255.0 * (1.0 - u), 0.0, 255.0 * u]
        };
        for (actual, source) in [pixel.r(), pixel.g(), pixel.b()].into_iter().zip(expected) {
            let premul = source * a + 32.0 * a * (1.0 - a);
            assert!(
                (actual as f64 * pixel.a() as f64 / 255.0 - premul).abs() <= 2.0,
                "{pixel:?}"
            );
        }
        if !green {
            let saved=client.data("apply_operations",json!({"request_id":"mesh-theme","expected_revision":conditions["expected_revision"],"expected_dependency_revision":conditions["expected_dependency_revision"],
                "operations":[{"op":"set_theme","target":"use","set":{"appearance":"dark"}}]}));
            conditions["expected_revision"] = saved["revision"].clone();
            conditions["expected_dependency_revision"] = saved["dependency_revision"].clone();
        }
    }
    client.finish();
    assert_eq!(fs::read_to_string(library).unwrap(), source);
}

#[test]
fn mcp_sampler_variables_keep_the_source_image_dependency_through_detachment() {
    use base64::Engine;
    let (temp, file, data) = fixture();
    fs::create_dir(temp.path().join("library")).unwrap();
    let png=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4/5/hPwgzwBgAbaQL9REU6k8AAAAASUVORK5CYII=").unwrap();
    fs::write(temp.path().join("library/image.png"), &png).unwrap();
    fs::write(temp.path().join("image.png"), &png).unwrap();
    fs::write(temp.path().join("library/sample.glsl"),"precision highp float; uniform sampler2D image; void main(){gl_FragColor=texture2D(image,vec2(0.5));}").unwrap();
    fs::write(temp.path().join("library/ui.lib.pen"),json!({"version":"2.21","variables":{
        "picture":{"type":"string","value":"image.png"},"alias":{"type":"string","value":"$picture"}},
        "children":[{"id":"tile","type":"rectangle","reusable":true,"width":10,"height":10,
            "fill":{"type":"shader","url":"sample.glsl","uniforms":{"image":"$alias"}}}]}).to_string()).unwrap();
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},"children":[{"id":"use","type":"ref","ref":"ui:tile"}]}).to_string()).unwrap();
    let expected = temp
        .path()
        .join("library/image.png")
        .canonicalize()
        .unwrap();
    let mut client = Client::start(&file, &data);
    let before = client.data("get_document", json!({}));
    assert_eq!(
        before["capabilities"]["shader"]["variable_image_origins"],
        true
    );
    let image_paths = |value: &Value| {
        value["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["kind"] == "image")
            .map(|entry| entry["path"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(image_paths(&before), vec![json!(expected)]);
    let saved=client.data("apply_operations",json!({"request_id":"detach-sampler","expected_revision":before["revision"],"expected_dependency_revision":before["dependency_revision"],"operations":[{"op":"detach_instance","target":"use"}]}));
    assert_eq!(image_paths(&saved), vec![json!(expected)]);
    let node = client.data("read_nodes", json!({"node_id":"use","depth":0}));
    assert_eq!(node["nodes"][0]["fill"]["uniforms"]["image"], "$ui:alias");
    #[cfg(feature = "rendering")]
    {
        let layout=client.data("get_layout",json!({"expected_revision":saved["revision"],"expected_dependency_revision":saved["dependency_revision"]}));
        assert_eq!(image_paths(&layout), vec![json!(expected)]);
    }
    client.finish();
}

#[test]
fn mcp_copies_a_component_and_its_image_then_works_without_the_source_library() {
    use base64::Engine;
    let (temp, file, data) = fixture();
    let png=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4/5/hPwgzwBgAbaQL9REU6k8AAAAASUVORK5CYII=").unwrap();
    fs::write(temp.path().join("source.png"), png).unwrap();
    let library = temp.path().join("ui.lib.pen");
    fs::write(&library,json!({"version":"2.21","children":[{"id":"tile","type":"rectangle","reusable":true,"width":20,"height":20,"fill":{"type":"image","url":"source.png"}}]}).to_string()).unwrap();
    fs::write(
        &file,
        json!({"version":"2.21","imports":{"ui":"ui.lib.pen"},"children":[]}).to_string(),
    )
    .unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    assert_eq!(
        document["capabilities"]["import_component"]["modes"],
        json!(["link", "copy"])
    );
    let mut request = json!({"request_id":"mcp-import-preview","expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"dry_run":true,
        "operations":[{"op":"import_component","source":{"library_alias":"ui","node_id":"tile"},"mode":"copy","client_ref":"tile"},{"op":"remove_import","alias":"ui"}]});
    let preview = client.data("apply_operations", request.clone());
    assert_eq!(preview["dry_run"], true);
    assert!(!temp.path().join("assets").exists());
    request["request_id"] = json!("mcp-import");
    request["dry_run"] = json!(false);
    let result = client.data("apply_operations", request.clone());
    let component = client.data(
        "read_nodes",
        json!({"node_id":result["generated_ids"]["0"]["component_id"]}),
    );
    assert!(
        component["nodes"][0]["fill"]["url"]
            .as_str()
            .unwrap()
            .starts_with("assets/lazypencil/")
    );
    fs::remove_file(library).unwrap();
    fs::remove_file(temp.path().join("source.png")).unwrap();
    let replay = client.data("apply_operations", request);
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["generated_ids"], result["generated_ids"]);
    #[cfg(feature = "rendering")]
    {
        let document = client.data("get_document", json!({}));
        let render=client.call("render",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
        assert_ne!(render["isError"], true, "{render}");
        let content = render["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "image")
            .unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(content["data"].as_str().unwrap())
            .unwrap();
        let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!(image.dimensions(), skia_safe::ISize::new(20, 20));
        assert_eq!(
            image.peek_pixels().unwrap().get_color((10, 10)),
            skia_safe::Color::YELLOW
        );
    }
    client.finish();
}

#[test]
fn mcp_detaches_inherited_external_instances_and_replays_the_saved_result() {
    let (temp, file, data) = fixture();
    let library = temp.path().join("ui.lib.pen");
    fs::write(&library,json!({"version":"2.21","variables":{"ink":{"type":"color","value":"#ff0000"}},"children":[
        {"id":"dot","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"$ink"},
        {"id":"card","type":"frame","reusable":true,"width":20,"height":20,"children":[{"id":"action","type":"ref","ref":"dot"}]}
    ]}).to_string()).unwrap();
    let library_bytes = fs::read(&library).unwrap();
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"ui.lib.pen"},"children":[{"id":"use","type":"ref","ref":"ui:card"}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    assert_eq!(
        document["capabilities"]["detach_instance"]["inherited"],
        true
    );
    assert_eq!(
        document["capabilities"]["detach_instance"]["external"],
        true
    );
    let request = json!({"request_id":"mcp-external-detach","expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],
        "operations":[{"op":"detach_instance","target":{"node_id":"use","instance_path":["action"]},"client_ref":"dot"}]});
    let result = client.data("apply_operations", request.clone());
    let read = client.data("read_nodes", json!({"node_id":result["bindings"]["dot"]}));
    assert_eq!(read["nodes"][0]["type"], "rectangle");
    assert_eq!(read["nodes"][0]["fill"], "$ui:ink");
    let replay = client.data("apply_operations", request);
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["revision"], result["revision"]);
    assert_eq!(replay["generated_ids"], result["generated_ids"]);
    assert_eq!(fs::read(&library).unwrap(), library_bytes);
    #[cfg(feature = "rendering")]
    {
        let document = client.data("get_document", json!({}));
        let render=client.call("render",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
        assert_ne!(render["isError"], true, "{render}");
        assert!(
            render["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "image")
        );
    }
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_uses_project_font_configuration_without_cli_flags_and_rejects_stale_settings() {
    let (temp, file, data) = fixture();
    let fonts = temp.path().join("fonts");
    fs::create_dir(&fonts).unwrap();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/faces.ttc"),
        fonts.join("faces.ttc"),
    )
    .unwrap();
    let configuration = temp.path().join("lazypencil.toml");
    fs::write(&configuration,"version=1\n[fonts]\nfiles=['fonts/faces.ttc']\ndefault_family='LP Test Sans'\nlocale='en-US'\nfallbacks=['Noto Sans JP','LP Test Supplement']\n").unwrap();
    fs::write(&file,json!({"version":"2.21","children":[{"id":"text","type":"text","content":"A日\u{f0000}","fontSize":32,"fill":"#111111"}]}).to_string()).unwrap();
    let saved = fs::read(&file).unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    assert_eq!(
        document["configuration"]["settings"]["fonts"]["locale"],
        "en-US"
    );
    let catalogue = client.data("list_fonts", json!({}));
    assert_eq!(catalogue["fonts"].as_array().unwrap().len(), 5);
    assert_eq!(catalogue["default_family"], "LP Test Sans");
    assert_eq!(catalogue["locale"], "en-US");
    let mut arguments = json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]});
    let layout = client.data("get_layout", arguments.clone());
    assert_eq!(
        layout["nodes"][0]["text"]["fonts"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        layout["nodes"][0]["text"]["requested_family"],
        "LP Test Sans"
    );
    assert_eq!(layout["nodes"][0]["text"]["locale"], "en-US");
    arguments["snapshot_id"] = layout["snapshot_id"].clone();
    let rendered = client.call("render", arguments.clone());
    assert_ne!(rendered["isError"], true, "{rendered}");
    assert!(
        rendered["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|content| content["type"] == "image")
    );
    fs::write(
        &configuration,
        "[fonts]\nfiles=['fonts/faces.ttc']\ndefault_family='LP Test Sans'\nlocale='ja'\n",
    )
    .unwrap();
    let stale = client.call("render", arguments);
    assert_eq!(stale["isError"], true);
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let updated = client.data("get_document", json!({}));
    assert_ne!(
        updated["dependency_revision"],
        document["dependency_revision"]
    );
    assert_eq!(updated["revision"], document["revision"]);
    assert_eq!(fs::read(&file).unwrap(), saved);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_decorated_text_strokes_return_new_ink_from_the_retained_layout() {
    use base64::Engine;
    let (_temp, file, data) = fixture();
    fs::write(&file,json!({"version":"2.21","children":[{"id":"frame","type":"frame","layout":"none","width":240,"height":160,"children":[
        {"id":"label","type":"text","x":12,"y":12,"width":210,"height":120,"textGrowth":"fixed-width-height","content":"A A 日本語","fontSize":32,"fill":[],"stroke":"#0066FF","strokeWidth":2}
    ]}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let mut images = Vec::new();
    for decorated in [false, true] {
        let document = client.data("get_document", json!({}));
        let document = if decorated {
            client.data("apply_operations",json!({"request_id":"decorate-text","expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"operations":[{"op":"update","target":"label","set":{"underline":true,"strikethrough":true}}]}))
        } else {
            document
        };
        let layout=client.data("get_layout",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
        let response=client.call("render",json!({"expected_revision":layout["revision"],"expected_dependency_revision":layout["dependency_revision"],"snapshot_id":layout["snapshot_id"]}));
        assert_ne!(response["isError"], true, "{response}");
        let image = response["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "image")
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().unwrap())
            .unwrap();
        images.push(
            skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))
                .unwrap()
                .make_raster_image(None, None)
                .unwrap(),
        );
    }
    assert_eq!(images[0].dimensions(), images[1].dimensions());
    let before = images[0].peek_pixels().unwrap();
    let after = images[1].peek_pixels().unwrap();
    let mut added = 0;
    for y in 0..images[1].height() {
        for x in 0..images[1].width() {
            if before.get_color((x, y)).a() == 0 && after.get_color((x, y)).a() > 200 {
                added += 1;
                assert_eq!(after.get_color((x, y)).b(), 255);
            }
        }
    }
    assert!(
        added > 50,
        "Decorations must add actual visible stroke pixels"
    );
    let saved: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved["children"][0]["children"][0]["underline"], true);
    assert_eq!(saved["children"][0]["children"][0]["strikethrough"], true);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_font_collections_resolve_styles_fallback_pixels_and_dependency_changes() {
    use base64::Engine;
    let (temp, file, data) = fixture();
    let font = temp.path().join("faces.ttc");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/faces.ttc"),
        &font,
    )
    .unwrap();
    fs::write(&file,json!({"version":"2.21","children":[
        {"id":"mixed","type":"text","content":"A日\u{f0000}","fontFamily":"LP Test Sans","fontSize":48,"width":320,"height":70,"textGrowth":"fixed-width","fill":"#111111"},
        {"id":"bold","type":"text","content":"A","fontFamily":"LP Test Sans","fontSize":48,"fontWeight":"700","y":80,"width":320,"height":70,"textGrowth":"fixed-width","fill":"#111111"}
    ]}).to_string()).unwrap();
    let saved = fs::read(&file).unwrap();
    let mut client = Client::start_configured(&file, &data, None, std::slice::from_ref(&font));
    let catalogue = client.data("list_fonts", json!({}));
    assert_eq!(catalogue["fonts"].as_array().unwrap().len(), 5);
    assert_eq!(catalogue["fallback_families"][0], "Noto Sans JP");
    let arguments = json!({"expected_revision":catalogue["revision"],"expected_dependency_revision":catalogue["dependency_revision"]});
    let layout = client.data("get_layout", arguments.clone());
    let nodes = layout["nodes"].as_array().unwrap();
    let mixed = nodes.iter().find(|node| node["id"] == "mixed").unwrap();
    let families = mixed["text"]["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|font| font["family"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        families,
        vec!["LP Test Sans", "Noto Sans JP", "LP Test Supplement"]
    );
    let bold = nodes.iter().find(|node| node["id"] == "bold").unwrap();
    assert_eq!(bold["text"]["fonts"][0]["face_index"], 1);
    assert_eq!(bold["text"]["fonts"][0]["weight"], 700);
    assert_eq!(bold["text"]["fonts"][0]["synthetic_bold"], false);
    let mut render = arguments;
    render["snapshot_id"] = layout["snapshot_id"].clone();
    let output = client.call("render", render.clone());
    assert_ne!(output["isError"], true, "{output}");
    let content = output["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|content| content["type"] == "image")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content["data"].as_str().unwrap())
        .unwrap();
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let pixels = image.peek_pixels().unwrap();
    assert!(
        (0..image.height()).any(|y| (0..image.width()).any(|x| pixels.get_color((x, y)).a() > 0))
    );
    let mut changed = fs::read(&font).unwrap();
    changed.push(0);
    fs::write(&font, changed).unwrap();
    let stale = client.call("render", render);
    assert_eq!(stale["isError"], true);
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    assert_eq!(fs::read(&file).unwrap(), saved);
    client.finish();
}

#[test]
fn real_stdio_server_lists_typed_tools_and_edits_one_bound_document() {
    let (temp, file, data) = fixture();
    let outside = temp.path().join("outside.pen");
    fs::write(&outside, "untouched").unwrap();
    let mut client = Client::start(&file, &data);
    let tools = client.request("tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    let apply = tools
        .iter()
        .find(|t| t["name"] == "apply_operations")
        .unwrap();
    assert_eq!(apply["inputSchema"]["additionalProperties"], false);
    assert!(apply["inputSchema"]["properties"]["operations"].is_object());
    assert_eq!(apply["annotations"]["readOnlyHint"], false);
    assert!(
        tools
            .iter()
            .all(|t| t["inputSchema"]["properties"].get("file").is_none())
    );
    let before = client.data("get_document", json!({}));
    let guide = client.data("get_guidance", json!({}));
    assert_eq!(guide, lazypencil::guidance::get_guidance(None).unwrap());
    let styles = client.data("list_styles", json!({"use_case":"dashboard"}));
    assert_eq!(styles["styles"][0]["id"], "dense-workspace");
    let style = client.data("get_style", json!({"id":"dense-workspace"}));
    assert_eq!(
        style,
        lazypencil::guidance::get_style("dense-workspace").unwrap()
    );
    let request = json!({"request_id":"create","expected_revision":before["revision"],"expected_dependency_revision":before["dependency_revision"],
        "operations":[{"op":"insert","client_ref":"card","node":{"type":"frame","layout":"vertical","width":260,"height":120,
            "fill":"#FFFFFF","padding":16,"children":[{"type":"text","content":"日本語 MCP","fontSize":20}]}}]});
    let edited = client.data("apply_operations", request.clone());
    let card = edited["bindings"]["card"].as_str().unwrap();
    assert_ne!(edited["revision"], before["revision"]);
    assert_eq!(client.data("apply_operations", request)["replayed"], true);
    let nodes = client.data("read_nodes", json!({"node_id":card,"depth":1}));
    assert_eq!(nodes["nodes"][0]["children"][0]["content"], "日本語 MCP");
    let bad = client.call("get_document", json!({"file":outside}));
    assert_eq!(bad["isError"], true);
    assert_eq!(
        bad["structuredContent"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    assert_eq!(fs::read_to_string(outside).unwrap(), "untouched");
    assert_eq!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let changed = client.data(
        "apply_operations",
        json!({"request_id":"change","expected_revision":edited["revision"],
        "operations":[{"op":"update","target":card,"set":{"width":300}}]}),
    );
    let restored=client.data("restore_checkpoint",json!({"request_id":"restore","expected_revision":changed["revision"],"checkpoint_id":edited["checkpoint_id"]}));
    assert_eq!(
        client.data("read_nodes", json!({"node_id":card}))["nodes"][0]["width"],
        260
    );
    assert_eq!(restored["restored_checkpoint"], edited["checkpoint_id"]);
    let components = client.data("list_components", json!({}));
    assert_eq!(components["revision"], restored["revision"]);
    client.finish();
}

#[test]
fn code_assets_are_read_by_captured_hash_with_utf8_chunk_boundaries() {
    let (temp, file, data) = fixture();
    let script = temp.path().join("draw.js");
    let source = "// 日本語\nreturn [];";
    fs::write(&script, source).unwrap();
    fs::write(&file,json!({"version":"2.21","children":[{"id":"script","type":"script","width":100,"height":100,"scriptUri":"draw.js"}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    let hash = document["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "script")
        .unwrap()["revision"]
        .clone();
    let first = client.data("read_asset", json!({"hash":hash,"limit":6}));
    assert_eq!(first["content"], "// 日");
    assert_eq!(first["next_offset"], 6);
    let invalid = client.call("read_asset", json!({"hash":hash,"offset":4}));
    assert_eq!(
        invalid["structuredContent"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    let missing = client.call("read_asset", json!({"hash":"not-a-declared-asset"}));
    assert_eq!(
        missing["structuredContent"]["error"]["code"],
        "ASSET_NOT_FOUND"
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn real_stdio_server_reuses_layout_snapshots_and_returns_decodable_png_content() {
    use base64::Engine;
    let (_temp, file, data) = fixture();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/welcome.pen"),
        &file,
    )
    .unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    let mut args = json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"limit":1});
    let first = client.data("get_layout", args.clone());
    assert_eq!(first["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(first["total_matches"], 9);
    args["snapshot_id"] = first["snapshot_id"].clone();
    args["cursor"] = first["next_cursor"].clone();
    let second = client.data("get_layout", args);
    assert_eq!(second["snapshot_id"], first["snapshot_id"]);
    assert_ne!(second["nodes"][0]["id"], first["nodes"][0]["id"]);
    let rendered=client.call("render",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],
        "snapshot_id":first["snapshot_id"]}));
    assert_ne!(rendered["isError"], true, "{rendered}");
    let image = rendered["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "image")
        .unwrap();
    assert_eq!(image["mimeType"], "image/png");
    let png = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let decoded = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&png)).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (960, 640));
    assert_eq!(
        rendered["structuredContent"]["snapshot_id"],
        first["snapshot_id"]
    );
    let edited = client.data(
        "apply_operations",
        json!({"request_id":"update","expected_revision":document["revision"],
        "operations":[{"op":"update","target":"heading","set":{"content":"Changed"}}]}),
    );
    let stale = client.call(
        "render",
        json!({"expected_revision":document["revision"],"snapshot_id":first["snapshot_id"]}),
    );
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let missing = client.call(
        "render",
        json!({"expected_revision":edited["revision"],"snapshot_id":"not-retained"}),
    );
    assert_eq!(
        missing["structuredContent"]["error"]["code"],
        "SNAPSHOT_UNAVAILABLE"
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn mcp_discovers_icons_and_renders_variable_bound_icons_in_reused_components() {
    use base64::Engine;
    for (library, alias) in [("lucide", "home"), ("phosphor", "House")] {
        let (_temp, file, data) = fixture();
        if library == "phosphor" {
            lazypencil::icons::setup(
                &data,
                "phosphor",
                Some(
                    &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/icons/phosphor-core-2.1.1.tgz"),
                ),
            )
            .unwrap();
        }
        fs::write(&file,json!({"version":"2.21","variables":{"pack":{"type":"string","value":library},"symbol":{"type":"string","value":"circle"},"ink":{"type":"color","value":"#FF0000"}},"children":[{"id":"component","type":"frame","reusable":true,"width":48,"height":48,"layout":"none","children":[{"id":"symbol","type":"icon","library":"$pack","icon":"$symbol","width":48,"height":48,"fill":"$ink"}]},{"id":"instance","type":"ref","ref":"component","x":60,"descendants":{"symbol":{"fill":"#0000FF"}}}]}).to_string()).unwrap();
        let mut client = Client::start(&file, &data);
        let tools = client.request("tools/list", json!({}));
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "list_icons")
        );
        let listed = client.data("list_icons", json!({"pack":library,"name":alias}));
        assert!(
            listed["icons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|icon| icon["name"] == alias && icon["alias_of"] == "house")
        );
        let document = client.data("get_document", json!({}));
        assert_eq!(document["capabilities"]["icons"]["node_type"], "icon");
        let layout = client.data("get_layout", json!({"limit":20,"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
        let icons = layout["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["node_type"] == "icon")
            .collect::<Vec<_>>();
        assert_eq!(icons.len(), 2);
        assert!(
            icons
                .iter()
                .all(|node| node["icon"]["canonical_name"] == "circle")
        );
        let rendered=client.call("render",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"snapshot_id":layout["snapshot_id"]}));
        assert_ne!(rendered["isError"], true, "{rendered}");
        let image = rendered["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "image")
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().unwrap())
            .unwrap();
        let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes)).unwrap();
        let raster = image.make_raster_image(None, None).unwrap();
        let bitmap = raster.peek_pixels().unwrap();
        assert_eq!(bitmap.get_color((5, 24)), skia_safe::Color::RED);
        assert_eq!(bitmap.get_color((65, 24)), skia_safe::Color::BLUE);
        assert_eq!(bitmap.get_color((24, 24)).a(), 0);
        client.finish();
    }
}

#[cfg(feature = "rendering")]
#[test]
#[ignore = "Requires the original Outlined TTF in LAZYPENCIL_TEST_MATERIAL_DIR"]
fn real_mcp_material_weight_edits_return_pixels_and_invalidate_changed_font_snapshots() {
    use base64::Engine;
    let (_temp, file, data) = fixture();
    let source = std::path::PathBuf::from(
        std::env::var_os("LAZYPENCIL_TEST_MATERIAL_DIR").expect("Set LAZYPENCIL_TEST_MATERIAL_DIR"),
    )
    .join("MaterialSymbolsOutlined[FILL,GRAD,opsz,wght].ttf");
    let installed =
        lazypencil::icons::setup(&data, "Material Symbols Outlined", Some(&source)).unwrap();
    fs::write(&file,json!({"version":"2.21","variables":{"thickness":{"type":"number","value":100}},"children":[{"id":"symbol","type":"icon","library":"Material Symbols Outlined","icon":"home","weight":"$thickness","width":96,"height":96,"fill":"#123456"}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let list = client.data(
        "list_icons",
        json!({"pack":"Material Symbols Outlined","name":"home","limit":500}),
    );
    assert!(
        list["icons"].as_array().unwrap().iter().all(
            |icon| icon["variable_weight"] == true && icon["weight_range"] == json!([100, 700])
        )
    );
    let mut ink = Vec::new();
    let mut previous = None;
    for (iteration, weight) in [(0, 100), (1, 700)] {
        let mut document = client.data("get_document", json!({}));
        if iteration == 1 {
            client.data("apply_operations",json!({"request_id":"weight-edit","expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"operations":[{"op":"set_variable","name":"thickness","definition":{"type":"number","value":weight}}]}));
            document = client.data("get_document", json!({}));
            let stale = client.call(
                "render",
                json!({"expected_revision":document["revision"],"snapshot_id":previous}),
            );
            assert_eq!(
                stale["structuredContent"]["error"]["code"],
                "REVISION_CONFLICT"
            );
        }
        let layout=client.data("get_layout",json!({"limit":10,"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
        assert_eq!(
            layout["nodes"][0]["icon"]["weight"].as_f64(),
            Some(f64::from(weight))
        );
        let rendered=client.call("render",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"snapshot_id":layout["snapshot_id"]}));
        assert_ne!(rendered["isError"], true, "{rendered}");
        let block = rendered["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "image")
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(block["data"].as_str().unwrap())
            .unwrap();
        let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let pixels = image.peek_pixels().unwrap();
        let mut count = 0;
        for y in 0..96 {
            for x in 0..96 {
                count += usize::from(pixels.get_color((x, y)).a() > 128);
            }
        }
        ink.push(count);
        previous = layout["snapshot_id"].as_str().map(str::to_owned);
    }
    assert!(ink[1] > ink[0] * 2, "{ink:?}");
    let metadata = client.data("get_document", json!({}));
    let document = fs::read(&file).unwrap();
    let font = std::path::Path::new(installed["path"].as_str().unwrap()).join("font.ttf");
    let mut bytes = fs::read(&font).unwrap();
    bytes[0] ^= 1;
    fs::write(&font, bytes).unwrap();
    let stale = client.call(
        "render",
        json!({"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"snapshot_id":previous}),
    );
    assert_eq!(
        stale["structuredContent"]["error"]["code"], "REVISION_CONFLICT",
        "{stale}"
    );
    assert_eq!(fs::read(&file).unwrap(), document);
    client.finish();
}

#[test]
fn real_stdio_server_reports_stale_edits_without_mutating_the_document() {
    let (_temp, file, data) = fixture();
    let mut client = Client::start(&file, &data);
    let original = fs::read(&file).unwrap();
    let result = client.call(
        "apply_operations",
        json!({"request_id":"stale","expected_revision":"not-current",
        "operations":[{"op":"insert","node":{"type":"rectangle","width":20,"height":20}}]}),
    );
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    assert_eq!(fs::read(&file).unwrap(), original);
    assert!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.finish();
}

#[test]
fn cancelling_asset_publication_unblocks_the_server_without_releasing_dependency_readers() {
    let (temp, file, data) = fixture();
    let store = lazypencil::store::Store::open(&file, Some(data.clone())).unwrap();
    let mut client = Client::start(&file, &data);
    let before = client.data("get_document", json!({}));
    let mut cases = vec![
        (
            json!({"kind":"script","source":{"text":"return [];"}}),
            b"return [];".to_vec(),
            "js",
        ),
        (
            json!({"kind":"shader","source":{"text":"void main(){}"}}),
            b"void main(){}".to_vec(),
            "glsl",
        ),
    ];
    #[cfg(feature = "rendering")]
    {
        use base64::Engine;
        let encoded = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4/5/hPwgzwBgAbaQL9REU6k8AAAAASUVORK5CYII=";
        let raw = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let normalized = lazypencil::image_input::normalize(&raw, Some("image/png")).unwrap();
        cases.push((
            json!({"kind":"image","source":{"mime_type":"image/png","data":encoded}}),
            normalized.png,
            "png",
        ));
    }
    for (arguments, bytes, extension) in cases.drain(..) {
        let uri = format!(
            "assets/lazypencil/{}.{extension}",
            lazypencil::document::hash(&bytes)
        );
        let target = temp.path().join(&uri);
        let locks = store
            .lock_dependencies(std::slice::from_ref(&target))
            .unwrap();
        let request = client.start_request(
            "tools/call",
            json!({"name":"put_asset","arguments":arguments}),
        );
        assert!(
            matches!(
                client.messages.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "Asset publication must wait for the dependency reader"
        );
        let started = Instant::now();
        client.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request,"reason":"cancel asset lock wait"}}));
        assert_eq!(
            client.data("get_document", json!({}))["revision"],
            before["revision"]
        );
        assert!(started.elapsed() < Duration::from_millis(1500));
        assert!(!target.exists());
        drop(locks);
        let result = client.data("put_asset", arguments);
        assert_eq!(result["uri"], uri);
        assert_eq!(fs::read(&target).unwrap(), bytes);
    }
    assert_eq!(
        client.data("get_document", json!({}))["revision"],
        before["revision"]
    );
    assert!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.finish();
}

#[test]
fn queued_tool_cancellation_does_not_execute_the_cancelled_edit() {
    let (_temp, file, data) = fixture();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    let store = lazypencil::store::Store::open(&file, Some(data.clone())).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.lock_file_path())
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let edit = client.start_request(
        "tools/call",
        json!({"name":"apply_operations","arguments":{
            "request_id":"slow-edit","expected_revision":document["revision"],"operations":[]
        }}),
    );
    let cancelled = client.start_request(
        "tools/call",
        json!({"name":"apply_operations","arguments":{
            "request_id":"cancelled-edit","expected_revision":document["revision"],
            "operations":[{"op":"insert","node":{"type":"rectangle","width":10,"height":10}}]
        }}),
    );
    client.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":cancelled,"reason":"test"}}));
    std::thread::sleep(Duration::from_millis(100));
    drop(lock);
    let response = client.response(edit);
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert_eq!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lazypencil::store::Store::open(&file, Some(data.clone()))
            .unwrap()
            .read()
            .unwrap()
            .node_count(),
        0
    );
    client.finish();
}

#[test]
fn cancelling_locked_edits_restore_and_history_unblocks_the_mcp_worker() {
    let (_temp, file, data) = fixture();
    let mut client = Client::start(&file, &data);
    let before = client.data("get_document", json!({}));
    let saved = client.data("apply_operations", json!({"request_id":"initial","expected_revision":before["revision"],"operations":[{"op":"insert","client_ref":"box","node":{"type":"rectangle","width":10,"height":10}}]}));
    let node_id = saved["bindings"]["box"].as_str().unwrap();
    let changed = client.data("apply_operations", json!({"request_id":"second","expected_revision":saved["revision"],"operations":[{"op":"update","target":node_id,"set":{"width":20}}]}));
    let store = lazypencil::store::Store::open(&file, Some(data)).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.lock_file_path())
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    for (tool, arguments) in [
        (
            "apply_operations",
            json!({"request_id":"cancelled-apply","expected_revision":changed["revision"],"operations":[{"op":"import_component","mode":"copy","source":{"style_id":"standard-ui","node_id":"button"}}]}),
        ),
        #[cfg(feature = "rendering")]
        (
            "apply_operations",
            json!({"request_id":"cancelled-runtime-apply","expected_revision":changed["revision"],"expected_runtime_dependency_revision":"must-not-evaluate","operations":[]}),
        ),
        (
            "restore_checkpoint",
            json!({"request_id":"cancelled-restore","expected_revision":changed["revision"],"checkpoint_id":saved["checkpoint_id"]}),
        ),
        ("get_history", json!({})),
    ] {
        let request =
            client.start_request("tools/call", json!({"name":tool,"arguments":arguments}));
        std::thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        client.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request,"reason":"cancel lock wait"}}));
        assert_eq!(
            client.data("get_document", json!({}))["revision"],
            changed["revision"]
        );
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{tool} did not release the document worker while the lock was still held"
        );
    }
    drop(lock);
    assert_eq!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(store.read().unwrap().node(node_id).unwrap()["width"], 20);
    assert!(!file.parent().unwrap().join("assets").exists());
    client.finish();
}

#[test]
fn real_stdio_server_stores_assets_and_exposes_inherited_edits_without_changing_definitions() {
    let (temp, file, data) = fixture();
    fs::write(&file,r#"{"version":"2.21","children":[
        {"id":"component","type":"frame","reusable":true,"children":[{"id":"label","type":"text","content":"Before"}]},
        {"id":"instance","type":"ref","ref":"component"}
    ]}"#).unwrap();
    let mut client = Client::start(&file, &data);
    let tools = client.request("tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    let read = tools.iter().find(|t| t["name"] == "read_nodes").unwrap();
    assert!(read["inputSchema"]["properties"]["instance_path"].is_object());
    assert!(read["inputSchema"]["properties"]["library_alias"].is_object());
    let put = tools.iter().find(|t| t["name"] == "put_asset").unwrap();
    assert_eq!(put["annotations"]["readOnlyHint"], false);
    assert_eq!(put["inputSchema"]["additionalProperties"], false);
    let before = client.data("get_document", json!({}));
    let source = "// immutable source\nreturn [];";
    let asset = client.data(
        "put_asset",
        json!({"kind":"script","source":{"text":source}}),
    );
    assert_eq!(
        client.data("get_document", json!({}))["revision"],
        before["revision"]
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(asset["uri"].as_str().unwrap())).unwrap(),
        source
    );
    let edited=client.data("apply_operations",json!({"request_id":"override-and-link","expected_revision":before["revision"],"operations":[
        {"op":"update","target":{"node_id":"instance","instance_path":["label"]},"set":{"content":"After"}},
        {"op":"insert","node":{"type":"script","scriptUri":asset["uri"],"width":20,"height":20}}
    ]}));
    let inherited = client.data(
        "read_nodes",
        json!({"node_id":"instance","instance_path":["label"],"depth":0}),
    );
    assert_eq!(inherited["nodes"][0]["properties"]["content"], "After");
    assert_eq!(
        client.data("read_nodes", json!({"node_id":"label"}))["nodes"][0]["content"],
        "Before"
    );
    assert_eq!(
        client.data("read_asset", json!({"hash":asset["hash"]}))["content"],
        source
    );
    let invalid = client.call(
        "put_asset",
        json!({"kind":"image","source":{"attachment_id":"unregistered"}}),
    );
    assert_eq!(invalid["isError"], true);
    let unlinked = client.call(
        "read_asset",
        json!({"hash":"0000000000000000000000000000000000000000000000000000000000000000"}),
    );
    assert_eq!(unlinked["isError"], true);
    assert_eq!(
        client.data("get_document", json!({}))["revision"],
        edited["revision"]
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn real_stdio_image_upload_is_linked_atomically_and_returns_actual_pixels() {
    use base64::Engine;
    use skia_safe::{Color, Data, Image};
    let (temp, file, data) = fixture();
    let mut client = Client::start(&file, &data);
    let mut surface = skia_safe::surfaces::raster_n32_premul((8, 4)).unwrap();
    surface.canvas().clear(Color::BLUE);
    let png =
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(png.as_bytes());
    let wrong = client.call(
        "put_asset",
        json!({"kind":"image","source":{"mime_type":"image/jpeg","data":encoded}}),
    );
    assert_eq!(wrong["isError"], true);
    assert!(!temp.path().join("assets").exists());
    let asset = client.data(
        "put_asset",
        json!({"kind":"image","source":{"mime_type":"image/png","data":encoded}}),
    );
    assert_eq!(asset["width"], 8);
    assert_eq!(asset["height"], 4);
    let document = client.data("get_document", json!({}));
    assert_eq!(document["node_count"], 0);
    let edited=client.data("apply_operations",json!({"request_id":"image","expected_revision":document["revision"],"operations":[
        {"op":"insert","node":{"type":"rectangle","width":32,"height":16,"fill":{"type":"image","url":asset["uri"]}}}
    ]}));
    let result=client.call("render",json!({"expected_revision":edited["revision"],"expected_dependency_revision":edited["dependency_revision"]}));
    assert_ne!(result["isError"], true, "{result}");
    let content = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "image")
        .unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(content["data"].as_str().unwrap())
        .unwrap();
    let image = Image::from_encoded(Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!((image.width(), image.height()), (32, 16));
    assert_eq!(image.peek_pixels().unwrap().get_color((16, 8)), Color::BLUE);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn failed_script_nested_library_source_can_be_read_and_repaired() {
    let (temp, file, data) = fixture();
    let library = temp.path().join("library");
    fs::create_dir(&library).unwrap();
    let outer =
        "/** @schema 2.21 */ return [{type:'script',scriptUri:'nested.js',width:40,height:20}];";
    let broken = concat!(
        "/** @schema 2.21 */\n",
        "const label = '日本語';\n",
        "// Context outside the diagnostic excerpt.\n",
        "// Keep every original byte, including Unicode.\n",
        "const width = 40;\n",
        "// The source can be read without evaluating it again.\n",
        "throw new Error('Broken nested source');"
    );
    fs::write(library.join("outer.js"), outer).unwrap();
    fs::write(library.join("nested.js"), broken).unwrap();
    fs::write(library.join("ui.pen"),json!({"version":"2.21","children":[{"id":"chart","type":"script","reusable":true,"scriptUri":"outer.js","width":40,"height":20}]}).to_string()).unwrap();
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"library/ui.pen"},"children":[{"id":"use","type":"ref","ref":"ui:chart"}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    let failure = client.call("read_nodes", json!({"calculated":true,"node_id":"use"}));
    assert_eq!(
        failure["structuredContent"]["error"]["code"],
        "SCRIPT_EXECUTION_FAILED"
    );
    let details = &failure["structuredContent"]["error"]["details"];
    assert_eq!(
        details["source_hash"],
        lazypencil::document::hash(broken.as_bytes())
    );
    assert_eq!(
        details["script_path"],
        fs::canonicalize(library.join("nested.js"))
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert!(details["node"].to_string().contains("$generated"));
    assert_eq!(details["script_diagnostic"]["line"], 7);
    assert_eq!(details["script_diagnostic"]["stage"], "runtime");
    assert_eq!(
        details["script_diagnostic"]["source_excerpt"][1]["text"],
        "throw new Error('Broken nested source');"
    );
    assert!(details["failure_capture_id"].is_string());
    let mut source = String::new();
    let mut offset = 0;
    loop {
        let chunk = client.data(
            "read_asset",
            json!({"hash":details["source_hash"],"offset":offset,"limit":11}),
        );
        assert_eq!(chunk["failed_calculation"], true);
        assert_eq!(chunk["failure_capture_id"], details["failure_capture_id"]);
        assert!(chunk.get("snapshot_id").is_none());
        source.push_str(chunk["content"].as_str().unwrap());
        let Some(next) = chunk["next_offset"].as_u64() else {
            break;
        };
        offset = next;
    }
    assert_eq!(source, broken);
    let unavailable = client.call("get_layout", json!({"expected_revision":document["revision"],"snapshot_id":details["failure_capture_id"]}));
    assert_eq!(
        unavailable["structuredContent"]["error"]["code"],
        "SNAPSHOT_UNAVAILABLE"
    );
    // A changed outer source invalidates even unchanged failing nested code.
    fs::write(
        library.join("outer.js"),
        format!("{outer}\n// changed parent"),
    )
    .unwrap();
    let stale = client.call("read_asset", json!({"hash":details["source_hash"]}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    fs::write(library.join("outer.js"), outer).unwrap();
    assert_eq!(
        client.data("read_asset", json!({"hash":details["source_hash"]}))["content"],
        broken
    );
    let stored = client.data(
        "read_nodes",
        json!({"node_id":"use","expand_instances":true}),
    );
    assert_eq!(stored["nodes"][0]["properties"]["scriptUri"], "outer.js");
    assert!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let repaired = client.data("put_asset",json!({"kind":"script","source":{"text":"/** @schema 2.21 */ return [{type:'rectangle',width:40,height:20,fill:'#FF0000'}];"}}));
    let saved = client.data("apply_operations",json!({"request_id":"repair-source-from-diagnostic","expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"operations":[{"op":"update","target":"use","set":{"scriptUri":repaired["uri"]}}]}));
    let calculation = client.data(
        "read_nodes",
        json!({"calculated":true,"node_id":"use","depth":1}),
    );
    assert_eq!(calculation["revision"], saved["revision"]);
    assert_eq!(
        calculation["nodes"][0]["children"][0]["properties"]["fill"],
        "#FF0000"
    );
    let old_source = client.call("read_asset", json!({"hash":details["source_hash"]}));
    assert_eq!(
        old_source["structuredContent"]["error"]["code"],
        "ASSET_NOT_FOUND"
    );
    assert_eq!(
        fs::read_to_string(library.join("nested.js")).unwrap(),
        broken
    );
    assert_eq!(fs::read_to_string(library.join("outer.js")).unwrap(), outer);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn failed_script_source_reads_choose_latest_successful_or_failed_context() {
    let (temp, file, data) = fixture();
    let library = temp.path().join("ui.pen");
    let outer = "/** @schema 2.21 */ return [{type:'script',scriptUri:'nested.js',width:pencil.width,height:20}];";
    let nested =
        "/** @schema 2.21 */ if(pencil.width === 41) throw new Error('width rejected'); return [];";
    fs::write(temp.path().join("outer.js"), outer).unwrap();
    fs::write(temp.path().join("nested.js"), nested).unwrap();
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"ui.pen"},"children":[{"id":"use","type":"ref","ref":"ui:chart"}]}).to_string()).unwrap();
    fs::write(&library,json!({"version":"2.21","children":[{"id":"chart","type":"script","reusable":true,"scriptUri":"outer.js","width":40,"height":20}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let source_hash = lazypencil::document::hash(nested.as_bytes());
    for width in [40, 41, 42] {
        fs::write(&library,json!({"version":"2.21","children":[{"id":"chart","type":"script","reusable":true,"scriptUri":"outer.js","width":width,"height":20}]}).to_string()).unwrap();
        let calculated = client.call("read_nodes", json!({"calculated":true,"node_id":"use"}));
        let source = client.data("read_asset", json!({"hash":source_hash}));
        assert_eq!(source["content"], nested);
        if width == 41 {
            assert_eq!(
                calculated["structuredContent"]["error"]["code"],
                "SCRIPT_EXECUTION_FAILED"
            );
            assert_eq!(source["failed_calculation"], true);
            assert_eq!(
                source["failure_capture_id"],
                calculated["structuredContent"]["error"]["details"]["failure_capture_id"]
            );
            assert!(source.get("snapshot_id").is_none());
        } else {
            assert_ne!(calculated["isError"], true, "{calculated}");
            assert_eq!(
                source["snapshot_id"],
                calculated["structuredContent"]["snapshot_id"]
            );
            assert!(source.get("failed_calculation").is_none());
        }
    }
    client.finish();
}

#[cfg(all(feature = "rendering", unix))]
#[test]
fn failed_script_source_rejects_a_retargeted_link_even_when_bytes_match() {
    use std::os::unix::fs::symlink;
    let (temp, file, data) = fixture();
    let code = "/** @schema 2.21 */ throw new Error('nested');";
    for name in ["a.js", "b.js"] {
        fs::write(temp.path().join(name), code).unwrap();
    }
    let requested = temp.path().join("nested.js");
    symlink("a.js", &requested).unwrap();
    fs::write(
        temp.path().join("outer.js"),
        "/** @schema 2.21 */ return [{type:'script',scriptUri:'nested.js',width:20,height:20}];",
    )
    .unwrap();
    fs::write(&file,json!({"version":"2.21","children":[{"id":"chart","type":"script","scriptUri":"outer.js","width":20,"height":20}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let failure = client.call("read_nodes", json!({"calculated":true}));
    let hash = &failure["structuredContent"]["error"]["details"]["source_hash"];
    assert_eq!(
        client.data("read_asset", json!({"hash":hash}))["content"],
        code
    );
    fs::remove_file(&requested).unwrap();
    symlink("b.js", &requested).unwrap();
    let stale = client.call("read_asset", json!({"hash":hash}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn failed_script_sources_are_bounded_and_reject_arbitrary_file_reads() {
    let (temp, file, data) = fixture();
    fs::write(
        temp.path().join("outer.js"),
        "/** @schema 2.21 */ return [{type:'script',scriptUri:'nested.js',width:20,height:20}];",
    )
    .unwrap();
    fs::write(&file,json!({"version":"2.21","children":[{"id":"chart","type":"script","scriptUri":"outer.js","width":20,"height":20}]}).to_string()).unwrap();
    let private = "// Never declared or encountered during execution";
    fs::write(temp.path().join("private.js"), private).unwrap();
    let mut client = Client::start(&file, &data);
    let mut hashes = Vec::new();
    let mut last_source = String::new();
    for number in 0..5 {
        let code = format!("// Broken header {number} 日本語\nthrow new Error('never executed');");
        fs::write(temp.path().join("nested.js"), &code).unwrap();
        let failure = client.call("read_nodes", json!({"calculated":true}));
        assert_eq!(
            failure["structuredContent"]["error"]["details"]["script_diagnostic"]["stage"],
            "header"
        );
        assert!(failure["structuredContent"]["error"]["details"]["failure_capture_id"].is_string());
        hashes.push(lazypencil::document::hash(code.as_bytes()));
        last_source = code;
    }
    let evicted = client.call("read_asset", json!({"hash":hashes[0]}));
    assert_eq!(
        evicted["structuredContent"]["error"]["code"],
        "ASSET_NOT_FOUND"
    );
    let stale = client.call("read_asset", json!({"hash":hashes[1]}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let latest = client.data("read_asset", json!({"hash":hashes[4]}));
    assert_eq!(latest["content"], last_source);
    assert!(latest.get("script_header_error").is_some());
    assert!(latest.get("script_header").is_none());
    let unknown = client.call(
        "read_asset",
        json!({"hash":lazypencil::document::hash(private.as_bytes())}),
    );
    assert_eq!(
        unknown["structuredContent"]["error"]["code"],
        "ASSET_NOT_FOUND"
    );
    fs::remove_file(temp.path().join("nested.js")).unwrap();
    let missing = client.call("read_asset", json!({"hash":hashes[4]}));
    assert_eq!(
        missing["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    assert!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn real_stdio_script_source_inputs_and_generated_pixels_form_one_workflow() {
    use base64::Engine;
    use skia_safe::{Color, Data, Image};
    let (_temp, file, data) = fixture();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    assert_eq!(document["capabilities"]["script"]["rendering"], true);
    let asset=client.data("put_asset",json!({"kind":"script","source":{"text":"/**\n@schema 2.21\n@input color: color = #0000FF\n*/\nreturn [{type:'rectangle',width:pencil.width,height:pencil.height,fill:pencil.input.color}];"}}));
    let edited=client.data("apply_operations",json!({"request_id":"script-create","expected_revision":document["revision"],"operations":[
        {"op":"insert","client_ref":"chart","node":{"type":"script","scriptUri":asset["uri"],"width":40,"height":20}}
    ]}));
    let id = edited["bindings"]["chart"].as_str().unwrap();
    assert_eq!(
        client.data("read_asset", json!({"hash":asset["hash"]}))["script_header"]["inputs"][0]["name"],
        "color"
    );
    let layout=client.data("get_layout",json!({"expected_revision":edited["revision"],"expected_dependency_revision":edited["dependency_revision"]}));
    assert_eq!(layout["script_executions"].as_array().unwrap().len(), 1);
    assert_eq!(layout["nodes"][1]["generated_by"]["node_id"], id);
    let generated = client.data(
        "read_nodes",
        json!({"snapshot_id":layout["snapshot_id"],"node_id":layout["nodes"][1]["id"],"depth":0}),
    );
    assert_eq!(generated["view"], "calculated");
    assert_eq!(generated["variables_resolved"], true);
    assert_eq!(generated["nodes"][0]["properties"]["fill"], "#0000FF");
    assert_eq!(generated["nodes"][0]["layout"], layout["nodes"][1]);
    assert_eq!(generated["nodes"][0]["generated_by"]["node_id"], id);
    let fresh = client.data(
        "read_nodes",
        json!({"calculated":true,"node_id":id,"depth":1}),
    );
    assert_eq!(fresh["snapshot_id"], layout["snapshot_id"]);
    assert_eq!(
        fresh["nodes"][0]["children"][0]["id"],
        layout["nodes"][1]["id"]
    );
    let page = client.data("read_nodes", json!({"calculated":true,"depth":0,"limit":1}));
    assert_eq!(page["total_matches"], 2);
    let next = client.data(
        "read_nodes",
        json!({"snapshot_id":page["snapshot_id"],"depth":0,"limit":3,"cursor":page["next_cursor"]}),
    );
    assert_eq!(next["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(next["nodes"][0]["id"], layout["nodes"][1]["id"]);
    assert!(next["next_cursor"].is_null());
    let unavailable = client.call("read_nodes", json!({"snapshot_id":"not-retained"}));
    assert_eq!(
        unavailable["structuredContent"]["error"]["code"],
        "SNAPSHOT_UNAVAILABLE"
    );
    let updated=client.data("apply_operations",json!({"request_id":"script-input","expected_revision":edited["revision"],"expected_dependency_revision":edited["dependency_revision"],
        "operations":[{"op":"update","target":id,"set":{"inputs":{"color":"#FF0000"}}}]}));
    let stale = client.call("read_nodes", json!({"snapshot_id":layout["snapshot_id"]}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let render=client.call("render",json!({"expected_revision":updated["revision"],"expected_dependency_revision":updated["dependency_revision"]}));
    assert_ne!(render["isError"], true, "{render}");
    let image = render["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|content| content["type"] == "image")
        .unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    let image = Image::from_encoded(Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(image.peek_pixels().unwrap().get_color((10, 10)), Color::RED);
    let replacement=client.data("put_asset",json!({"kind":"script","source":{"text":"/**\n@schema 2.21\n@input color: color = #0000FF\n*/\nreturn [{type:'rectangle',width:pencil.width,height:pencil.height,fill:'#00FF00'}];"}}));
    let updated=client.data("apply_operations",json!({"request_id":"script-code","expected_revision":updated["revision"],"expected_dependency_revision":updated["dependency_revision"],
        "operations":[{"op":"update","target":id,"set":{"scriptUri":replacement["uri"]}}]}));
    let render=client.call("render",json!({"expected_revision":updated["revision"],"expected_dependency_revision":updated["dependency_revision"]}));
    assert_ne!(render["isError"], true, "{render}");
    let image = render["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|content| content["type"] == "image")
        .unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    let image = Image::from_encoded(Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(
        image.peek_pixels().unwrap().get_color((10, 10)),
        Color::GREEN
    );
    assert!(
        client.data("read_nodes", json!({"node_id":id}))["nodes"][0]
            .get("children")
            .is_none()
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn real_stdio_external_script_detach_keeps_generated_refs_after_a_fresh_calculation() {
    let (temp, file, data) = fixture();
    fs::create_dir(temp.path().join("library")).unwrap();
    fs::write(
        temp.path().join("library/draw.js"),
        "/* @schema 2.21 */ return [{type:'ref',ref:'badge'}];",
    )
    .unwrap();
    fs::write(temp.path().join("library/ui.lib.pen"),json!({"version":"2.21","children":[
        {"id":"badge","type":"rectangle","reusable":true,"width":20,"height":20,"fill":"#0000ff"},
        {"id":"generator","type":"script","reusable":true,"width":20,"height":20,"scriptUri":"draw.js"}
    ]}).to_string()).unwrap();
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},"children":[
        {"id":"badge","type":"rectangle","reusable":true,"x":40,"width":20,"height":20,"fill":"#ff0000"},
        {"id":"use","type":"ref","ref":"ui:generator"}
    ]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    assert_eq!(
        document["capabilities"]["detach_instance"]["external_scripts"],
        true
    );
    let before = client.data("get_layout",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
    let image_before = client.call("render", json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"snapshot_id":before["snapshot_id"]}));
    let saved = client.data("apply_operations",json!({"request_id":"detach-script","expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"operations":[{"op":"detach_instance","target":"use","client_ref":"detached"}]}));
    let after = client.data("get_layout",json!({"expected_revision":saved["revision"],"expected_dependency_revision":saved["dependency_revision"]}));
    let image_after = client.call("render", json!({"expected_revision":saved["revision"],"expected_dependency_revision":saved["dependency_revision"],"snapshot_id":after["snapshot_id"]}));
    let image_data = |result: &Value| {
        assert_ne!(result["isError"], true, "{result}");
        result["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|content| content["type"] == "image")
            .unwrap()["data"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(image_data(&image_before), image_data(&image_after));
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(image_data(&image_after))
        .unwrap();
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(
        image.peek_pixels().unwrap().get_color((5, 5)),
        skia_safe::Color::BLUE
    );
    let node = client.data(
        "read_nodes",
        json!({"node_id":saved["bindings"]["detached"],"depth":0}),
    );
    assert_eq!(
        node["nodes"][0]["metadata"]["lazypencil"]["script_scope"]["library"],
        "ui"
    );
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn cancelling_a_running_script_unblocks_the_mcp_document_worker() {
    let (temp, file, data) = fixture();
    fs::write(
        temp.path().join("draw.js"),
        "/** @schema 2.21 */ return [];",
    )
    .unwrap();
    fs::write(&file,r#"{"version":"2.21","children":[{"id":"chart","type":"script","scriptUri":"draw.js","width":40,"height":20}]}"#).unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    let layout = client.data("get_layout",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
    fs::write(
        temp.path().join("draw.js"),
        "/** @schema 2.21 */ for (;;) {}",
    )
    .unwrap();
    let document = client.data("get_document", json!({}));
    for tool in ["render", "validate_document", "read_nodes"] {
        let mut arguments = json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]});
        if tool == "validate_document" {
            arguments["scope"] = json!("layout");
        } else if tool == "read_nodes" {
            arguments = json!({"calculated":true});
        }
        let request =
            client.start_request("tools/call", json!({"name":tool,"arguments":arguments}));
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(200));
        client.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request,"reason":"stop script"}}));
        assert_eq!(
            client.data("get_document", json!({}))["revision"],
            document["revision"]
        );
        assert!(started.elapsed() < Duration::from_millis(1500));
    }
    let request = client.start_request(
        "tools/call",
        json!({"name":"apply_operations","arguments":{
            "request_id":"cancelled-runtime-edit","expected_revision":document["revision"],
            "expected_dependency_revision":document["dependency_revision"],
            "expected_runtime_dependency_revision":layout["runtime_dependency_revision"],
            "operations":[{"op":"update","target":"chart","set":{"name":"Must not be saved"}}]
        }}),
    );
    let started = Instant::now();
    std::thread::sleep(Duration::from_millis(200));
    client.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request,"reason":"stop precondition evaluation"}}));
    assert_eq!(
        client.data("get_document", json!({}))["revision"],
        document["revision"]
    );
    assert!(
        client.data("get_history", json!({}))["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(started.elapsed() < Duration::from_millis(1500));
    fs::write(
        temp.path().join("draw.js"),
        "/** @schema 2.21 */ return [];",
    )
    .unwrap();
    let document = client.data("get_document", json!({}));
    client.data("get_layout",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn runtime_generated_script_sources_can_be_read_only_from_a_current_captured_snapshot() {
    let (temp, file, data) = fixture();
    fs::write(
        temp.path().join("draw.js"),
        "/** @schema 2.21 */ return [{type:'script',scriptUri:'nested.js',width:20,height:20}];",
    )
    .unwrap();
    fs::write(
        temp.path().join("nested.js"),
        "/** @schema 2.21 */ return [];",
    )
    .unwrap();
    fs::write(&file,r#"{"version":"2.21","children":[{"id":"chart","type":"script","scriptUri":"draw.js","width":20,"height":20}]}"#).unwrap();
    let mut client = Client::start(&file, &data);
    let document = client.data("get_document", json!({}));
    let layout=client.data("get_layout",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]}));
    let hash = layout["runtime_dependencies"][0]["revision"]
        .as_str()
        .unwrap();
    let source = client.data("read_asset", json!({"hash":hash}));
    assert_eq!(source["script_header"]["schema"], "2.21");
    assert_eq!(source["snapshot_id"], layout["snapshot_id"]);
    let nodes = client.data(
        "read_nodes",
        json!({"snapshot_id":layout["snapshot_id"],"depth":2}),
    );
    assert_eq!(
        nodes["nodes"][0]["children"][0]["properties"]["scriptUri"],
        "nested.js"
    );
    assert_eq!(
        client.data("get_document", json!({}))["dependency_revision"],
        document["dependency_revision"]
    );
    fs::write(
        temp.path().join("nested.js"),
        "/** @schema 2.21 */ return [{type:'rectangle',width:20,height:20}];",
    )
    .unwrap();
    let stale = client.call("read_asset", json!({"hash":hash}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let stale = client.call("read_nodes", json!({"snapshot_id":layout["snapshot_id"]}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let stale=client.call("render",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"snapshot_id":layout["snapshot_id"]}));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    let edit = json!({"request_id":"runtime-precondition","expected_revision":document["revision"],
        "expected_dependency_revision":document["dependency_revision"],
        "expected_runtime_dependency_revision":layout["runtime_dependency_revision"],
        "operations":[{"op":"update","target":"chart","set":{"name":"Reviewed"}}]});
    let stale = client.call("apply_operations", edit.clone());
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    assert_eq!(
        client.data("get_document", json!({}))["revision"],
        document["revision"]
    );
    fs::write(
        temp.path().join("nested.js"),
        "/** @schema 2.21 */ return [];",
    )
    .unwrap();
    let saved = client.data("apply_operations", edit.clone());
    assert_eq!(
        saved["validated_runtime_dependency_revision"],
        layout["runtime_dependency_revision"]
    );
    fs::remove_file(temp.path().join("nested.js")).unwrap();
    assert_eq!(client.data("apply_operations", edit)["replayed"], true);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn real_mcp_background_blur_returns_transparent_image_content_without_chromium() {
    use base64::Engine;
    use skia_safe::{Data, Image};
    let (temp, file, data) = fixture();
    fs::write(&file,json!({"version":"2.21","children":[
        {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#0000FF80"},
        {"id":"glass","type":"rectangle","x":20,"y":10,"width":40,"height":20,"effect":{"type":"background_blur","radius":8}}
    ]}).to_string()).unwrap();
    let absent = temp.path().join("absent-chromium");
    let mut client = Client::start_with_chromium(&file, &data, Some(&absent));
    let document = client.data("get_document", json!({}));
    let result = client.call(
        "render",
        json!({"expected_revision":document["revision"],"node_id":"glass"}),
    );
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["width"], 40);
    assert!(
        result["structuredContent"]["dynamic_artifacts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let content = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|content| content["type"] == "image")
        .unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(content["data"].as_str().unwrap())
        .unwrap();
    let image = Image::from_encoded(Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let pixel = image.peek_pixels().unwrap().get_color((20, 10));
    assert!((pixel.a() as i32 - 128).abs() <= 1, "{pixel:?}");
    assert_eq!(pixel.b(), 255);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
#[ignore = "Requires LAZYPENCIL_TEST_CHROMIUM and permission to launch Chromium"]
fn real_mcp_shader_assets_uniform_edits_and_browser_refresh_return_image_content() {
    use base64::Engine;
    let (_temp, file, data) = fixture();
    let chromium = std::path::PathBuf::from(std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap());
    let mut client = Client::start_with_chromium(&file, &data, Some(&chromium));
    let document = client.data("get_document", json!({}));
    let asset=client.data("put_asset",json!({"kind":"shader","source":{"text":"precision highp float;uniform vec4 tint;void main(){gl_FragColor=tint;}"}}));
    let edited=client.data("apply_operations",json!({"request_id":"shader","expected_revision":document["revision"],"operations":[
        {"op":"insert","client_ref":"rect","node":{"type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":asset["uri"],"uniforms":{"tint":"#FF0000"}}}}
    ]}));
    let mut revision = edited["revision"].clone();
    let rectangle_id = edited["bindings"]["rect"].clone();
    let mut dependency = edited["dependency_revision"].clone();
    for color in ["#FF0000", "#0000FF"] {
        if color == "#0000FF" {
            let changed=client.data("apply_operations",json!({"request_id":"uniform-edit","expected_revision":revision,"expected_dependency_revision":dependency,"operations":[
                {"op":"update","target":edited["bindings"]["rect"],"set":{"fill":{"type":"shader","url":asset["uri"],"uniforms":{"tint":color}}}}
            ]}));
            revision = changed["revision"].clone();
        }
        let result = client.call(
            "render",
            json!({"expected_revision":revision,"expected_dependency_revision":dependency}),
        );
        assert_ne!(result["isError"], true, "{result}");
        let image = result["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|content| content["type"] == "image")
            .unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().unwrap())
            .unwrap();
        let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!(
            image.peek_pixels().unwrap().get_color((10, 10)),
            if color == "#FF0000" {
                skia_safe::Color::RED
            } else {
                skia_safe::Color::BLUE
            }
        );
    }
    let numeric = client.data("put_asset",json!({"kind":"shader","source":{"text":"precision highp float;uniform float red,green;uniform float blue[2];uniform mat2 transform;void main(){gl_FragColor=vec4(transform*vec2(red,green),blue[1],1.0);}"}}));
    let current = client.data("get_document", json!({}));
    let changed = client.data("apply_operations",json!({"request_id":"numeric-uniform-edit","expected_revision":revision,
        "expected_dependency_revision":current["dependency_revision"],"operations":[{"op":"update","target":rectangle_id,
        "set":{"fill":{"type":"shader","url":numeric["uri"],"uniforms":{"red":0.25,"green":0.5,"blue":[0.0,0.75],"transform":[1,0,0,1]}}}}]}));
    revision = changed["revision"].clone();
    dependency = changed["dependency_revision"].clone();
    let result = client.call("render",json!({"expected_revision":revision,"expected_dependency_revision":dependency,"node_id":rectangle_id}));
    assert_ne!(result["isError"], true, "{result}");
    let image = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["type"] == "image")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let color = image.peek_pixels().unwrap().get_color((10, 10));
    for (actual, expected) in [color.r(), color.g(), color.b(), color.a()]
        .into_iter()
        .zip([64, 128, 191, 255])
    {
        assert!(
            (actual as i32 - expected).abs() <= 1,
            "Unexpected MCP numeric uniform pixel: {color:?}"
        );
    }
    let page = "data:text/html,<style>body{margin:0;background:%2300f}</style>";
    let edited=client.data("apply_operations",json!({"request_id":"browser","expected_revision":revision,"expected_dependency_revision":dependency,"operations":[
        {"op":"insert","client_ref":"page","node":{"type":"browser","url":page,"x":50,"width":40,"height":20}}
    ]}));
    let mut ids = Vec::new();
    for refresh in [false, false, true] {
        let result=client.data("render",json!({"expected_revision":edited["revision"],"expected_dependency_revision":edited["dependency_revision"],"node_id":edited["bindings"]["page"],"refresh":refresh}));
        let artifacts = result["dynamic_artifacts"].as_array().unwrap();
        let page = artifacts
            .iter()
            .find(|artifact| {
                artifact["node_id"] == edited["bindings"]["page"]
                    && artifact["acquisition"]["kind"] == "browser"
            })
            .expect("Browser acquisition metadata");
        ids.push(
            page["acquisition"]["acquisition_id"]
                .as_str()
                .expect("A real page acquisition ID")
                .to_string(),
        );
        assert!(
            !artifacts
                .iter()
                .any(|artifact| artifact["node_id"] == rectangle_id),
            "An offscreen shader must not be evaluated for this crop"
        );
    }
    assert_eq!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[2]);
    client.finish();
}

#[cfg(all(feature = "rendering", unix))]
#[test]
#[ignore = "Requires Chromium and a real 60-second idle interval without sending MCP jobs"]
fn idle_mcp_reaps_script_page_and_shader_processes_without_another_request() {
    fn children(parent: u32) -> Vec<u32> {
        let output = Command::new("ps")
            .args(["-axo", "pid=,ppid="])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse::<u32>().ok()?;
                let ppid = fields.next()?.parse::<u32>().ok()?;
                (ppid == parent).then_some(pid)
            })
            .collect()
    }
    let (temp, file, data) = fixture();
    fs::write(
        temp.path().join("generated.js"),
        "/** @schema 2.21 */ return [{type:'rectangle',width:16,height:16,fill:'#00FF00'}];",
    )
    .unwrap();
    fs::write(
        temp.path().join("blue.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(0.,0.,1.,1.);}",
    )
    .unwrap();
    fs::write(&file, json!({"version":"2.21","children":[
        {"id":"script","type":"script","scriptUri":"generated.js","width":16,"height":16},
        {"id":"page","type":"browser","x":16,"width":16,"height":16,"url":"data:text/html,<style>html{background:%23f00}</style>"},
        {"id":"shader","type":"rectangle","x":32,"width":16,"height":16,"fill":{"type":"shader","url":"blue.glsl"}}
    ]}).to_string()).unwrap();
    let saved = fs::read(&file).unwrap();
    let chromium = std::path::PathBuf::from(std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap());
    let mut client = Client::start_with_chromium(&file, &data, Some(&chromium));
    let document = client.data("get_document", json!({}));
    let revisions = json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"]});
    let layout = client.data("get_layout", revisions.clone());
    let mut arguments = revisions.clone();
    arguments["snapshot_id"] = layout["snapshot_id"].clone();
    let first = client.data("render", arguments.clone());
    assert_eq!(first["dynamic_artifacts"].as_array().unwrap().len(), 2);
    let pids = children(client.child.id());
    assert_eq!(
        pids.len(),
        3,
        "MCP must own script, page and shader children"
    );
    let started = Instant::now();
    loop {
        if children(client.child.id()).is_empty() {
            break;
        }
        assert!(client.child.try_wait().unwrap().is_none());
        assert!(
            started.elapsed() < Duration::from_secs(75),
            "Idle children did not exit"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    for pid in pids {
        assert_eq!(
            unsafe { libc::kill(pid as i32, 0) },
            -1,
            "Idle child {pid} remains alive"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
    let cached = client.data("render", arguments.clone());
    assert!(
        children(client.child.id()).is_empty(),
        "Cached rendering must not launch a runtime"
    );
    assert_eq!(cached["snapshot_id"], first["snapshot_id"]);
    assert_eq!(cached["png_hash"], first["png_hash"]);
    assert_eq!(cached["dynamic_artifacts"], first["dynamic_artifacts"]);
    let rebuilt = client.data("get_layout", revisions);
    assert_eq!(rebuilt["snapshot_id"], layout["snapshot_id"]);
    arguments["refresh"] = json!(true);
    let refreshed = client.data("render", arguments);
    assert_eq!(children(client.child.id()).len(), 3);
    assert_eq!(refreshed["png_hash"], first["png_hash"]);
    assert_ne!(refreshed["dynamic_artifacts"], first["dynamic_artifacts"]);
    assert_eq!(fs::read(&file).unwrap(), saved);
    client.finish();
}
