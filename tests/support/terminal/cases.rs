use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use sha2::{Digest, Sha256};

mod conversations;
mod dynamic;

fn hash(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes.as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn source_commit() -> Value {
    std::env::var("GITHUB_SHA")
        .ok()
        .map(Value::String)
        .unwrap_or(Value::Null)
}
fn config(h: &Harness) -> &Path {
    h.configuration.as_deref().expect("Case configuration")
}
fn state(h: &Harness) -> Value {
    h.session().expect("Saved session")
}
fn no_agent(h: &Harness) {
    assert!(!h.root.join("sent-turn.txt").exists());
    assert!(state(h)["native_session_id"].is_null() || state(h)["native_session_id"] == "");
}
fn status(h: &Harness) -> String {
    let text = h.screen.text();
    text.split('\n').rev().nth(1).unwrap_or_default().to_owned()
}
fn snapshot(h: &Harness, name: &str, offsets: &mut serde_json::Map<String, Value>) -> Result {
    offsets.insert(name.into(), h.output.len().into());
    h.save_screen(&format!("saved-history-{name}-screen.txt"))
}
fn arrow(h: &mut Harness, distance: isize) -> Result {
    for _ in 0..distance.unsigned_abs() {
        h.key(if distance > 0 { b"\x1b[B" } else { b"\x1b[A" })?;
    }
    Ok(())
}
fn command(h: &mut Harness, label: &str, index: usize) -> Result {
    h.key(b"\x1bp")?;
    h.visible(label)?;
    arrow(h, index as isize)?;
    h.key(b"\r")
}
fn edit(h: &Harness, color: &str, request: &str) -> Result<Value> {
    let revision = h.cli(&["inspect".as_ref(), h.document.as_os_str()])?["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    let operations = h.root.join("operations.json");
    write_json(
        &operations,
        &json!([{"op":"update","target":"red","set":{"fill":color}}]),
    )?;
    h.cli(&[
        "apply".as_ref(),
        h.document.as_os_str(),
        "--operations".as_ref(),
        operations.as_os_str(),
        "--expected-revision".as_ref(),
        revision.as_ref(),
        "--request-id".as_ref(),
        request.as_ref(),
    ])
}

pub fn run(
    binary: &Path,
    root: &Path,
    case: &str,
    extra_environment: &[(OsString, OsString)],
) -> Result {
    let mut environment: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    environment.extend(extra_environment.iter().cloned());
    let mut document = root.join("design.pen");
    if case == "session-deletion-small" {
        let parent = root
            .join("long-project-path-".repeat(5))
            .join("long-design-directory-".repeat(5));
        fs::create_dir_all(&parent)?;
        document = parent.join("design.pen");
    }
    let mut design = json!({"version":"2.21","children":[{"type":"rectangle","id":"red","name":"日本語の対象","x":-80,"y":-30,"width":180,"height":90,"fill":"#FF0000"}]});
    let mut configuration = None;
    if matches!(case, "dynamic-idle" | "page-playback") {
        let sequence = if case == "dynamic-idle" {
            "++n<=2?'#f00':'#f0f'"
        } else {
            "++n<=2?'#f00':n<=4?'#0f0':'#00f'"
        };
        let html = format!(
            "<style>html{{background:#f00}}</style><script>let n=0;const raf=window.requestAnimationFrame.bind(window);window.requestAnimationFrame=cb=>raf(t=>{{document.documentElement.style.background={sequence};cb(t)}});</script>"
        );
        let url = format!("data:text/html;base64,{}", STANDARD.encode(html));
        design = json!({"version":"2.21","children":[{"type":"browser","id":"page","width":180,"height":90,"url":url}]});
        if case == "dynamic-idle" {
            fs::write(
                root.join("generated.js"),
                "/** @schema 2.21 */ return [{type:'rectangle',width:180,height:90,fill:'#00FF00'}];",
            )?;
            fs::write(
                root.join("blue.glsl"),
                "precision highp float;\n/** @time */ uniform float clock;\nvoid main(){gl_FragColor=clock<0.5?vec4(0.,0.,1.,1.):vec4(1.,1.,0.,1.);}",
            )?;
            design["children"][0]["x"] = 180.into();
            design["children"].as_array_mut().unwrap().insert(0, json!({"type":"script","id":"script","width":180,"height":90,"scriptUri":"generated.js"}));
            design["children"].as_array_mut().unwrap().push(json!({"type":"rectangle","id":"shader","x":360,"width":180,"height":90,"fill":{"type":"shader","url":"blue.glsl"}}));
        }
    }
    if matches!(
        case,
        "font-config"
            | "user-font-config"
            | "script-config"
            | "user-agent-config"
            | "project-agent-config"
            | "user-key-config"
            | "user-navigation-config"
    ) {
        let path = if case.starts_with("user-") {
            let directory = root.join("user-settings");
            fs::create_dir(&directory)?;
            environment.insert(
                "LAZYPENCIL_CONFIG_DIR".into(),
                directory.clone().into_os_string(),
            );
            directory.join("config.toml")
        } else {
            root.join("lazypencil.toml")
        };
        let text = match case {
            "font-config" | "user-font-config" => {
                design = json!({"version":"2.21","children":[{"type":"text","id":"label","content":"A","fontSize":80,"fill":"#FF0000"}]});
                let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts");
                format!(
                    "version=1\n[fonts]\nfiles={}\ndefault_family='LP Test Sans'\nlocale='en-US'\n",
                    serde_json::to_string(&[fonts.join("faces.ttc"), fonts.join("cff.otf")])?
                )
            }
            "script-config" => {
                fs::write(root.join("generated.js"), "/** @schema 2.21 */ for (;;) {}")?;
                design = json!({"version":"2.21","children":[{"type":"script","id":"script","width":180,"height":90,"scriptUri":"generated.js"}]});
                "[runtime.javascript]\ntimeout_ms=100\nheap_mib=8\nidle_seconds=0\nlayout_timeout_ms=5000\nmax_evaluations=8\nmax_passes=4\n".to_owned()
            }
            "user-agent-config" | "project-agent-config" => format!(
                "[agents]\ndefault='claude'\n[agents.claude]\nadapter={}\nadapter_args=['acp','settings-auth-check']\nmodel='fixture-model-2'\nmode='workspace-write'\nauth_method='fixture-auth'\ntimeout=7\n",
                serde_json::to_string(&crate::fixture_support::helper())?
            ),
            _ => {
                let mut text = "[tui.keys]\nsubmit=['F5']\npaste=[]\n".to_owned();
                if case == "user-navigation-config" {
                    text.push_str("[tui.navigation]\nnext_panel=['F6']\nprevious_panel=['F7']\nhelp=['F8','t']\ncommands=['F9']\nquit=['x']\npan_left=['a']\npan_right=['d']\npan_up=['w']\npan_down=['s']\nzoom_in=['z']\nzoom_out=['v']\n");
                }
                text
            }
        };
        fs::write(&path, text)?;
        configuration = Some(path);
    }
    write_json(&document, &design)?;
    // Checkpoints are created before opening the TUI so its initial revision is current.
    let mut first_checkpoint = Value::Null;
    if matches!(case, "restore" | "restore-conflict") {
        for (color, request) in [
            ("#22C55E", "first-checkpoint"),
            ("#2563EB", "second-checkpoint"),
        ] {
            let result = super::run_cli(
                binary,
                root,
                &environment,
                &["inspect".as_ref(), document.as_os_str()],
            )?;
            let operations = root.join("operations.json");
            write_json(
                &operations,
                &json!([{"op":"update","target":"red","set":{"fill":color}}]),
            )?;
            let value = super::run_cli(
                binary,
                root,
                &environment,
                &[
                    "apply".as_ref(),
                    document.as_os_str(),
                    "--operations".as_ref(),
                    operations.as_os_str(),
                    "--expected-revision".as_ref(),
                    result["revision"].as_str().unwrap().as_ref(),
                    "--request-id".as_ref(),
                    request.as_ref(),
                ],
            )?;
            if request == "first-checkpoint" {
                first_checkpoint = value;
            }
        }
    }
    let mut h = Harness::start(binary, root, &document, environment, case, configuration)?;
    h.wait("initial UI", 15.0, |h| {
        h.contains_bytes(
            if matches!(case, "user-key-config" | "user-navigation-config") {
                b"F5 sends"
            } else {
                b"Ctrl+V"
            },
        )
    })?;
    let mut report = match case {
        "ime-commit" => ime_commit(&mut h)?,
        "user-navigation-config" => navigation(&mut h)?,
        "session-selection" => conversations::selection(&mut h)?,
        "session-deletion" | "session-deletion-small" => conversations::deletion(&mut h)?,
        "saved-history" => conversations::history(&mut h)?,
        "page-playback" => {
            dynamic::page(&mut h)?;
            Value::Null
        }
        "dynamic-idle" => {
            dynamic::idle(&mut h)?;
            Value::Null
        }
        "script-config" => {
            dynamic::script(&mut h)?;
            Value::Null
        }
        "font-config" | "user-font-config" => {
            font(&mut h)?;
            Value::Null
        }
        "basic" => {
            basic(&mut h)?;
            Value::Null
        }
        "user-agent-config" => {
            agent(&mut h, &design)?;
            Value::Null
        }
        "project-agent-config" => {
            project_agent(&mut h, &design)?;
            Value::Null
        }
        "user-key-config" => {
            keys(&mut h)?;
            Value::Null
        }
        "restore" | "restore-conflict" => {
            restore(&mut h, &first_checkpoint)?;
            Value::Null
        }
        "auto-none" => {
            h.paste("最初の入力を保持する")?;
            h.draft("最初の入力を保持する")?;
            no_agent(&h);
            Value::Null
        }
        _ => {
            encoded(&mut h)?;
            Value::Null
        }
    };
    h.finish()?;
    if case == "user-navigation-config" {
        // Inspect the completed stream once, after the existing exit/drain checks.
        // Scanning the growing output on every wait poll can starve PTY reads.
        let mut synchronized = false;
        let mut clears = 0;
        for offset in 0..h.output.len() {
            let bytes = &h.output[offset..];
            if bytes.starts_with(b"\x1b[?2026h") {
                synchronized = true;
            }
            if bytes.starts_with(b"\x1b[?2026l") {
                synchronized = false;
            }
            if bytes.starts_with(b"\x1b[2J") {
                assert!(
                    synchronized,
                    "A fullscreen clear must not expose a blank camera frame"
                );
                clears += 1;
            }
        }
        assert!(
            !synchronized,
            "Normal exit must finish synchronized updates"
        );
        assert!(clears > 0, "The case must exercise image-placement redraws");
        report["synchronized_full_redraws"] = clears.into();
    }
    if !report.is_null() {
        report["exit_code"] = 0.into();
        report["terminal_modes_restored"] = true.into();
        let filename = match case {
            "ime-commit" => "ime-commit-report.json",
            "user-navigation-config" => "navigation-report.json",
            "saved-history" => "saved-history-report.json",
            "session-selection" => "session-selection-report.json",
            _ => "session-deletion-report.json",
        };
        if let Some(selected) = report["selected_on_exit"].as_str() {
            assert!(
                fs::read_to_string(root.join("terminal-errors.log"))?
                    .contains(&format!("Conversation: {selected}"))
            );
        }
        write_json(&root.join(filename), &report)?;
    }
    println!(
        "{}",
        json!({"case":case,"terminal_bytes":h.output.len(),"exit_code":0})
    );
    Ok(())
}

fn ime_commit(h: &mut Harness) -> Result<Value> {
    // Model the documented Kitty pure-text event, not bracketed paste. With
    // report-all-keys but no associated text, an IME commit has key number zero
    // and its actual characters are absent from the byte stream.
    fn flags(bytes: &[u8]) -> Option<u16> {
        bytes
            .windows(3)
            .enumerate()
            .filter_map(|(start, prefix)| {
                if prefix != b"\x1b[>" {
                    return None;
                }
                let rest = &bytes[start + 3..];
                let end = rest.iter().position(|byte| !byte.is_ascii_digit())?;
                if end == 0 || rest[end] != b'u' {
                    return None;
                }
                std::str::from_utf8(&rest[..end]).ok()?.parse().ok()
            })
            .next_back()
    }
    fn commit(flags: u16, text: &str) -> Vec<u8> {
        if flags & 8 == 0 {
            return text.as_bytes().to_vec();
        }
        if flags & 16 != 0 {
            let codepoints = text
                .chars()
                .map(|c| (c as u32).to_string())
                .collect::<Vec<_>>()
                .join(":");
            return format!("\x1b[0;1;{codepoints}u").into_bytes();
        }
        b"\x1b[0;1u".to_vec()
    }
    let original = fs::read(&h.document)?;
    h.wait("negotiated keyboard text mode", 15.0, |h| {
        flags(&h.output).is_some()
    })?;
    let negotiated = flags(&h.output).unwrap();
    let first = "日本語の変換確定";
    for part in commit(negotiated, first).chunks(2) {
        h.key(part)?;
        h.pump()?;
    }
    h.draft(first)?;
    h.wait(
        "IME anchor follows the committed Japanese text",
        15.0,
        |h| {
            h.screen
                .text()
                .lines()
                .position(|line| line.contains(first))
                .is_some_and(|row| h.screen.cursor() == (17, row as u16))
        },
    )?;
    let first_cursor = h.screen.cursor();
    h.key(b"\x1b[D")?;
    h.wait("IME anchor follows editing in wide text", 15.0, |h| {
        h.screen.cursor() == (15, first_cursor.1)
    })?;
    h.key(b"\x1b[C")?;
    h.wait("IME anchor returns to the insertion point", 15.0, |h| {
        h.screen.cursor() == first_cursor
    })?;
    assert_eq!(
        negotiated & 5,
        5,
        "Keep modified-key disambiguation and alternate-key reporting"
    );
    assert_eq!(negotiated & 2, 0, "Do not duplicate legacy key releases");
    assert_eq!(negotiated & 8, 0, "Keep native IME text on the UTF-8 path");
    h.key(b"\r")?;
    let second = "か\u{3099} 👩‍💻";
    h.key(&commit(negotiated, second))?;
    let mut expected = format!("{first}\n{second}");
    h.draft(&expected)?;
    h.wait(
        "IME anchor follows a rendered combining/ZWJ cluster",
        15.0,
        |h| h.screen.cursor() == (6, first_cursor.1 + 1),
    )?;
    let second_cursor = h.screen.cursor();
    let mut resize_cursors = Vec::new();
    for (columns, rows) in [(80, 24), (50, 16), (145, 38)] {
        let previous = h.screen.cursor();
        h.resize(columns, rows)?;
        h.wait("IME anchor follows the resized prompt", 15.0, |h| {
            let text = h.screen.text();
            let lines = text.lines().collect::<Vec<_>>();
            h.screen.cursor().1 != previous.1
                && lines
                    .iter()
                    .position(|line| line.contains(first))
                    .is_some_and(|row| {
                        h.screen.cursor() == (6, row as u16 + 1)
                            && lines.get(row + 1).is_some_and(|line| line.contains(second))
                    })
                && h.session()
                    .is_some_and(|session| session["draft"] == expected)
        })?;
        // Snapshot the same observed frame; another pump can read the clear
        // belonging to a second resize redraw before its text arrives.
        h.save_screen(&format!("ime-commit-{columns}x{rows}-screen.txt"))?;
        resize_cursors.push(json!({"columns":columns,"rows":rows,"cursor":h.screen.cursor()}));
    }
    let row = h.screen.cursor().1;
    let suffix = " 続行";
    h.key(&commit(negotiated, suffix))?;
    expected.push_str(suffix);
    h.draft(&expected)?;
    h.wait("IME commit remains usable after resizing", 15.0, |h| {
        h.screen.cursor() == (11, row)
            && h.screen
                .text()
                .lines()
                .nth(row as usize)
                .is_some_and(|line| line.contains("か\u{3099}") && line.contains("続行"))
    })?;
    h.save_screen("ime-commit-resumed-screen.txt")?;
    no_agent(h);
    assert_eq!(fs::read(&h.document)?, original);
    Ok(
        json!({"source_commit":source_commit(),"keyboard_flags":negotiated,"japanese_cursor":first_cursor,"combining_zwj_cursor":second_cursor,
        "resize_cursors":resize_cursors,"resumed_cursor":h.screen.cursor(),
        "screen_model":"VT100 cell model; CJK rows and native cursor are checked, ZWJ glyph appearance requires real-emulator evidence",
        "draft":expected,"draft_sha256":hash(expected.as_bytes()),"document_unchanged":true,
        "agent_started":false,"scope":"Real PTY plus Kitty keyboard-mode/IME-commit protocol model; physical IME confirmation is separate"}),
    )
}

fn encoded(h: &mut Harness) -> Result {
    if h.image_case == "kitty" {
        h.wait("complete Kitty image", 15.0, |h| {
            !h.frames.images.is_empty()
        })?;
        assert!(h.frames.images[0].has_color([255, 0, 0, 255]));
    } else {
        h.wait("complete Sixel image", 15.0, |h| sixel(&h.output).is_some())?;
        let (red, total) = observer::sixel_red_pixels(sixel(&h.output).unwrap())?;
        assert!(
            red > total / 4,
            "Decoded Sixel must contain actual red design pixels"
        );
    }
    no_agent(h);
    Ok(())
}
fn sixel(bytes: &[u8]) -> Option<&[u8]> {
    let mut rest = bytes;
    let mut last = None;
    while let Some(start) = rest.windows(2).position(|b| b == b"\x1bP") {
        rest = &rest[start + 2..];
        let header = rest
            .iter()
            .position(|b| !b.is_ascii_digit() && *b != b';')?;
        if rest[header] != b'q' {
            continue;
        }
        let body = &rest[header + 1..];
        let end = body.windows(2).position(|b| b == b"\x1b\\")?;
        last = Some(&body[..end]);
        rest = &body[end + 2..];
    }
    last
}
fn font(h: &mut Harness) -> Result {
    h.wait("configured font preview", 15.0, |h| {
        !h.frames.images.is_empty()
            && h.screen
                .text()
                .lines()
                .any(|s| s.starts_with("1 nodes · 0 diagnostics ·"))
    })?;
    let saved = fs::read(&h.document)?;
    let before = h.frames.images.last().unwrap().clone();
    h.paste("設定を変えても残る下書き")?;
    h.draft("設定を変えても残る下書き")?;
    let frames = h.frames.images.len();
    let path = config(h).to_owned();
    fs::write(
        &path,
        fs::read_to_string(&path)?.replace(
            "default_family='LP Test Sans'",
            "default_family='LP Test CFF'",
        ),
    )?;
    h.wait("automatic configuration/font reload", 15.0, |h| {
        h.frames.images.len() > frames && h.frames.images.last() != Some(&before)
    })?;
    assert!(h.last_color([255, 0, 0, 255]));
    assert_eq!(state(h)["draft"], "設定を変えても残る下書き");
    no_agent(h);
    assert_eq!(fs::read(&h.document)?, saved);
    Ok(())
}
fn keys(h: &mut Harness) -> Result {
    let text = "日本語の複数行\nq と hjkl は本文";
    h.paste(text)?;
    h.draft(text)?;
    h.key(b"\x1b\r")?;
    h.paste(" 続けて入力")?;
    h.wait("input after replaced shortcut", 15.0, |h| {
        state(h)["draft"]
            .as_str()
            .is_some_and(|s| s.ends_with(" 続けて入力"))
    })?;
    assert!(
        h.turns().is_empty(),
        "Replaced key must not launch an agent"
    );
    let expected = state(h)["draft"].as_str().unwrap().to_owned();
    h.key(b"\x1b[15~")?;
    h.wait("F5 submitted actual turn", 15.0, |h| {
        h.count("complete") == 1
    })?;
    assert!(
        read_json(&h.root.join("received-turn-1.json")).unwrap()["text"]
            .as_array()
            .unwrap()
            .contains(&json!(expected))
    );
    assert_eq!(state(h)["draft"], "");
    fs::write(config(h), "[tui.keys]\nsubmit=['F6']\npaste=[]\n")?;
    h.paste("同じキーで次の依頼")?;
    h.key(b"\x1b[15~")?;
    h.wait("startup shortcut retained", 15.0, |h| {
        h.count("complete") == 2
    })?;
    assert_eq!(h.count("ready"), 1);
    Ok(())
}
fn agent(h: &mut Harness, design: &Value) -> Result {
    assert_eq!(state(h)["agent"], "claude");
    no_agent(h);
    h.paste("保存したエージェント設定で送信")?;
    h.key(b"\x1b\r")?;
    h.wait("configured agent completed turn", 15.0, |h| {
        h.count("complete") == 1
    })?;
    let ready = h
        .turns()
        .into_iter()
        .find(|v| v["type"] == "ready")
        .unwrap();
    assert_eq!(
        ready["response"]["modes"]["currentModeId"],
        "workspace-write"
    );
    assert_eq!(
        ready["response"]["configOptions"][0]["currentValue"],
        "fixture-model-2"
    );
    fs::write(
        config(h),
        "[agents]\ndefault='codex'\n[agents.codex]\nadapter='missing/never-launch-this'\n",
    )?;
    h.paste("同じ接続で次の依頼")?;
    h.key(b"\x13")?;
    h.wait("second turn kept resolved settings", 15.0, |h| {
        h.count("complete") == 2
    })?;
    let starts: Vec<_> = h
        .turns()
        .into_iter()
        .filter(|v| v["type"] == "start")
        .collect();
    assert_eq!(starts.len(), 2);
    assert!(starts.iter().all(|v| v["agent"] == "claude"));
    assert_eq!(starts[0]["session"], starts[1]["session"]);
    assert_eq!(h.count("ready"), 1);
    assert_eq!(&read_json(&h.document).unwrap(), design);
    Ok(())
}
fn project_agent(h: &mut Harness, design: &Value) -> Result {
    fn closed(h: &mut Harness, label: &str) -> Result {
        // Keep draining Kitty output between process observations. Synchronous
        // ps on every read can fill the PTY and block the application's redraw.
        let mut next = Instant::now();
        let mut gone = false;
        h.wait(label, 15.0, |h| {
            if Instant::now() >= next {
                gone = dynamic::children(h).is_empty();
                next = Instant::now() + Duration::from_millis(200);
            }
            gone
        })
    }
    assert_eq!(state(h)["agent"], "claude");
    h.wait("untrusted project still renders", 15.0, |h| {
        !h.frames.images.is_empty()
    })?;
    no_agent(h);
    h.cli(&["config".as_ref(), "trust".as_ref(), h.document.as_os_str()])?;
    let draft = "実行許可まで保持する下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\x13")?;
    h.wait(
        "design grant does not authorize project executables",
        15.0,
        |h| h.screen.text().contains("PROJECT_NOT_TRUSTED"),
    )?;
    assert_eq!(state(h)["draft"], draft);
    no_agent(h);
    h.cli(&[
        "config".as_ref(),
        "trust".as_ref(),
        h.document.as_os_str(),
        "--executables".as_ref(),
    ])?;
    h.key(b"\x13")?;
    h.wait("project grant permits the pending turn", 15.0, |h| {
        h.count("complete") == 1
    })?;
    assert!(!dynamic::children(h).is_empty());
    h.cli(&[
        "config".as_ref(),
        "untrust".as_ref(),
        h.document.as_os_str(),
    ])?;
    let draft = "許可を取り消しても保持する下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\x13")?;
    closed(h, "revoked project grant closes the existing adapter")?;
    assert_eq!(state(h)["draft"], draft);
    assert_eq!(h.count("start"), 1);
    h.cli(&[
        "config".as_ref(),
        "trust".as_ref(),
        h.document.as_os_str(),
        "--executables".as_ref(),
    ])?;
    h.key(b"\x13")?;
    h.wait("regrant resumes the same conversation", 15.0, |h| {
        h.count("complete") == 2
    })?;
    assert_eq!(h.count("ready"), 2);
    let configuration =
        fs::read_to_string(config(h))?.replace("model='fixture-model-2'", "model='fixture-model'");
    fs::write(config(h), configuration)?;
    h.cli(&[
        "config".as_ref(),
        "trust".as_ref(),
        h.document.as_os_str(),
        "--executables".as_ref(),
    ])?;
    let draft = "変更前の設定では送信しない";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\x13")?;
    closed(h, "changed settings do not reuse the earlier selection")?;
    assert_eq!(state(h)["draft"], draft);
    assert_eq!(h.count("start"), 2);
    assert_eq!(&read_json(&h.document).unwrap(), design);
    Ok(())
}

fn basic(h: &mut Harness) -> Result {
    let text = "日本語の入力\nhjkl + - q は本文です";
    h.paste(text)?;
    h.draft(text)?;
    no_agent(h);
    h.key(b"\x1b\r")?;
    h.wait("first completed turn", 15.0, |h| h.count("complete") == 1)?;
    h.wait("visible turn completion", 15.0, |h| {
        h.screen.contains_words("Request", "complete")
    })?;
    h.paste("次の依頼")?;
    h.key(b"\x13")?;
    h.wait("second completed turn", 15.0, |h| h.count("complete") == 2)?;
    assert!(
        read_json(&h.root.join("received-turn-1.json")).unwrap()["text"]
            .as_array()
            .unwrap()
            .contains(&json!(text))
    );
    assert_eq!(state(h)["draft"], "");
    assert_eq!(state(h)["native_session_id"], "fixture-native");
    let turns = h.turns();
    let starts: Vec<_> = turns.iter().filter(|v| v["type"] == "start").collect();
    let usage: Vec<_> = turns
        .iter()
        .filter(|v| v["type"] == "design_resource")
        .collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(usage.len(), 6);
    assert!(
        usage
            .iter()
            .all(|v| v["connection"] == usage[0]["connection"])
    );
    for (number, guide_id) in [(1, "typography"), (2, "color")] {
        let resources = read_json(&h.root.join(format!("received-turn-{number}.json"))).unwrap()["design_resources"].clone();
        let guides: Vec<_> = usage
            .iter()
            .filter(|v| v["tool"] == "get_guidance" && v["resources"][0]["id"] == guide_id)
            .collect();
        assert_eq!(guides.len(), 1);
        let turn = &guides[0]["turn"];
        let turn_usage: Vec<_> = usage.iter().filter(|v| &v["turn"] == turn).collect();
        assert_eq!(turn_usage.len(), 3);
        assert_eq!(guides[0]["resources"][0]["hash"], resources["guide_hash"]);
        assert_eq!(
            turn_usage
                .iter()
                .find(|v| v["tool"] == "get_style")
                .unwrap()["resources"][0]["hash"],
            resources["style_hash"]
        );
        assert_eq!(starts.iter().filter(|v| &v["turn"] == turn).count(), 1);
    }
    h.key(b"\tll+0")?;
    for repetition in 0..8 {
        h.resize(145 + repetition % 2, 38)?;
        h.key(b"\x1bOP")?;
        h.visible("/clear-images")?;
        h.key(b"\x1bOP")?;
        h.hidden("/clear-images")?;
    }
    Ok(())
}
fn restore(h: &mut Harness, checkpoint: &Value) -> Result {
    h.paste("復元しても保持する下書き")?;
    h.draft("復元しても保持する下書き")?;
    command(h, "Restore checkpoint", 0)?;
    h.wait("checkpoint candidates", 15.0, |h| {
        h.screen.text().contains("first-checkpoint")
            && h.screen.text().contains("second-checkpoint")
    })?;
    h.key(b"\x1b[B\r")?;
    h.visible("Restore selected checkpoint?")?;
    assert_eq!(
        read_json(&h.document).unwrap()["children"][0]["fill"],
        "#2563EB"
    );
    h.key(b"\x1b")?;
    h.wait("cancelled review", 15.0, |h| {
        h.screen.text().contains("Checkpoints")
            && !h.screen.text().contains("Restore selected checkpoint?")
    })?;
    h.key(b"\r")?;
    h.visible("Restore selected checkpoint?")?;
    if h.case == "restore-conflict" {
        let external = edit(h, "#D946EF", "external-edit")?;
        h.key(b"\r")?;
        h.visible("REVISION_CONFLICT")?;
        assert_eq!(
            read_json(&h.document).unwrap()["children"][0]["fill"],
            "#D946EF"
        );
        assert_eq!(
            h.cli(&["history".as_ref(), h.document.as_os_str()])?["entries"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        h.key(b"r")?;
        let revision = &external["revision"].as_str().unwrap()[..12];
        h.wait("explicit revision refresh", 15.0, |h| {
            h.screen.text().contains(revision) && h.screen.text().contains("Checkpoints")
        })?;
        h.key(b"\x1b[B\x1b[B\r")?;
        h.wait("fresh checkpoint review", 15.0, |h| {
            h.screen.text().contains("Restore selected checkpoint?")
                && h.screen.text().contains("Checkpoint #1")
        })?;
    }
    h.key(b"\r")?;
    h.wait("restored document", 15.0, |h| {
        read_json(&h.document).is_some_and(|d| d["children"][0]["fill"] == "#22C55E")
    })?;
    h.wait("visible restoration", 15.0, |h| {
        h.screen.contains_words("Checkpoint", "restored")
    })?;
    assert_eq!(state(h)["draft"], "復元しても保持する下書き");
    let entries = h.cli(&["history".as_ref(), h.document.as_os_str()])?["entries"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        entries.len(),
        if h.case == "restore-conflict" { 4 } else { 3 }
    );
    assert_eq!(
        entries[0]["result"]["restored_checkpoint"],
        checkpoint["checkpoint_id"]
    );
    no_agent(h);
    Ok(())
}
fn navigation(h: &mut Harness) -> Result<Value> {
    h.wait("initial complete navigation preview", 15.0, |h| {
        !h.frames.images.is_empty()
    })?;
    let before_index = h.frames.images.len() - 1;
    let before = h.frames.images[before_index].clone();
    assert!(before.has_color([255, 0, 0, 255]));
    let text = "日本語の下書き\nx a d w s z v t は本文";
    h.paste(text)?;
    h.draft(text)?;
    h.key(b"\x1b[17~h\x1b[19~")?;
    h.visible("/clear-images")?;
    assert!(h.screen.text().contains("a: Preview: pan left"));
    assert_eq!(h.frames.images.last(), Some(&before));
    h.key(b"\x1b[F")?;
    h.wait("help final wrapped lines", 15.0, |h| {
        h.screen
            .text()
            .contains("Clipboard access happens only on explicit paste.")
            && h.screen
                .text()
                .contains("protocols do not provide clipboard access.")
    })?;
    h.key(b"\x1b[H")?;
    h.visible("/clear-images")?;
    h.key(b"\x1b")?;
    h.hidden("/clear-images")?;
    let cameras = h.frames.camera_frames;
    h.key(b"a")?;
    h.wait("remapped pan actual pixels", 15.0, |h| {
        h.frames.images.last() != Some(&before)
    })?;
    assert!(
        h.frames.camera_frames > cameras,
        "Pan must display separate cached feedback before the exact frame"
    );
    let panned_index = h.frames.images.len() - 1;
    let panned = h.frames.images[panned_index].clone();
    let cameras = h.frames.camera_frames;
    h.key(b"z")?;
    h.wait("remapped zoom actual pixels", 15.0, |h| {
        h.frames.images.last() != Some(&panned)
    })?;
    assert!(
        h.frames.camera_frames > cameras,
        "Zoom must display separate cached feedback before the exact frame"
    );
    let zoomed_index = h.frames.images.len() - 1;
    let zoomed = h.frames.images[zoomed_index].clone();
    assert!(panned.has_color([255, 0, 0, 255]) && zoomed.has_color([255, 0, 0, 255]));
    h.key(b"\x1b[20~")?;
    h.wait("configured command palette", 15.0, |h| {
        h.screen.text().contains("Commands") && h.screen.text().contains("Restore checkpoint")
    })?;
    h.key(b"x")?;
    h.wait("quit key remains search text", 15.0, |h| {
        h.screen.text().contains("Commands") && h.screen.text().contains('x')
    })?;
    h.key(b"\x1b")?;
    h.hidden("Restore checkpoint")?;
    h.key(b"\x1b[18~")?;
    h.paste(" 続き")?;
    h.draft(&format!("{text} 続き"))?;
    fs::write(config(h), "[tui.navigation]\nnext_panel=['F10']\n")?;
    h.key(b"\x1b[17~\x1b[19~")?;
    h.visible("/clear-images")?;
    h.key(b"\x1b")?;
    h.hidden("/clear-images")?;
    assert!(h.turns().is_empty());
    no_agent(h);
    assert_eq!(state(h)["draft"], format!("{text} 続き"));
    let mut frames = serde_json::Map::new();
    for (name, index, frame) in [
        ("before", before_index, before),
        ("panned", panned_index, panned),
        ("zoomed", zoomed_index, zoomed),
    ] {
        frames.insert(
            name.into(),
            json!({"index":index,"sha256":hash(frame.rgba()),"rgba_bytes":frame.rgba().len()}),
        );
    }
    Ok(
        json!({"source_commit":source_commit(),"case":h.case,"frames":frames,"draft_sha256":hash(format!("{text} 続き")),"help_end_reached":true,"startup_keys_retained":true,"turns":h.turns().len()}),
    )
}
