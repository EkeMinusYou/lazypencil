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
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_id: u64,
}
impl Mcp {
    fn connect(config: &Value) -> Result<Self> {
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
            child,
            input,
            output,
            next_id: 2,
        };
        result.send(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acp-fixture","version":"1"}}}))?;
        assert_eq!(result.read()?["result"]["protocolVersion"], "2025-11-25");
        result.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
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
    fn call_raw(&mut self, name: &str, args: Value) -> Result<Box<RawValue>> {
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
    selection: String,
    authenticated: bool,
    session: String,
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
            selection: "read-only".into(),
            authenticated: false,
            session: "fixture-native".into(),
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
        self.send(json!({"method":"_auth/status_update","params":{"authStatus":{"kind":kind,"label":"Fixture account","account":{"plan":"test-plan","email":"must-not-log@example.test"},"accessToken":"fixture-not-a-token","vendor":{"secret":"must-not-log"}}}}))
    }
    fn config(&self) -> Value {
        json!([{"id":"model","name":"Model","category":"model","type":"select","currentValue":self.model,"options":[{"group":"fixture","name":"Fixture","options":[{"value":"fixture-model","name":"Fixture"},{"value":"fixture-model-2","name":"Fixture 2"}]}]}])
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
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
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
                self.response(&message,json!({"protocolVersion":if mode=="wrong-version"{2}else{1},"agentCapabilities":caps,"agentInfo":{"name":"acp-fixture","version":"1"},"authMethods":[{"id":"fixture-auth","name":"Fixture"}]}))?;
            }
            "authenticate" => {
                assert_eq!(params["methodId"], "fixture-auth");
                self.authenticated = true;
                self.response(&message, json!({}))?;
            }
            "session/new" | "session/load" => {
                let method = message["method"].as_str().unwrap();
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
                    self.mcp = Some(Mcp::connect(config)?);
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
                if mode.starts_with("settings-") {
                    result["modes"] = json!({"currentModeId":self.selection,"availableModes":[{"id":"read-only","name":"Read only"},{"id":"workspace-write","name":"Workspace"}]});
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
            "session/set_mode" => {
                self.selection = params["modeId"]
                    .as_str()
                    .ok_or("Missing selected mode")?
                    .into();
                self.response(&message, json!({}))?;
            }
            "session/set_config_option" => {
                assert_eq!(params["configId"], "model");
                self.model = params["value"]
                    .as_str()
                    .ok_or("Missing selected model")?
                    .into();
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
        let mode = self.mode;
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
        if mode == "resource-observation" {
            let doc = self.call("get_document", json!({}))?["structuredContent"].clone();
            for round in 0..2 {
                if round == 1 {
                    self.resource_barrier("bound-render-again")?;
                }
                // Omit snapshot_id so the second request executes JavaScript
                // again while retaining the real document-bound MCP process.
                let preview = self.call("render", json!({"expected_revision":doc["revision"],"expected_dependency_revision":doc["dependency_revision"]}))?;
                let metadata = &preview["structuredContent"];
                assert_eq!(metadata["node_count"], 1000);
                assert_eq!(metadata["width"], 1920);
                assert_eq!(metadata["height"], 1080);
                assert_eq!(metadata["diagnostics"], json!([]));
                assert_eq!(metadata["script_executions"].as_array().unwrap().len(), 1);
                assert_eq!(metadata["dynamic_artifacts"], json!([]));
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
        ]
        .contains(&mode)
        {
            let mut tool = json!({"toolCallId":"shell-1","title":"Unrelated shell operation"});
            if mode == "document-permission" {
                tool["name"] = json!("mcp__lazypencil__apply_operations");
            }
            if mode == "spoofed-permission" {
                tool["title"] = json!("mcp__lazypencil__apply_operations");
                tool["name"] = json!("Bash");
            }
            let correlated = ["correlated-document", "configured-document-ask"].contains(&mode);
            if correlated {
                self.notify(json!({"sessionUpdate":"tool_call","toolCallId":"shell-1","kind":"execute","status":"pending","_meta":{"is_mcp_tool_call":true},"rawInput":{"server":"lazypencil","tool":"apply_operations","arguments":{}}}))?;
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
