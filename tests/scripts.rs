use lazypencil::script::{ScriptRequest, ScriptRuntime, header::Header};
use serde_json::json;
use std::time::{Duration, Instant};

fn request(body: &str) -> ScriptRequest {
    ScriptRequest {
        code: format!("/**\n * @schema 2.21\n */\n{body}"),
        width: 120.0,
        height: 80.0,
        inputs: Default::default(),
    }
}
fn runtime() -> ScriptRuntime {
    ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil"))
}

#[test]
fn configured_limits_control_the_real_worker_and_invalid_updates_preserve_its_policy() {
    use lazypencil::config::runtime::ScriptSettings;
    let mut worker = runtime();
    let policy = ScriptSettings {
        timeout_ms: 50,
        heap_mib: 4,
        idle_seconds: 2,
        ..Default::default()
    };
    worker.configure(policy).unwrap();
    let started = Instant::now();
    assert_eq!(
        worker.execute(&request("for (;;) {} ")).unwrap_err().code,
        "RENDER_TIMEOUT"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(worker.process_id(), None);
    assert_eq!(
        worker
            .execute(&request("try { for (;;) {} } catch (_) {} return []; "))
            .unwrap_err()
            .code,
        "RENDER_TIMEOUT"
    );
    worker
        .configure(ScriptSettings {
            timeout_ms: 2000,
            ..policy
        })
        .unwrap();
    assert!(
        worker
            .execute(&request(
                "const values=[];for(;;)values.push(new Array(16384).fill('allocated')); "
            ))
            .is_err()
    );
    worker
        .configure(ScriptSettings {
            heap_mib: 16,
            timeout_ms: 2000,
            ..policy
        })
        .unwrap();
    let before = Instant::now();
    assert_eq!(
        worker
            .execute(&request("return [{type:'rectangle',width:7}];"))
            .unwrap()
            .nodes[0]["width"],
        7
    );
    let after = Instant::now();
    let pid = worker.process_id();
    let deadline = worker.idle_deadline().unwrap();
    assert!(deadline >= before + Duration::from_secs(2));
    assert!(deadline <= after + Duration::from_secs(2));
    let saved = worker.settings();
    assert_eq!(
        worker
            .configure(ScriptSettings {
                timeout_ms: 0,
                ..saved
            })
            .unwrap_err()
            .code,
        "CONFIGURATION_ERROR"
    );
    assert_eq!(worker.settings(), saved);
    assert_eq!(worker.process_id(), pid);
    worker
        .configure(ScriptSettings {
            idle_seconds: 0,
            ..saved
        })
        .unwrap();
    assert_eq!(worker.process_id(), None);
    worker.execute(&request("return []; ")).unwrap();
    worker.expire_idle(Instant::now());
    assert_eq!(worker.process_id(), None);
}

#[test]
fn an_absolute_host_deadline_bounds_the_worker_and_allows_following_requests() {
    let mut unavailable = ScriptRuntime::new("a-worker-that-must-not-be-started");
    assert_eq!(
        unavailable
            .execute_until(
                &request("return [];"),
                Some(Instant::now() - Duration::from_millis(1))
            )
            .unwrap_err()
            .code,
        "RENDER_TIMEOUT"
    );
    assert_eq!(unavailable.process_id(), None);
    let mut worker = runtime();
    let started = Instant::now();
    assert_eq!(
        worker
            .execute_until(
                &request("for (;;) {}"),
                Some(started + Duration::from_millis(300))
            )
            .unwrap_err()
            .code,
        "RENDER_TIMEOUT"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(worker.process_id(), None);
    assert_eq!(
        worker
            .execute(&request("return [{type:'rectangle',width:7}];"))
            .unwrap()
            .nodes[0]["width"],
        7
    );
}

#[test]
fn the_internal_worker_protocol_validates_policy_before_executing_source() {
    use lazypencil::config::runtime::ScriptSettings;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("__script_worker")
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    for input in [
        json!({"protocol":999,"limits":ScriptSettings::default(),"request":request("for (;;) {}")}),
        json!({"protocol":1,"limits":{"heap_mib":1024},"request":request("for (;;) {}")}),
        json!({"protocol":1,"limits":ScriptSettings::default(),"request":request("return [{type:'rectangle',width:7}];")}),
    ] {
        writeln!(child.stdin.as_mut().unwrap(), "{input}").unwrap();
    }
    drop(child.stdin.take());
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let replies = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["message"], "Unsupported script worker protocol");
    assert_eq!(replies[0]["timeout"], false);
    assert_eq!(replies[1]["message"], "Invalid script worker limits");
    assert_eq!(replies[1]["timeout"], false);
    assert_eq!(replies[2]["output"]["nodes"][0]["width"], 7);
}

#[test]
fn typed_headers_clamp_numbers_preserve_strings_and_validate_all_six_input_types() {
    let header = Header::parse(
        r##"/**
        * @schema 2.21
        * @input count: number(min=1, max=10) = 40
        * @input title: string = "日本語 and \"quotes\""
        * @input enabled: boolean = true
        * @input color: color = #2563EB
        * @input mode: enum("a,b", "closing)parenthesis") = "a,b"
        * @input component: ref
        */
        return [];
    "##,
    )
    .unwrap();
    let inputs = header
        .prepare(
            json!({"count":-2,"component":"button"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(inputs["count"].as_f64(), Some(1.0));
    assert_eq!(inputs["color"], "#2563EB");
    assert_eq!(inputs["mode"], "a,b");
    assert_eq!(inputs["enabled"], true);
    assert_eq!(inputs["component"], "button");
    assert!(
        !header
            .prepare(&Default::default())
            .unwrap()
            .contains_key("component")
    );
    for input in [
        json!({"enabled":"true"}),
        json!({"mode":"unknown"}),
        json!({"count":"two"}),
        json!({"color":"bad"}),
        json!({"missing":1}),
    ] {
        assert!(header.prepare(input.as_object().unwrap()).is_err());
    }
}

#[test]
fn malformed_headers_are_rejected_without_evaluating_input_declarations() {
    for code in [
        "return [];",
        "/** @schema 9 */ return [];",
        "/**\n@schema 2.21\n@schema 2.21\n*/return [];",
        "/**\n@schema 2.21\n@input n: number(min=3,max=1)\n*/return [];",
        "/**\n@schema 2.21\n@input n: string = process.exit()\n*/return [];",
        "/**\n@schema 2.21\n@input n: boolean = true\n@input n: boolean = false\n*/return [];",
        "/**\n@schema 2.21\n@input n: number(min=NaN)\n*/return [];",
    ] {
        assert!(Header::parse(code).is_err(), "{code}");
    }
}

#[test]
fn a_real_worker_uses_dimensions_inputs_and_isolated_host_free_globals() {
    let mut worker = runtime();
    let output=worker.execute(&request(r#"
        for (const name of ['process','require','fetch','document','window','setTimeout','std','os']) {
          if (typeof globalThis[name] !== 'undefined') throw Error(name + ' should not exist');
        }
        return [{type:'rectangle',width:pencil.width,height:pencil.height,fill:'#2563EB'}];
    "#)).unwrap();
    assert_eq!(output.nodes[0]["width"], 120);
    assert_eq!(output.nodes[0]["height"], 80);
    assert!(output.reads_width && output.reads_height);
    let mut input = request("return [{type:'text',content:pencil.input.title}];");
    input.code = input.code.replace(
        "@schema 2.21",
        "@schema 2.21\n * @input title: string = \"Default\"",
    );
    input.inputs.insert("title".into(), json!("Provided"));
    assert_eq!(
        worker.execute(&input).unwrap().nodes[0]["content"],
        "Provided"
    );
}

#[test]
fn process_reuse_resets_the_js_context_and_random_sequence() {
    let mut worker = runtime();
    let code = request(
        "globalThis.counter=(globalThis.counter||0)+1; return [{type:'rectangle',width:Math.random(),height:globalThis.counter}];",
    );
    let a = worker.execute(&code).unwrap();
    let b = worker.execute(&code).unwrap();
    assert_eq!(a.nodes, b.nodes);
    assert_eq!(a.nodes[0]["height"], 1);
}

#[test]
fn idle_script_processes_are_reused_until_expiry_reaped_and_restarted() {
    let mut worker = runtime();
    assert_eq!(worker.idle_deadline(), None);
    assert_eq!(worker.process_id(), None);
    let code = request("return [{type:'rectangle',width:Math.random(),height:20}];");
    let before = Instant::now();
    let first = worker.execute(&code).unwrap();
    let after = Instant::now();
    let pid = worker.process_id().unwrap();
    let first_deadline = worker.idle_deadline().unwrap();
    assert!(first_deadline >= before + Duration::from_secs(60));
    assert!(first_deadline <= after + Duration::from_secs(60));
    worker.expire_idle(first_deadline - Duration::from_nanos(1));
    assert_eq!(worker.process_id(), Some(pid));
    assert_eq!(worker.execute(&code).unwrap().nodes, first.nodes);
    let next_deadline = worker.idle_deadline().unwrap();
    assert!(next_deadline > first_deadline);
    let mut invalid = request("return [];");
    invalid.code = "missing header".into();
    assert!(worker.execute(&invalid).is_err());
    assert_eq!(worker.idle_deadline(), Some(next_deadline));
    worker.expire_idle(first_deadline);
    assert_eq!(worker.process_id(), Some(pid));
    worker.expire_idle(next_deadline);
    assert_eq!(worker.process_id(), None);
    assert_eq!(worker.idle_deadline(), None);
    #[cfg(unix)]
    {
        assert_eq!(
            unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
    assert_eq!(worker.execute(&code).unwrap().nodes, first.nodes);
    assert_ne!(worker.process_id(), Some(pid));
    assert!(
        worker
            .execute(&request("throw Error('recoverable');"))
            .is_err()
    );
    assert!(worker.idle_deadline().is_some());
    worker.expire_idle(worker.idle_deadline().unwrap());
    assert_eq!(worker.process_id(), None);
    assert_eq!(worker.execute(&code).unwrap().nodes, first.nodes);
}

#[test]
fn invalid_or_async_generated_output_never_becomes_a_successful_reply() {
    let mut worker = runtime();
    for body in [
        "return {};",
        "return Promise.resolve([]);",
        "Promise.resolve().then(()=>{});return [];",
        "return [{type:'rectangle',reusable:true}];",
        "return [{type:'future-node'}];",
        "return [{type:'rectangle',width:NaN}];",
        "const a=[]; a.push(a);return a;",
        "return new Array(1001).fill({type:'rectangle'});",
        "throw new Error('meaningful failure');",
        "return [{type:'rectangle',data:undefined}];",
        "return [{type:'rectangle',data:()=>1}];",
        "return [{type:'text',content:'x'.repeat(5*1024*1024)}];",
        "return [{type:'script',children:[{type:'rectangle'}]}];",
    ] {
        assert!(worker.execute(&request(body)).is_err(), "{body}");
    }
    assert!(
        worker
            .execute(&request("return [];"))
            .unwrap()
            .nodes
            .is_empty()
    );
}

#[test]
fn time_and_memory_limits_do_not_stop_later_requests() {
    let mut worker = runtime();
    let start = Instant::now();
    assert_eq!(
        worker.execute(&request("for (;;) {}")).unwrap_err().code,
        "RENDER_TIMEOUT"
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(
        worker
            .execute(&request(
                "const a=[];for (;;) a.push(new Array(16384).fill('allocated'));"
            ))
            .is_err()
    );
    assert_eq!(
        worker
            .execute(&request("return [{type:'rectangle',width:1}];"))
            .unwrap()
            .nodes
            .len(),
        1
    );
}

#[test]
fn cancelling_an_executing_worker_terminates_it_and_allows_a_new_request() {
    let mut worker = runtime();
    let cancel = tokio_util::sync::CancellationToken::new();
    worker.set_cancellation(Some(cancel.clone()));
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        cancel.cancel();
    });
    let start = Instant::now();
    assert_eq!(
        worker.execute(&request("for (;;) {}")).unwrap_err().code,
        "CANCELLED"
    );
    thread.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    worker.set_cancellation(None);
    assert!(
        worker
            .execute(&request("return [];"))
            .unwrap()
            .nodes
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn an_unresponsive_process_is_killed_by_the_parent_watchdog() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let temp = tempfile::TempDir::new().unwrap();
    let file = temp.path().join("unresponsive");
    fs::write(&file, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    let mut worker = ScriptRuntime::new(&file);
    let start = Instant::now();
    assert_eq!(
        worker.execute(&request("return [];")).unwrap_err().code,
        "RENDER_TIMEOUT"
    );
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn explicit_supported_schema_headers_execute_without_rewriting_the_source() {
    let mut worker = runtime();
    for version in ["2.19", "2.21"] {
        let mut input = request(
            "return [{type:'rectangle',width:pencil.width,height:pencil.height,fill:'#ff0000'}];",
        );
        input.code = input.code.replace("2.21", version);
        assert_eq!(Header::parse(&input.code).unwrap().schema, version);
        let output = worker.execute(&input).unwrap();
        assert_eq!(output.nodes[0]["width"], 120);
        assert_eq!(output.nodes[0]["height"], 80);
    }
    for version in ["2.18", "2.20", "2.22"] {
        assert_eq!(
            Header::parse(&format!("/** @schema {version} */ return [];"))
                .unwrap_err()
                .code,
            "UNSUPPORTED_VERSION"
        );
    }
}
