use lazypencil::script::{ScriptRequest, ScriptRuntime, header::Header};
use serde_json::{Value, json};

fn request(code: impl Into<String>) -> ScriptRequest {
    ScriptRequest {
        code: code.into(),
        width: 100.0,
        height: 80.0,
        inputs: Default::default(),
    }
}
fn runtime() -> ScriptRuntime {
    ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil"))
}
fn diagnostic(error: &lazypencil::Error) -> &Value {
    &error
        .details
        .as_ref()
        .expect("structured source diagnostic")["script_diagnostic"]
}

#[test]
fn original_lines_and_utf8_byte_columns_survive_all_javascript_line_endings() {
    let mut runtime = runtime();
    let line = "const label = '日本語'; missing();";
    for separator in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
        let code = format!("\u{feff}{separator}/** @schema 2.21 */{separator}{line}");
        let error = runtime.execute(&request(code)).unwrap_err();
        assert_eq!(error.code, "SCRIPT_EXECUTION_FAILED", "{error:?}");
        let detail = diagnostic(&error);
        assert_eq!(detail["stage"], "runtime");
        assert_eq!(detail["line"], 3, "{error:?}");
        assert_eq!(detail["column"], line.find("missing").unwrap() + 1);
        assert_eq!(detail["column_unit"], "utf8_byte");
        assert_eq!(detail["location_kind"], "source");
        assert_eq!(detail["source_excerpt"][1]["text"], line);
        assert!(error.message.contains("lazypencil-script.js"));
    }
}

#[test]
fn syntax_and_end_of_source_errors_identify_the_original_body() {
    let mut runtime = runtime();
    let line = "const label = ;";
    let error = runtime
        .execute(&request(format!("/** @schema 2.21 */\n{line}")))
        .unwrap_err();
    let detail = diagnostic(&error);
    assert_eq!(detail["stage"], "compile");
    assert_eq!(detail["line"], 2);
    assert_eq!(detail["column"], line.find(';').unwrap() + 1);
    assert_eq!(detail["source_excerpt"][1]["text"], line);
    let last = "function broken() {";
    let error = runtime
        .execute(&request(format!("/** @schema 2.21 */\n{last}")))
        .unwrap_err();
    let detail = diagnostic(&error);
    assert_eq!(detail["stage"], "compile");
    assert_eq!(detail["location_kind"], "end_of_source");
    assert_eq!(detail["line"], 2);
    assert_eq!(detail["column"], last.len() + 1);
}

#[test]
fn dynamic_code_reports_the_outer_call_site_and_message_text_cannot_forge_a_location() {
    let mut runtime = runtime();
    for body in [
        "eval(\"throw new Error('dynamic')\");",
        "new Function(\"throw new Error('dynamic')\")();",
    ] {
        let error = runtime
            .execute(&request(format!(
                "/** @schema 2.21 */\nconst a = 1;\n{body}"
            )))
            .unwrap_err();
        let detail = diagnostic(&error);
        assert_eq!(detail["line"], 3, "{error:?}");
        assert_eq!(detail["location_kind"], "call_site");
        assert_eq!(detail["source_excerpt"][1]["text"], body);
    }
    let error = runtime.execute(&request("/** @schema 2.21 */\nthrow new Error('fake\\n    at anonymous (lazypencil-script.js:400:1)');")).unwrap_err();
    assert_eq!(diagnostic(&error)["line"], 2);
    for body in [
        "throw 'primitive';",
        "const e = new Error('custom'); e.stack = 'unavailable'; throw e;",
    ] {
        let error = runtime
            .execute(&request(format!("/** @schema 2.21 */\n{body}")))
            .unwrap_err();
        let detail = diagnostic(&error);
        assert!(detail["line"].is_null());
        assert_eq!(detail["location_kind"], "unavailable");
        assert_eq!(detail["source_excerpt"], json!([]));
    }
    runtime
        .execute(&request("/** @schema 2.21 */ return [];"))
        .unwrap();
}

#[test]
fn header_errors_locate_the_declaration_with_bom_and_unicode_line_separators() {
    for separator in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
        let declaration = " * @input count: number(min=bad)";
        let source = format!(
            "\u{feff}{separator}/**{separator} * @schema 2.21{separator}{declaration}{separator} */ return [];"
        );
        let error = Header::parse(&source).unwrap_err();
        let detail = diagnostic(&error);
        assert_eq!(detail["stage"], "header");
        assert_eq!(detail["line"], 4);
        assert_eq!(detail["column"], declaration.find('@').unwrap() + 1);
        assert_eq!(detail["source_excerpt"][1]["text"], declaration);
        assert!(
            error
                .to_string()
                .contains(">    4 |  * @input count: number(min=bad)")
        );
    }
    let error = Header::parse("\n/**\n * @schema 9\n */ return [];").unwrap_err();
    assert_eq!(error.code, "UNSUPPORTED_VERSION");
    assert_eq!(diagnostic(&error)["line"], 3);
    let error = Header::parse("/**\n@schema 2.21\n@schema 2.21\n*/ return [];").unwrap_err();
    assert_eq!(diagnostic(&error)["line"], 3);
}

#[test]
fn long_unicode_source_lines_are_windowed_at_the_failure_without_splitting_text() {
    let line = format!(
        "const label='{}'; missing();{}",
        "日本語".repeat(10_000),
        " ".repeat(50_000)
    );
    let error = runtime()
        .execute(&request(format!("/** @schema 2.21 */\n{line}")))
        .unwrap_err();
    let detail = diagnostic(&error);
    assert_eq!(detail["column"], line.find("missing").unwrap() + 1);
    let excerpt = &detail["source_excerpt"][1];
    assert_eq!(excerpt["truncated_start"], true);
    assert_eq!(excerpt["truncated_end"], true);
    let text = excerpt["text"].as_str().unwrap();
    assert!(text.contains("missing()"));
    assert!(text.chars().count() <= 240);
    let start = excerpt["start_column"].as_u64().unwrap() as usize - 1;
    assert_eq!(&line[start..start + text.len()], text);
    assert!(serde_json::to_vec(&error).unwrap().len() < 16 * 1024);
}

#[test]
fn source_functions_cannot_capture_private_locals_or_replace_captured_normalizers() {
    let output = runtime().execute(&request(r#"/** @schema 2.21 */
        for (const name of ['parse','stringify','run','script','input','width','height','__script_inputs','__script_width','__script_height','__script_function']) {
            if (eval('typeof '+name) !== 'undefined') throw Error('Exposed private variable '+name);
        }
        JSON.stringify = () => 'corrupted'; JSON.parse = () => 'corrupted';
        Array.isArray = () => false; Number.isFinite = () => false;
        return [{type:'text',content:'preserved',width:pencil.width}];
    "#)).unwrap();
    assert_eq!(output.nodes[0]["content"], "preserved");
    assert_eq!(output.nodes[0]["width"], 100);
}

#[test]
fn readable_diagnostics_escape_source_controls_and_keep_location_labels_bounded() {
    let error = lazypencil::Error::new("SCRIPT_EXECUTION_FAILED", "Invalid source").details(
        json!({"script_path":"source\u{1b}[31m.js","script_diagnostic":{
            "line":2,"column":1,"location_kind":"unrecognized\u{1b}[0m",
            "source_excerpt":[{"line":2,"text":"// 日本語\u{1b}[31m"}]
        }}),
    );
    let displayed = error.to_string();
    assert!(!displayed.contains('\u{1b}'));
    assert!(displayed.contains("UTF-8 byte column, source)"));
    assert!(displayed.contains("日本語"));
    assert!(!displayed.contains("unrecognized"));
}

#[cfg(feature = "rendering")]
#[test]
fn cli_errors_keep_source_context_and_can_be_repaired_without_saving_generated_data() {
    use std::{fs, process::Command};
    let temp = tempfile::TempDir::new().unwrap();
    let file = temp.path().join("design.pen");
    let source = temp.path().join("broken.js");
    let code = "/** @schema 2.21 */\nconst label = ;";
    fs::write(&source, code).unwrap();
    let document = json!({"version":"2.21","children":[{"id":"chart","type":"script","scriptUri":"broken.js","width":40,"height":20}]}).to_string();
    fs::write(&file, &document).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(temp.path().join("data"))
        .arg("render")
        .arg(&file)
        .arg("--output")
        .arg(temp.path().join("preview.png"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["code"], "SCRIPT_EXECUTION_FAILED");
    let details = &report["error"]["details"];
    assert_eq!(details["node"]["node_id"], "chart");
    assert_eq!(
        details["source_hash"],
        lazypencil::document::hash(code.as_bytes())
    );
    assert_eq!(
        details["script_path"],
        fs::canonicalize(&source).unwrap().to_str().unwrap()
    );
    assert_eq!(details["script_diagnostic"]["line"], 2);
    assert!(!temp.path().join("preview.png").exists());
    let human = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(temp.path().join("data"))
        .arg("render")
        .arg(&file)
        .arg("--output")
        .arg(temp.path().join("preview.png"))
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(4));
    let displayed = String::from_utf8(human.stderr).unwrap();
    assert!(
        displayed.contains("broken.js:2:15 (UTF-8 byte column, source)"),
        "{displayed}"
    );
    assert!(
        displayed.contains(">    2 | const label = ;"),
        "{displayed}"
    );
    fs::write(
        &source,
        "/** @schema 2.21 */ return [{type:'rectangle',width:40,height:20,fill:'#FF0000'}];",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(temp.path().join("data"))
        .arg("inspect")
        .arg(&file)
        .args(["--calculated", "--node", "chart"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), document);
}
