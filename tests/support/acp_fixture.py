"""Deterministic ACP peer for process, frontend, and real MCP integration tests."""

import base64
import json
import os
import subprocess
import sys
import time


mode = sys.argv[1]
sys.stderr = open("fixture-errors.log", "a", encoding="utf-8")
server = None
usage_manifest = None
pending = None
turns = 0
permission_answered = False
cancel_received = False
selected_model = "fixture-model"
selected_mode = "read-only"
authenticated = False
native_session = "fixture-native"


def config_options():
    return [{"id": "model", "name": "Model", "category": "model", "type": "select",
        "currentValue": selected_model, "options": [{"group": "fixture", "name": "Fixture", "options": [
            {"value": "fixture-model", "name": "Fixture"}, {"value": "fixture-model-2", "name": "Fixture 2"}]}]}]


def send(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)


def response(message, result):
    send({"id": message["id"], "result": result})


def notify(update):
    send({"method": "session/update", "params": {
        "sessionId": native_session, "update": update,
    }})


def auth_status(kind):
    send({"method": "_auth/status_update", "params": {"authStatus": {
        "kind": kind, "label": "Fixture account", "account": {
            "plan": "test-plan", "email": "must-not-log@example.test"},
        "accessToken": "fixture-not-a-token", "vendor": {"secret": "must-not-log"}}}})


def call_mcp(name, arguments):
    number = call_mcp.number
    call_mcp.number += 1
    server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": number,
        "method": "tools/call", "params": {"name": name, "arguments": arguments}}) + "\n")
    server.stdin.flush()
    while True:
        line = server.stdout.readline()
        if not line:
            raise RuntimeError("MCP server terminated")
        value = json.loads(line)
        if value.get("id") == number:
            assert "error" not in value, value
            assert not value["result"].get("isError"), value
            return value["result"]


call_mcp.number = 2


def connect_mcp(config):
    global server, usage_manifest
    assert os.path.isabs(config["command"]), config
    if "--guidance-audit-manifest" in config["args"]:
        usage_manifest = config["args"][config["args"].index("--guidance-audit-manifest") + 1]
    environment = {**os.environ, **{item["name"]: item["value"] for item in config.get("env", [])}}
    server = subprocess.Popen([config["command"], *config["args"]], env=environment,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True, encoding="utf-8")
    server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "acp-fixture", "version": "1"}}}) + "\n")
    server.stdin.flush()
    assert json.loads(server.stdout.readline())["result"]["protocolVersion"] == "2025-11-25"
    server.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
    server.stdin.flush()


try:
    for line in sys.stdin:
        message = json.loads(line)
        method = message.get("method")
        params = message.get("params", {})
        if mode == "malformed":
            print("not json", flush=True)
            continue
        if mode == "bad-envelope":
            print('{"jsonrpc":"2.0","unexpected":"not an RPC message"}', flush=True)
            continue
        if mode == "crash":
            sys.exit(9)
        if method == "initialize":
            if mode == "setup-hang":
                continue
            assert params["protocolVersion"] == 1
            capabilities = params["clientCapabilities"]
            assert not capabilities.get("terminal")
            assert not capabilities.get("fs", {}).get("writeTextFile")
            agent_capabilities = {"loadSession": mode != "no-load" and not (
                mode == "resume-no-load" and os.path.exists("fixture-restart")),
                "promptCapabilities": {"image": mode != "no-images"}}
            if mode.startswith("auth-"):
                agent_capabilities["_meta"] = {"authStatus": {}}
            response(message, {"protocolVersion": 2 if mode == "wrong-version" else 1,
                "agentCapabilities": agent_capabilities,
                "agentInfo": {"name": "acp-fixture", "version": "1"},
                "authMethods": [{"id": "fixture-auth", "name": "Fixture"}]})
        elif method == "authenticate":
            assert params["methodId"] == "fixture-auth"
            authenticated = True
            response(message, {})
        elif method in ["session/new", "session/load"]:
            if mode.startswith("resume-"):
                with open("received-session-methods.jsonl", "a", encoding="utf-8") as handle:
                    handle.write(json.dumps({"method": method, "params": params}) + "\n")
                if method == "session/load" and mode == "resume-load-error" and os.path.exists("fixture-restart"):
                    send({"id": message["id"], "error": {"code": -32603, "message": "fixture session storage failure"}})
                    continue
                if method == "session/new" and os.path.exists("fixture-restart"):
                    native_session = "fixture-restarted"
            if mode == "settings-auth-check":
                assert authenticated, "Configured authentication must precede the session request"
            assert os.path.isabs(params["cwd"])
            if method == "session/load":
                assert params["sessionId"] == "fixture-native"
                notify({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "replayed\n"}})
            if mode in ["edit", "image-edit", "edit-cancel", "image-inspect-audit", "user-config", "supervised-dynamic"] or mode.startswith(("project-guidance", "audit-", "resume-")):
                assert len(params["mcpServers"]) == 1
                connect_mcp(params["mcpServers"][0])
            if mode.startswith("auth-") and mode != "auth-absent":
                auth_status({"auth-api": "api_key", "auth-none": "none", "auth-gateway": "gateway", "auth-external": "external", "auth-unknown": "future_kind"}.get(mode, "account"))
            session_result = {"sessionId": native_session} if method == "session/new" else {}
            if mode.startswith("settings-"):
                session_result.update({"modes": {"currentModeId": selected_mode, "availableModes": [
                    {"id": "read-only", "name": "Read only"}, {"id": "workspace-write", "name": "Workspace"}]},
                    "configOptions": config_options()})
            response(message, session_result)
            if mode == "grandchild":
                grandchild = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                with open("fixture-child.pid.tmp", "w") as handle:
                    handle.write(str(grandchild.pid))
                os.replace("fixture-child.pid.tmp", "fixture-child.pid")
            if mode.startswith("process-tree"):
                from pathlib import Path
                flags = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS if os.name == "nt" else 0
                subprocess.Popen([sys.executable, str(Path(__file__).with_name("agent_process_tree.py")),
                    "branch", str(os.getpid()), *(["--guardian", str(os.getppid())] if os.name != "nt" else []),
                    *(["--detach"] if mode == "process-tree-detached" else [])], stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=flags,
                    start_new_session=(os.name != "nt" and mode == "process-tree-detached"))
        elif method == "session/prompt":
            if mode.startswith("resume-"):
                assert params["sessionId"] == native_session
                with open("received-prompts.jsonl", "a", encoding="utf-8") as handle:
                    handle.write(json.dumps(params, ensure_ascii=False) + "\n")
            with open("sent-turn.txt", "w") as handle:
                handle.write("sent")
            pending = message
            turns += 1
            if mode == "user-config":
                document = call_mcp("get_document", {})["structuredContent"]
                configuration = document["configuration"]
                assert configuration["settings"]["fonts"]["default_family"] == "LP Test Sans", configuration
                assert configuration["settings"]["fonts"]["locale"] == "en-US", configuration
                assert configuration["font_origins"]["files"] == "user", configuration
                assert os.path.isabs(configuration["user"]["requested_path"]), configuration
                with open("received-user-configuration.json", "w", encoding="utf-8") as handle:
                    json.dump(configuration, handle, ensure_ascii=False)
            if mode.startswith("project-guidance"):
                texts = "\n".join(block.get("text", "") for block in params["prompt"])
                assert "ACP_PROJECT_RULE" in texts, texts
                assert "REFERENCE_NOT_AUTOMATIC" not in texts, texts
                selected = "content-page" if mode.endswith("override") else "project:brand"
                assert '"id":"' + selected + '"' in texts, texts
                selected_style = json.loads(params["prompt"][0]["text"].rsplit("\n\nSelected style:\n", 1)[1])
                assert "document" not in selected_style["example"], selected_style
                assert selected_style["example"]["retrieval_tool"] == "get_style"
                guidance = call_mcp("get_guidance", {})["structuredContent"]
                assert "ACP_PROJECT_RULE" in guidance["project_guides"][0]["body"], guidance
                complete_style = call_mcp("get_style", {"id": selected})["structuredContent"]
                assert complete_style["id"] == selected
                assert complete_style["hash"] == selected_style["hash"]
                assert "document" in complete_style["example"]
                catalog = call_mcp("list_styles", {})["structuredContent"]
                reference = call_mcp("get_guidance", {"id": "project:reference"})["structuredContent"]
                assert reference["body"] == "REFERENCE_NOT_AUTOMATIC"
                with open("received-project-guidance.json", "w", encoding="utf-8") as handle:
                    json.dump({"text": texts, "workflow_hash": guidance["hash"], "style_hash": complete_style["hash"],
                        "reference_hash": reference["hash"], "catalog": catalog}, handle, ensure_ascii=False)
            if mode.startswith("audit-"):
                call_mcp("get_guidance", {"id": "typography"})
                call_mcp("get_style", {"id": "standard-ui"})
                call_mcp("list_styles", {"limit": 1})
                document = call_mcp("get_document", {})["structuredContent"]
                call_mcp("apply_operations", {"request_id": "fixture-audit-edit-" + str(turns) + "-" + document["revision"][:12],
                    "expected_revision": document["revision"], "expected_dependency_revision": document["dependency_revision"],
                    "operations": [{"op": "insert", "node": {"type": "rectangle", "width": 40, "height": 20, "fill": "#2563EB"}}]})
                if mode == "audit-corrupt":
                    with open(usage_manifest, encoding="utf-8") as handle:
                        active = json.load(handle)
                    journal = os.path.join(os.path.dirname(usage_manifest), active["turn"] + ".jsonl")
                    with open(journal, "a", encoding="utf-8") as handle:
                        handle.write('{"body":"MUST_NOT_LOG"}\n')
                if mode == "audit-cancel":
                    response(message, {"stopReason": "cancelled"})
                    continue
                if mode == "audit-failure":
                    send({"id": message["id"], "error": {"code": -32603, "message": "fixture failure after design retrieval"}})
                    continue
            if mode.startswith("image-inspect"):
                resources = None
                if mode == "image-inspect-audit":
                    guide = call_mcp("get_guidance", {"id": "typography" if turns == 1 else "color"})["structuredContent"]
                    style = call_mcp("get_style", {"id": "standard-ui"})["structuredContent"]
                    call_mcp("list_styles", {"limit": 1})
                    resources = {"guide_id": guide["id"], "guide_hash": guide["hash"], "style_hash": style["hash"]}
                images = []
                for block in params["prompt"]:
                    if block["type"] == "image":
                        assert block["mimeType"] == "image/png"
                        data = base64.b64decode(block["data"], validate=True)
                        assert data.startswith(b"\x89PNG")
                        name = f"received-image-{turns}-{len(images)}.png"
                        with open(name, "wb") as handle:
                            handle.write(data)
                        images.append(name)
                with open(f"received-turn-{turns}.json", "w", encoding="utf-8") as handle:
                    json.dump({"images": images, "text": [block["text"] for block in params["prompt"] if block["type"] == "text"], "design_resources": resources}, handle)
            if mode in ["settings-check", "settings-auth-check"]:
                assert selected_model == "fixture-model-2"
                assert selected_mode == "workspace-write"
            if mode == "auth-change":
                auth_status("api_key")
                continue
            if mode == "supervised-dynamic":
                current = call_mcp("get_document", {})["structuredContent"]
                preview = call_mcp("render", {"expected_revision": current["revision"],
                    "expected_dependency_revision": current["dependency_revision"]})
                image = next(block for block in preview["content"] if block.get("type") == "image")
                with open("supervised-dynamic.png", "wb") as handle:
                    handle.write(base64.b64decode(image["data"], validate=True))
                with open("supervised-dynamic.json", "w", encoding="utf-8") as handle:
                    json.dump(preview["structuredContent"], handle)
            if mode == "process-tree-pressure":
                for _ in range(4096):
                    notify({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x" * 32768}})
                raise RuntimeError("The undrained test pipe never applied backpressure")
            if mode in ["cancel", "hang-cancel", "cancel-permission", "process-tree-hang"]:
                notify({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "working\n"}})
                if mode != "cancel-permission":
                    continue
            if mode in ["permission", "interactive-permission", "permission-cancel", "cancel-permission", "document-permission", "correlated-document", "configured-document-ask", "spoofed-permission"]:
                tool = {"toolCallId": "shell-1", "title": "Unrelated shell operation"}
                if mode == "document-permission":
                    tool["name"] = "mcp__lazypencil__apply_operations"
                if mode == "spoofed-permission":
                    tool["title"] = "mcp__lazypencil__apply_operations"
                    tool["name"] = "Bash"
                if mode in ["correlated-document", "configured-document-ask"]:
                    notify({"sessionUpdate": "tool_call", "toolCallId": "shell-1", "kind": "execute", "status": "pending",
                        "_meta": {"is_mcp_tool_call": True}, "rawInput": {"server": "lazypencil", "tool": "apply_operations", "arguments": {}}})
                send({"id": 500, "method": "session/request_permission", "params": {
                    "sessionId": "fixture-native", "toolCall": tool,
                    **({"_meta": {"is_mcp_tool_approval": True}} if mode in ["correlated-document", "configured-document-ask"] else {}),
                    "options": [{"optionId": "allow", "name": "Allow once", "kind": "allow_once"},
                        {"optionId": "deny", "name": "Reject once", "kind": "reject_once"}]}})
                continue
            if mode in ["edit", "image-edit", "edit-cancel"] or mode.startswith("resume-"):
                texts = "\n".join(block.get("text", "") for block in params["prompt"])
                assert "日本語" in texts
                assert "get_document" in texts
                document = call_mcp("get_document", {})["structuredContent"]
                fill = "#E65100"
                if mode == "image-edit":
                    blocks = [block for block in params["prompt"] if block["type"] == "image"]
                    assert len(blocks) == 1
                    assert blocks[0]["mimeType"] == "image/png"
                    assert base64.b64decode(blocks[0]["data"]).startswith(b"\x89PNG")
                    attachment = document["attachments"]["images"][0]
                    assert attachment["id"] in texts
                    asset = call_mcp("put_asset", {"kind": "image", "source": {"attachment_id": attachment["id"]}})["structuredContent"]
                    fill = {"type": "image", "url": asset["uri"]}
                applied = call_mcp("apply_operations", {"request_id": "fixture-" + str(time.time_ns()),
                    "expected_revision": document["revision"], "operations": [{"op": "insert", "node": {
                        "type": "rectangle", "name": "日本語の変更", "x": 0, "y": 0, "width": 80, "height": 40, "fill": fill}}]})
                assert applied["structuredContent"]["revision"] != document["revision"]
                if mode == "image-edit":
                    current = call_mcp("get_document", {})["structuredContent"]
                    preview = call_mcp("render", {"expected_revision": current["revision"],
                        "expected_dependency_revision": current["dependency_revision"]})
                    assert any(block.get("type") == "image" and base64.b64decode(block["data"]).startswith(b"\x89PNG") for block in preview["content"])
                if mode == "edit-cancel":
                    notify({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "committed\n"}})
                    continue
            if mode == "env":
                assert "OPENAI_API_KEY" not in os.environ
                assert "CODEX_API_KEY" not in os.environ
                assert "ANTHROPIC_API_KEY" not in os.environ
                assert "DEFAULT_AUTH_REQUEST" not in os.environ
                assert "ANTHROPIC_BASE_URL" not in os.environ
            if mode == "unknown-notification":
                send({"method": "_fixture/future", "params": {"version": 2}})
                notify({"sessionUpdate": "future_update", "value": "preserve me"})
            notify({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "完了 " + str(turns) + "\n"}})
            response(message, {"stopReason": "end_turn"})
        elif method == "session/set_mode":
            selected_mode = params["modeId"]
            response(message, {})
        elif method == "session/set_config_option":
            assert params["configId"] == "model"
            selected_model = params["value"]
            response(message, {"configOptions": config_options()})
        elif method == "session/cancel":
            cancel_received = True
            if mode in ["hang-cancel", "process-tree-hang"]:
                continue
            if mode == "cancel-permission" and not permission_answered:
                continue
            if pending:
                response(pending, {"stopReason": "cancelled"})
                pending = None
        elif message.get("id") == 500:
            outcome = message["result"]["outcome"]
            if mode == "cancel-permission":
                assert outcome["outcome"] == "cancelled", outcome
                permission_answered = True
                if cancel_received:
                    response(pending, {"stopReason": "cancelled"})
                    pending = None
            else:
                assert outcome["outcome"] == "selected", outcome
                assert outcome["optionId"] == ("allow" if mode in ["interactive-permission", "document-permission", "correlated-document"] else "deny"), outcome
                response(pending, {"stopReason": "cancelled" if mode == "permission-cancel" else "end_turn"})
                pending = None
finally:
    if server:
        server.stdin.close()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
