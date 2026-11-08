use super::{Result, append_json, spawn, write_json};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Mcp {
    document_id: Option<String>,
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_id: u64,
}
impl Mcp {
    fn connect(config: &Value, cwd: &Path) -> Result<Self> {
        let command = config["command"].as_str().ok_or("Missing MCP command")?;
        assert!(Path::new(command).is_absolute());
        let mut process = Command::new(command);
        for arg in config["args"].as_array().ok_or("Missing MCP arguments")? {
            process.arg(arg.as_str().ok_or("Invalid MCP argument")?);
        }
        for item in config["env"].as_array().into_iter().flatten() {
            process.env(
                item["name"]
                    .as_str()
                    .ok_or("Missing MCP environment name")?,
                item["value"]
                    .as_str()
                    .ok_or("Missing MCP environment value")?,
            );
        }
        let mut child = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().ok_or("Missing MCP output")?);
        let mut result = Self {
            document_id: None,
            child,
            input,
            output,
            next_id: 2,
        };
        result.send(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acp-fixture","version":"1"}}}))?;
        assert_eq!(result.read()?["result"]["protocolVersion"], "2025-11-25");
        result.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        let args = config["args"].as_array().ok_or("Missing MCP args")?;
        let grant = args.windows(2).find(|pair| {
            pair[0] == "--attachments-manifest" || pair[0] == "--guidance-audit-manifest"
        });
        let path = if let Some(pair) = grant {
            let manifest: Value =
                serde_json::from_slice(&fs::read(pair[1].as_str().ok_or("Missing grant path")?)?)?;
            PathBuf::from(
                manifest["document"]
                    .as_str()
                    .ok_or("Missing grant document")?,
            )
        } else {
            cwd.join("design.pen")
        };
        let opened = result.call("open_document", json!({"path":path}))?;
        result.document_id = Some(
            opened["structuredContent"]["document_id"]
                .as_str()
                .ok_or("Missing document_id")?
                .to_owned(),
        );
        Ok(result)
    }
    fn send(&mut self, value: &Value) -> Result {
        let input = self.input.as_mut().ok_or("MCP input already closed")?;
        serde_json::to_writer(&mut *input, value)?;
        writeln!(input)?;
        input.flush()?;
        Ok(())
    }
    fn read_raw(&mut self) -> Result<String> {
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            return Err("MCP server terminated".into());
        }
        Ok(line)
    }
    fn read(&mut self) -> Result<Value> {
        Ok(serde_json::from_str(&self.read_raw()?)?)
    }
    fn call(&mut self, name: &str, args: Value) -> Result<Value> {
        Ok(serde_json::from_str(self.call_raw(name, args)?.get())?)
    }
    fn call_raw(&mut self, name: &str, mut args: Value) -> Result<Box<RawValue>> {
        if !matches!(
            name,
            "open_document" | "create_document" | "list_documents" | "list_icons"
        ) && let Some(id) = &self.document_id
        {
            args["document_id"] = json!(id);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}}))?;
        loop {
            let line = self.read_raw()?;
            let response: Value = serde_json::from_str(&line)?;
            if response["id"] == id {
                assert!(response.get("error").is_none(), "{response}");
                assert_ne!(response["result"]["isError"], true, "{response}");
                #[derive(Deserialize)]
                struct Envelope {
                    result: Box<RawValue>,
                }
                return Ok(serde_json::from_str::<Envelope>(&line)?.result);
            }
        }
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        self.input.take();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Peer<'a> {
    mode: &'a str,
    mcp: Option<Mcp>,
    usage_manifest: Option<PathBuf>,
    pending: Option<Value>,
    turns: u64,
    permission_answered: bool,
    cancel_received: bool,
    model: String,
    fast: bool,
    selection: String,
    authenticated: bool,
    session: String,
    mcp_server: String,
}
impl<'a> Peer<'a> {
    fn new(mode: &'a str) -> Self {
        Self {
            mode,
            mcp: None,
            usage_manifest: None,
            pending: None,
            turns: 0,
            permission_answered: false,
            cancel_received: false,
            model: "fixture-model".into(),
            fast: mode == "settings-fast-inherited",
            selection: "read-only".into(),
            authenticated: false,
            session: "fixture-native".into(),
            mcp_server: "lazypencil".into(),
        }
    }
    fn send(&self, mut value: Value) -> Result {
        value["jsonrpc"] = json!("2.0");
        let mut out = std::io::stdout().lock();
        serde_json::to_writer(&mut out, &value)?;
        writeln!(out)?;
        out.flush()?;
        Ok(())
    }
    fn response(&self, message: &Value, result: Value) -> Result {
        self.send(json!({"id":message["id"],"result":result}))
    }
    fn notify(&self, update: Value) -> Result {
        self.send(
            json!({"method":"session/update","params":{"sessionId":self.session,"update":update}}),
        )
    }
    fn auth(&self, kind: &str) -> Result {
        let email = if self.mode == "auth-inspect-account" && Path::new("account-changed").exists()
        {
            "other-account@example.test"
        } else {
            "must-not-log@example.test"
        };
        self.send(json!({"method":"_auth/status_update","params":{"authStatus":{"kind":kind,"label":"Fixture account","account":{"plan":"test-plan","email":email},"accessToken":"fixture-not-a-token","vendor":{"secret":"must-not-log"}}}}))
    }
    fn config(&self) -> Value {
        let mut options = json!([{"id":"model","name":"Model","category":"model","type":"select","currentValue":self.model,"options":[{"group":"fixture","name":"Fixture","options":[{"value":"fixture-model","name":"Fixture"},{"value":"fixture-model-2","name":"Fixture 2"}]}]}]);
        if self.mode.starts_with("settings-fast") {
            options.as_array_mut().unwrap().push(json!({"id":"reasoning_effort","category":"thought_level","name":"Reasoning effort","type":"select","currentValue":if self.model == "fixture-model-2" {"high"} else {"medium"},"options":[{"value":"medium","name":"Medium"},{"value":"high","name":"High"}]}));
        }
        if self.mode.starts_with("settings-fast")
            && !(self.mode == "settings-fast-model-only" && self.model != "fixture-model-2")
            && !(self.mode == "settings-fast-model-removes" && self.model == "fixture-model-2")
        {
            let fast = json!({"id":"fast-mode","name":"Fast mode","category":"model_config","type":"select","currentValue":if self.fast {"on"} else {"off"},"options":[{"value":"off","name":"Off"},{"value":"on","name":"On"}]});
            options.as_array_mut().unwrap().push(fast.clone());
            if self.mode == "settings-fast-duplicate" {
                options.as_array_mut().unwrap().push(fast);
            }
        }
        if self.mode.starts_with("settings-permissions") {
            options.as_array_mut().unwrap().push(json!({"id":if self.mode=="settings-permissions-selector" {"permissions"}else{"mode"},"name":"Permissions","category":"mode","type":"select","currentValue":self.selection,"options":[{"value":"read-only","name":"Read-only"},{"value":"workspace-write","name":"Workspace access"},{"value":"agent-full-access","name":"Full access"}]}));
        }
        options
    }
    fn call(&mut self, name: &str, args: Value) -> Result<Value> {
        self.mcp
            .as_mut()
            .ok_or("Fixture is not connected to MCP")?
            .call(name, args)
    }
    fn message(&mut self, message: Value) -> Result {
        let params = &message["params"];
        let mode = self.mode;
        if mode == "malformed" {
            println!("not json");
            std::io::stdout().flush()?;
            return Ok(());
        }
        if mode == "bad-envelope" {
            self.send(json!({"unexpected":"not an RPC message"}))?;
            return Ok(());
        }
        if mode == "crash" {
            std::process::exit(9);
        }
        if mode.starts_with("auth-inspect-") {
            fs::write("account-adapter.pid", std::process::id().to_string())?;
            append_json(
                "received-account-methods.jsonl",
                &json!({"method":message["method"]}),
            )?;
        }
        let error_phase = match mode {
            "protocol-error-initialize" | "protocol-error-frame" => Some("initialize"),
            "protocol-error-authenticate" => Some("authenticate"),
            "protocol-error-new" => Some("session/new"),
            "protocol-error-load" => Some("session/load"),
            "protocol-error-mode" => Some("session/set_mode"),
            "protocol-error-model" => Some("session/set_config_option"),
            "protocol-error-prompt" => Some("session/prompt"),
            _ => None,
        };
        if let Some(phase) = error_phase
            && message["method"] == phase
        {
            write_json("rejected-acp-phase.json", &json!({"phase":phase}))?;
            if mode == "protocol-error-frame" {
                println!("MUST_NOT_ECHO_TRANSPORT_FRAME invalid JSON");
                std::io::stdout().flush()?;
            } else {
                self.send(json!({"id":message["id"],"error":{"code":-32603,
                    "message":"MUST_NOT_ECHO_RPC_MESSAGE Bearer fixture-private-credential",
                    "data":{"code":"CANCELLED","message":"MUST_NOT_ECHO_HOST_MESSAGE",
                        "details":{"authorization":"MUST_NOT_ECHO_ERROR_DATA","document":"private document excerpt"}}}}))?;
            }
            return Ok(());
        }
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                #[cfg(unix)]
                fs::write(
                    "fixture-agent-guardian.pid",
                    unsafe { libc::getppid() }.to_string(),
                )?;
                if mode == "setup-hang" {
                    return Ok(());
                }
                assert_eq!(params["protocolVersion"], 1);
                assert_ne!(params["clientCapabilities"]["terminal"], true);
                assert_ne!(params["clientCapabilities"]["fs"]["writeTextFile"], true);
                let mut caps = json!({"loadSession":mode!="no-load"&&!(mode=="resume-no-load"&&Path::new("fixture-restart").exists()),"promptCapabilities":{"image":mode!="no-images"}});
                if mode.starts_with("auth-") {
                    caps["_meta"] = json!({"authStatus":{}});
                }
                let mut initialized = json!({"protocolVersion":if mode=="wrong-version"{2}else{1},"agentCapabilities":caps,"agentInfo":{"name":"acp-fixture","version":"1"},"authMethods":[{"id":"fixture-auth","name":"Fixture"}]});
                if mode.starts_with("steering") {
                    initialized["_meta"] = json!({"steering":{"supported":true}});
                    initialized["agentInfo"] = if mode == "steering-claude" {
                        json!({"name":"@agentclientprotocol/claude-agent-acp","version":"0.100.0"})
                    } else {
                        json!({"name":"@agentclientprotocol/codex-acp","version":if mode=="steering-old" {"2.2.2"} else {"2.3.0"}})
                    };
                }
                self.response(&message, initialized)?;
                if mode.starts_with("auth-inspect-") && mode != "auth-inspect-hang" {
                    self.auth(if mode == "auth-inspect-none" {
                        "none"
                    } else {
                        "account"
                    })?;
                }
            }
            "authenticate" => {
                assert_eq!(params["methodId"], "fixture-auth");
                self.authenticated = true;
                self.response(&message, json!({}))?;
            }
            "session/new" | "session/load" => {
                let method = message["method"].as_str().unwrap();
                if mode.starts_with("settings-fast") {
                    append_json(
                        "received-settings-sessions.jsonl",
                        &json!({"method":method,"mcpServers":params["mcpServers"]}),
                    )?;
                }
                if mode.starts_with("resume-") || mode == "selection" {
                    append_json(
                        "received-session-methods.jsonl",
                        &json!({"method":method,"params":params}),
                    )?;
                    if method == "session/load"
                        && mode == "resume-load-error"
                        && Path::new("fixture-restart").exists()
                    {
                        self.send(json!({"id":message["id"],"error":{"code":-32603,"message":"fixture session storage failure"}}))?;
                        return Ok(());
                    }
                    if method == "session/new" && Path::new("fixture-restart").exists() {
                        self.session = "fixture-restarted".into();
                    }
                }
                if mode == "settings-auth-check" {
                    assert!(
                        self.authenticated,
                        "Authentication must precede session creation"
                    );
                }
                if let Some(name) = params["mcpServers"][0]["name"].as_str() {
                    self.mcp_server = name.into();
                }
                if mode == "mcp-environment" {
                    write_json(
                        "received-mcp-environment.json",
                        &json!({"servers":params["mcpServers"],"adapter_malloc_space_efficient":std::env::var("MallocSpaceEfficient").ok(),"adapter_malloc_medium_zone":std::env::var("MallocMediumZone").ok()}),
                    )?;
                }
                assert!(
                    Path::new(params["cwd"].as_str().ok_or("Missing session cwd")?).is_absolute()
                );
                if method == "session/load" {
                    if mode == "resume-native" {
                        assert!(
                            ["fixture-native", "fixture-restarted"]
                                .contains(&params["sessionId"].as_str().unwrap())
                        );
                        self.session = params["sessionId"].as_str().unwrap().into();
                    } else {
                        assert_eq!(params["sessionId"], "fixture-native");
                    }
                    self.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"replayed\n"}}))?;
                }
                if [
                    "edit",
                    "image-edit",
                    "edit-cancel",
                    "image-inspect-audit",
                    "user-config",
                    "supervised-dynamic",
                    "resource-observation",
                    "dynamic-resource-observation",
                ]
                .contains(&mode)
                    || ["project-guidance", "audit-", "resume-"]
                        .iter()
                        .any(|p| mode.starts_with(p))
                {
                    let servers = params["mcpServers"]
                        .as_array()
                        .ok_or("Missing MCP servers")?;
                    assert_eq!(servers.len(), 1);
                    let config = &servers[0];
                    let args = config["args"].as_array().ok_or("Missing MCP arguments")?;
                    if let Some(index) = args.iter().position(|s| s == "--guidance-audit-manifest")
                    {
                        self.usage_manifest =
                            Some(PathBuf::from(args[index + 1].as_str().unwrap()));
                    }
                    self.mcp = Some(Mcp::connect(
                        config,
                        Path::new(params["cwd"].as_str().ok_or("Missing session cwd")?),
                    )?);
                }
                if mode.starts_with("auth-") && mode != "auth-absent" {
                    self.auth(match mode {
                        "auth-api" => "api_key",
                        "auth-none" => "none",
                        "auth-gateway" => "gateway",
                        "auth-external" => "external",
                        "auth-unknown" => "future_kind",
                        _ => "account",
                    })?;
                }
                let mut result = if method == "session/new" {
                    json!({"sessionId":self.session})
                } else {
                    json!({})
                };
                if mode.starts_with("settings-") || mode.starts_with("protocol-error-") {
                    result["modes"] = json!({"currentModeId":self.selection,"availableModes":[{"id":"read-only","name":"Read only"},{"id":"workspace-write","name":"Workspace"}]});
                    if mode.starts_with("settings-permissions") {
                        result["modes"]["availableModes"].as_array_mut().unwrap().push(json!({"id":"agent-full-access","name":"Full access","description":"Unrestricted computer access"}));
                    }
                    result["configOptions"] = self.config();
                }
                self.response(&message, result)?;
                if mode == "grandchild" {
                    let child = spawn(&["sleep".into()], false)?;
                    let temporary = "fixture-child.pid.tmp";
                    fs::write(temporary, child.id().to_string())?;
                    fs::rename(temporary, "fixture-child.pid")?;
                }
                if mode.starts_with("process-tree") {
                    let mut args = vec![
                        "agent-tree".into(),
                        "branch".into(),
                        std::process::id().to_string(),
                    ];
                    #[cfg(unix)]
                    args.extend(["--guardian".into(), unsafe { libc::getppid() }.to_string()]);
                    if mode == "process-tree-detached" {
                        args.push("--detach".into());
                    }
                    spawn(&args, mode == "process-tree-detached")?;
                }
            }
            "session/prompt" => {
                if mode.starts_with("resume-") || mode == "selection" {
                    assert_eq!(params["sessionId"], self.session);
                    append_json("received-prompts.jsonl", params)?;
                }
                fs::write("sent-turn.txt", b"sent")?;
                self.pending = Some(message.clone());
                self.turns += 1;
                self.prompt(params)?;
            }
            "_session/steering" => {
                assert_eq!(params["sessionId"], self.session);
                assert_eq!(
                    params["_meta"]["steering"]["idleBehavior"],
                    "promptRequired"
                );
                append_json("received-steering.jsonl", params)?;
                if mode == "steering-hang" {
                    return Ok(());
                }
                if mode == "steering-failure" {
                    self.send(json!({"id":message["id"],"error":{"code":-32603,"message":"PRIVATE_STEERING_ERROR"}}))?;
                    return Ok(());
                }
                if mode == "steering-flood" {
                    for _ in 0..64 {
                        self.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"progress\n"}}))?;
                    }
                }
                // Completion can precede the steering receipt on the wire.
                if let Some(pending) = self.pending.take() {
                    self.response(&pending, json!({"stopReason":"end_turn"}))?;
                }
                self.response(&message, json!({"outcome":if mode == "steering-idle" {"promptRequired"} else if mode == "steering-unowned" {"startedNewTurn"} else {"injected"}}))?;
            }
            "session/set_mode" => {
                append_json("received-mode-changes.jsonl",params)?;
                self.selection = params["modeId"]
                    .as_str()
                    .ok_or("Missing selected mode")?
                    .into();
                self.response(&message, json!({}))?;
            }
            "session/set_config_option" => {
                if mode.starts_with("settings-fast") {
                    append_json("received-settings.jsonl", params)?;
                }
                match params["configId"].as_str() {
                    Some("mode"|"permissions") => {
                        append_json("received-mode-changes.jsonl",params)?;
                        if mode=="settings-permissions-timeout" {
                            return Ok(());
                        }
                        if mode=="settings-permissions-reject" {
                            self.send(json!({"id":message["id"],"error":{"code":-32603,"message":"PRIVATE_MODE_FAILURE"}}))?;
                            return Ok(());
                        }
                        if mode!="settings-permissions-unconfirmed" {
                            self.selection=params["value"].as_str().ok_or("Missing mode")?.into();
                        }
                    },
                    Some("model") => {
                        self.model = params["value"]
                            .as_str()
                            .ok_or("Missing selected model")?
                            .into();
                    }
                    Some("fast-mode") => {
                        assert!(mode.starts_with("settings-fast"));
                        if mode != "settings-fast-unconfirmed" {
                            self.fast = params["value"] == "on";
                        }
                    }
                    _ => return Err("Unexpected configuration selector".into()),
                }
                self.response(&message, json!({"configOptions":self.config()}))?;
            }
            "session/cancel" => {
                self.cancel_received = true;
                if ["hang-cancel", "process-tree-hang"].contains(&mode)
                    || (mode == "cancel-permission" && !self.permission_answered)
                {
                    return Ok(());
                }
                if let Some(pending) = self.pending.take() {
                    self.response(&pending, json!({"stopReason":"cancelled"}))?;
                }
            }
            _ => {
                if message["id"] == 500 {
                    let outcome = &message["result"]["outcome"];
                    if mode == "cancel-permission" {
                        assert_eq!(outcome["outcome"], "cancelled");
                        self.permission_answered = true;
                        if self.cancel_received
                            && let Some(pending) = self.pending.take()
                        {
                            self.response(&pending, json!({"stopReason":"cancelled"}))?;
                        }
                    } else {
                        assert_eq!(outcome["outcome"], "selected");
                        assert_eq!(
                            outcome["optionId"],
                            if [
                                "interactive-permission",
                                "document-permission",
                                "correlated-document"
                            ]
                            .contains(&mode)
                            {
                                "allow"
                            } else {
                                "deny"
                            }
                        );
                        if let Some(pending) = self.pending.take() {
                            self.response(&pending,json!({"stopReason":if mode=="permission-cancel"{"cancelled"}else{"end_turn"}}))?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn prompt(&mut self, params: &Value) -> Result {
        if self.mode == "auth-inspect-account" {
            self.auth("account")?;
        }
        let mode = self.mode;
        if mode.starts_with("steering") {
            append_json("received-prompts.jsonl", params)?;
            self.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working\n"}}))?;
            if self.turns > 1 || mode == "steering-old" {
                self.response(
                    self.pending.as_ref().unwrap(),
                    json!({"stopReason":"end_turn"}),
                )?;
            }
            return Ok(());
        }
        if mode == "queued-followups" {
            append_json("received-prompts.jsonl", params)?;
            fs::write(format!("started-turn-{}", self.turns), b"ready")?;
            self.resource_barrier(&format!("release-turn-{}", self.turns))?;
        }
        let blocks = params["prompt"].as_array().ok_or("Missing prompt blocks")?;
        let text = blocks
            .iter()
            .map(|v| v["text"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        if mode == "user-config" {
            let document = self.call("get_document", json!({}))?;
            let config = &document["structuredContent"]["configuration"];
            assert_eq!(
                config["settings"]["fonts"]["default_family"],
                "LP Test Sans"
            );
            assert_eq!(config["settings"]["fonts"]["locale"], "en-US");
            assert_eq!(config["font_origins"]["files"], "user");
            assert!(Path::new(config["user"]["requested_path"].as_str().unwrap()).is_absolute());
            write_json("received-user-configuration.json", config)?;
        }
        if mode.starts_with("project-guidance") {
            assert!(text.contains("ACP_PROJECT_RULE") && !text.contains("REFERENCE_NOT_AUTOMATIC"));
            let selected = if mode.ends_with("override") {
                "content-page"
            } else {
                "project:brand"
            };
            assert!(text.contains(&format!("\"id\":\"{selected}\"")));
            let style: Value = serde_json::from_str(
                blocks[0]["text"]
                    .as_str()
                    .unwrap()
                    .rsplit("\n\nSelected style:\n")
                    .next()
                    .unwrap(),
            )?;
            assert!(style["example"].get("document").is_none());
            assert_eq!(style["example"]["retrieval_tool"], "get_style");
            let guidance = self.call("get_guidance", json!({}))?["structuredContent"].clone();
            assert!(
                guidance["project_guides"][0]["body"]
                    .as_str()
                    .unwrap()
                    .contains("ACP_PROJECT_RULE")
            );
            let complete =
                self.call("get_style", json!({"id":selected}))?["structuredContent"].clone();
            assert_eq!(complete["id"], selected);
            assert_eq!(complete["hash"], style["hash"]);
            assert!(complete["example"].get("document").is_some());
            #[derive(Deserialize)]
            struct CatalogResponse {
                #[serde(rename = "structuredContent")]
                catalog: Box<RawValue>,
            }
            let raw = self
                .mcp
                .as_mut()
                .ok_or("Fixture is not connected to MCP")?
                .call_raw("list_styles", json!({}))?;
            let catalog: CatalogResponse = serde_json::from_str(raw.get())?;
            let reference =
                self.call("get_guidance", json!({"id":"project:reference"}))?["structuredContent"]
                    .clone();
            assert_eq!(reference["body"], "REFERENCE_NOT_AUTOMATIC");
            #[derive(Serialize)]
            struct Record<'a> {
                text: &'a str,
                workflow_hash: &'a Value,
                style_hash: &'a Value,
                reference_hash: &'a Value,
                // The audit hashes the response serialization, including key order.
                catalog: &'a RawValue,
            }
            write_json(
                "received-project-guidance.json",
                &Record {
                    text: &text,
                    workflow_hash: &guidance["hash"],
                    style_hash: &complete["hash"],
                    reference_hash: &reference["hash"],
                    catalog: &catalog.catalog,
                },
            )?;
        }
        if mode.starts_with("audit-") {
            self.call("get_guidance", json!({"id":"typography"}))?;
            self.call("get_style", json!({"id":"standard-ui"}))?;
            self.call("list_styles", json!({"limit":1}))?;
            let doc = self.call("get_document", json!({}))?["structuredContent"].clone();
            let revision = doc["revision"].as_str().unwrap();
            self.call("apply_operations",json!({"request_id":format!("fixture-audit-edit-{}-{}",self.turns,&revision[..12]),"expected_revision":doc["revision"],"expected_dependency_revision":doc["dependency_revision"],"operations":[{"op":"insert","node":{"type":"rectangle","width":40,"height":20,"fill":"#2563EB"}}]}))?;
            if mode == "audit-corrupt" {
                let path = self
                    .usage_manifest
                    .as_ref()
                    .ok_or("Missing usage manifest")?;
                let active: Value = serde_json::from_slice(&fs::read(path)?)?;
                append_json(
                    path.parent()
                        .unwrap()
                        .join(format!("{}.jsonl", active["turn"].as_str().unwrap())),
                    &json!({"body":"MUST_NOT_LOG"}),
                )?;
            }
            if mode == "audit-cancel" {
                self.response(
                    self.pending.as_ref().unwrap(),
                    json!({"stopReason":"cancelled"}),
                )?;
                return Ok(());
            }
            if mode == "audit-failure" {
                self.send(json!({"id":self.pending.as_ref().unwrap()["id"],"error":{"code":-32603,"message":"fixture failure after design retrieval"}}))?;
                return Ok(());
            }
        }
        if mode.starts_with("image-inspect") {
            let mut resources = Value::Null;
            if mode == "image-inspect-audit" {
                let guide = self.call(
                    "get_guidance",
                    json!({"id":if self.turns==1{"typography"}else{"color"}}),
                )?["structuredContent"]
                    .clone();
                let style =
                    self.call("get_style", json!({"id":"standard-ui"}))?["structuredContent"]
                        .clone();
                self.call("list_styles", json!({"limit":1}))?;
                resources = json!({"guide_id":guide["id"],"guide_hash":guide["hash"],"style_hash":style["hash"]});
            }
            let mut images = Vec::new();
            for block in blocks {
                if block["type"] == "image" {
                    assert_eq!(block["mimeType"], "image/png");
                    let bytes = STANDARD.decode(block["data"].as_str().unwrap())?;
                    assert!(bytes.starts_with(b"\x89PNG"));
                    let name = format!("received-image-{}-{}.png", self.turns, images.len());
                    fs::write(&name, bytes)?;
                    images.push(name);
                }
            }
            write_json(
                format!("received-turn-{}.json", self.turns),
                &json!({"images":images,"text":blocks.iter().filter(|v|v["type"]=="text").map(|v|v["text"].clone()).collect::<Vec<_>>(),"design_resources":resources}),
            )?;
        }
        if ["settings-check", "settings-auth-check"].contains(&mode) {
            assert_eq!(self.model, "fixture-model-2");
            assert_eq!(self.selection, "workspace-write");
        }
        if mode == "auth-change" {
            self.auth("api_key")?;
            return Ok(());
        }
        if mode == "supervised-dynamic" {
            let doc = self.call("get_document", json!({}))?["structuredContent"].clone();
            let preview=self.call("render",json!({"expected_revision":doc["revision"],"expected_dependency_revision":doc["dependency_revision"]}))?;
            let image = preview["content"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["type"] == "image")
                .ok_or("Missing dynamic preview image")?;
            fs::write(
                "supervised-dynamic.png",
                STANDARD.decode(image["data"].as_str().unwrap())?,
            )?;
            write_json("supervised-dynamic.json", &preview["structuredContent"])?;
        }
        if ["resource-observation", "dynamic-resource-observation"].contains(&mode) {
            let dynamic = mode == "dynamic-resource-observation";
            let doc = self.call("get_document", json!({}))?["structuredContent"].clone();
            for round in 0..2 {
                if round == 1 {
                    self.resource_barrier("bound-render-again")?;
                }
                // Omit snapshot_id so the second request executes JavaScript
                // again while retaining the real document-bound MCP process.
                let mut arguments = json!({"expected_revision":doc["revision"],"expected_dependency_revision":doc["dependency_revision"]});
                if dynamic {
                    arguments["refresh"] = json!(true);
                }
                let preview = self.call("render", arguments)?;
                let metadata = &preview["structuredContent"];
                assert_eq!(metadata["node_count"], 1000);
                assert_eq!(metadata["width"], 1920);
                assert_eq!(metadata["height"], 1080);
                assert_eq!(metadata["diagnostics"], json!([]));
                assert_eq!(metadata["script_executions"].as_array().unwrap().len(), 1);
                if dynamic {
                    assert_eq!(metadata["dynamic_artifacts"].as_array().unwrap().len(), 2);
                } else {
                    assert_eq!(metadata["dynamic_artifacts"], json!([]));
                }
                let image = preview["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|value| value["type"] == "image")
                    .ok_or("Missing resource preview")?;
                assert_eq!(image["mimeType"], "image/png");
                fs::write(
                    format!("bound-render-{round}.png"),
                    STANDARD.decode(image["data"].as_str().unwrap())?,
                )?;
                write_json(
                    format!("bound-render-{round}.json"),
                    &json!({
                        "adapter_pid":std::process::id(), "mcp_pid":self.mcp.as_ref().unwrap().child.id(),
                        "render":metadata
                    }),
                )?;
            }
            self.resource_barrier("bound-render-release")?;
            self.mcp.take();
            write_json("bound-render-closed.json", &json!({"mcp_reaped":true}))?;
        }
        if mode == "process-tree-pressure" {
            for _ in 0..4096 {
                self.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x".repeat(32768)}}))?;
            }
            return Err("Undrained pipe never applied backpressure".into());
        }
        if [
            "cancel",
            "hang-cancel",
            "cancel-permission",
            "process-tree-hang",
        ]
        .contains(&mode)
        {
            self.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working\n"}}))?;
            if mode != "cancel-permission" {
                return Ok(());
            }
        }
        if [
            "permission",
            "interactive-permission",
            "permission-cancel",
            "cancel-permission",
            "document-permission",
            "correlated-document",
            "configured-document-ask",
            "spoofed-permission",
            "settings-permissions-hold",
        ]
        .contains(&mode)
        {
            let mut tool = json!({"toolCallId":"shell-1","title":"Unrelated shell operation"});
            if mode == "document-permission" {
                tool["name"] = json!(format!("mcp__{}__apply_operations", self.mcp_server));
            }
            if mode == "spoofed-permission" {
                tool["title"] = json!("mcp__lazypencil__apply_operations");
                tool["name"] = json!("Bash");
            }
            let correlated = ["correlated-document", "configured-document-ask"].contains(&mode);
            if correlated {
                self.notify(json!({"sessionUpdate":"tool_call","toolCallId":"shell-1","kind":"execute","status":"pending","_meta":{"is_mcp_tool_call":true},"rawInput":{"server":self.mcp_server,"tool":"apply_operations","arguments":{}}}))?;
            }
            let mut params = json!({"sessionId":"fixture-native","toolCall":tool,"options":[{"optionId":"allow","name":"Allow once","kind":"allow_once"},{"optionId":"deny","name":"Reject once","kind":"reject_once"}]});
            if correlated {
                params["_meta"] = json!({"is_mcp_tool_approval":true});
            }
            self.send(json!({"id":500,"method":"session/request_permission","params":params}))?;
            return Ok(());
        }
        if ["edit", "image-edit", "edit-cancel"].contains(&mode) || mode.starts_with("resume-") {
            assert!(text.contains("日本語") && text.contains("get_document"));
            let doc = self.call("get_document", json!({}))?["structuredContent"].clone();
            let mut fill = json!("#E65100");
            if mode == "image-edit" {
                let images = blocks
                    .iter()
                    .filter(|v| v["type"] == "image")
                    .collect::<Vec<_>>();
                assert_eq!(images.len(), 1);
                assert_eq!(images[0]["mimeType"], "image/png");
                assert!(
                    STANDARD
                        .decode(images[0]["data"].as_str().unwrap())?
                        .starts_with(b"\x89PNG")
                );
                let attachment = &doc["attachments"]["images"][0];
                assert!(text.contains(attachment["id"].as_str().unwrap()));
                let asset = self.call(
                    "put_asset",
                    json!({"kind":"image","source":{"attachment_id":attachment["id"]}}),
                )?["structuredContent"]
                    .clone();
                fill = json!({"type":"image","url":asset["uri"]});
            }
            let applied=self.call("apply_operations",json!({"request_id":format!("fixture-{}",SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()),"expected_revision":doc["revision"],"operations":[{"op":"insert","node":{"type":"rectangle","name":"日本語の変更","x":0,"y":0,"width":80,"height":40,"fill":fill}}]}))?;
            assert_ne!(applied["structuredContent"]["revision"], doc["revision"]);
            if mode == "image-edit" {
                let current = self.call("get_document", json!({}))?["structuredContent"].clone();
                let preview=self.call("render",json!({"expected_revision":current["revision"],"expected_dependency_revision":current["dependency_revision"]}))?;
                assert!(preview["content"].as_array().unwrap().iter().any(|v| {
                    v["type"] == "image"
                        && STANDARD
                            .decode(v["data"].as_str().unwrap())
                            .unwrap()
                            .starts_with(b"\x89PNG")
                }));
            }
            if mode == "edit-cancel" {
                self.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"committed\n"}}))?;
                return Ok(());
            }
        }
        if mode == "env" {
            for name in [
                "OPENAI_API_KEY",
                "CODEX_API_KEY",
                "ANTHROPIC_API_KEY",
                "DEFAULT_AUTH_REQUEST",
                "ANTHROPIC_BASE_URL",
            ] {
                assert!(std::env::var_os(name).is_none());
            }
        }
        if mode == "unknown-notification" {
            self.send(json!({"method":"_fixture/future","params":{"version":2}}))?;
            self.notify(json!({"sessionUpdate":"future_update","value":"preserve me"}))?;
        }
        self.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("完了 {}\n",self.turns)}}))?;
        self.response(
            self.pending.as_ref().unwrap(),
            json!({"stopReason":"end_turn"}),
        )?;
        Ok(())
    }

    fn resource_barrier(&self, name: &str) -> Result {
        // Permit the separate 30-second all-process idle observation plus
        // bounded rendering/observer work before the owner releases the turn.
        let deadline = Instant::now() + Duration::from_secs(60);
        while !Path::new(name).is_file() {
            if Instant::now() >= deadline {
                return Err(format!("Resource observer did not release {name}").into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

pub fn run(mode: &str) -> Result {
    let path = std::env::current_dir()?.join("fixture-errors.log");
    let mut errors = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut file) = fs::OpenOptions::new().append(true).open(&path) {
            let _ = writeln!(file, "{info}");
        }
        previous(info);
    }));
    let result = (|| {
        let mut peer = Peer::new(mode);
        for line in std::io::stdin().lock().lines() {
            peer.message(serde_json::from_str(&line?)?)?;
        }
        Ok(())
    })();
    if let Err(error) = &result {
        let _ = writeln!(errors, "{error}");
    }
    result
}
