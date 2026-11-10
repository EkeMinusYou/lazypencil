use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use sha2::{Digest, Sha256};

mod conversations;
mod dynamic;
mod mouse;

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
    text.split('\n')
        .rev()
        .nth(1)
        .unwrap_or_default()
        .trim_start_matches("! ")
        .to_owned()
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
fn palette_selection(h: &Harness) -> Option<String> {
    h.screen
        .text()
        .lines()
        .find(|line| line.contains("▏›"))
        .map(str::to_owned)
}
fn command(h: &mut Harness, label: &str) -> Result {
    h.key(b"\x1bp")?;
    h.visible("Commands")?;
    h.wait("command palette selection", 15.0, |h| {
        palette_selection(h).is_some()
    })?;
    for _ in 0..64 {
        let current = palette_selection(h).ok_or("Missing command palette selection")?;
        if current.contains(label) {
            return h.key(b"\r");
        }
        if current.contains("Quit") {
            break;
        }
        h.key(b"\x1b[B")?;
        h.wait("next command palette selection", 15.0, |h| {
            palette_selection(h).is_some_and(|line| line != current)
        })?;
    }
    Err(format!("Command is not available in the palette: {label}").into())
}
fn command_input(h: &mut Harness, label: &str, input: &str) -> Result {
    command(h, label)?;
    h.visible("[Enter Run]")?;
    h.paste(input)?;
    h.key(b"\r")?;
    h.hidden("[Enter Run]")
}

fn edit(h: &Harness, color: &str, request: &str) -> Result<Value> {
    let inspected = h.cli(&["inspect".as_ref(), h.document.as_os_str()])?;
    let revision = inspected["revision"].as_str().unwrap();
    let dependency_revision = inspected["dependency_revision"].as_str().unwrap();
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
        "--expected-dependency-revision".as_ref(),
        dependency_revision.as_ref(),
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
    environment.remove(std::ffi::OsStr::new("HERDR_ENV"));
    environment.remove(std::ffi::OsStr::new("HC_TERMINAL_ID"));
    environment.extend(extra_environment.iter().cloned());
    if matches!(case, "stderr-terminal" | "stderr-file" | "picker-stderr") {
        environment.insert(
            "LAZYPENCIL_PROFILE_INPUT".into(),
            "owned-local-measurement".into(),
        );
    }
    if case.starts_with("background-") || case.starts_with("theme-") {
        // These cases inspect emitted RGB; NO_COLOR has separate coverage.
        environment.insert("NO_COLOR".into(), "".into());
    }
    if matches!(case, "kitty-no-color" | "kitty-placements-no-color") {
        environment.insert("NO_COLOR".into(), "1".into());
    }
    if case == "picker-loading" {
        environment.insert(
            "LAZYPENCIL_LOCK_DIR".into(),
            root.join("locks").into_os_string(),
        );
    }
    let mut document = root.join("design.pen");
    if case == "session-deletion-small" {
        let parent = root
            .join("long-project-path-".repeat(5))
            .join("long-design-directory-".repeat(5));
        fs::create_dir_all(&parent)?;
        document = parent.join("design.pen");
    }
    let mut design = json!({"version":"2.21","children":[{"type":"rectangle","id":"red","name":"日本語の対象","x":-80,"y":-30,"width":180,"height":90,"fill":"#FF0000"}]});
    if case.starts_with("preview-search") {
        design = json!({"version":"2.21","children":[
            {"type":"text","id":"first","content":"日本語の対象","width":200,"height":40},
            {"type":"text","id":"second","x":1000,"content":"日本語の対象","width":200,"height":40}
        ]});
    }
    if case == "kitty-progressive" {
        design = json!({"version":"2.21","children":[
            {"type":"rectangle","id":"red","name":"Red source","x":-80,"y":-30,"width":180,"height":90,"fill":"#FF0000"},
            {"type":"rectangle","id":"blue","name":"New destination","x":240,"y":-30,"width":180,"height":90,"fill":"#0000FF"}
        ]});
    }
    if case == "mouse-kitty" || case.starts_with("selection-") {
        environment.insert(
            "LAZYPENCIL_PROFILE_INPUT".into(),
            "owned-local-measurement".into(),
        );
    }
    if case.starts_with("selection-") {
        design = json!({"version":"2.21","children":[
            {"type":"frame","id":"left","name":"Left canvas shape","x":-180,"y":-40,"width":100,"height":80,"fill":"#ff0000","layout":"none",
             "children":[{"type":"rectangle","id":"child","name":"Canvas child","x":10,"y":10,"width":20,"height":20,"fill":"#ffff00"}]},
            {"type":"rectangle","id":"right","name":"Right canvas shape","x":60,"y":-40,"width":100,"height":80,"fill":"#00ff00"},
            {"type":"rectangle","id":"top","name":"Top canvas shape","x":80,"y":-20,"width":40,"height":40,"fill":"#0000ff"}
        ]});
    }
    let mut configuration = None;
    if case == "preview-search-light" {
        let directory = root.join("user-settings");
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("config.toml"), "[tui]\ntheme='light'\n")?;
        environment.insert(
            "LAZYPENCIL_CONFIG_DIR".into(),
            directory.clone().into_os_string(),
        );
        configuration = Some(directory);
    }
    if case.starts_with("theme-") {
        let directory = root.join("user-settings");
        fs::create_dir_all(&directory)?;
        let theme = match case {
            "theme-light-override" => "dark",
            "theme-dark-override" => "light",
            "theme-monochrome" => "monochrome",
            _ => "auto",
        };
        fs::write(
            directory.join("config.toml"),
            format!("[tui]\ntheme='{theme}'\n"),
        )?;
        environment.insert(
            "LAZYPENCIL_CONFIG_DIR".into(),
            directory.clone().into_os_string(),
        );
        configuration = Some(directory);
    }
    if case.starts_with("mouse-") {
        design = json!({"version":"2.21","children":(0..64).map(|index| json!({"type":"rectangle","id":format!("mouse-{index}"),"name":format!("Mouse row {index}"),"x":index*100,"width":80,"height":80,"fill":"#ff0000"})).collect::<Vec<_>>()});
    }
    if case.starts_with("mouse-") || case == "picker-mouse" {
        let directory = root.join("user-settings");
        fs::create_dir_all(&directory)?;
        fs::write(
            directory.join("config.toml"),
            if case == "mouse-disabled" {
                "[tui.mouse]\nenabled=false\n"
            } else {
                "[tui.mouse]\nshow_controls=true\n"
            },
        )?;
        environment.insert(
            "LAZYPENCIL_CONFIG_DIR".into(),
            directory.clone().into_os_string(),
        );
        configuration = Some(directory);
    }
    if case == "node-references" {
        design = json!({"version":"2.21","children":[
            {"type":"rectangle","id":"first","name":"First target","width":80,"height":40,"fill":"#00FF00"},
            {"type":"rectangle","id":"second","name":"Second target","x":100,"width":80,"height":40,"fill":"#0000FF"}
        ]});
    }
    if case == "diagnostics" {
        design = json!({"version":"2.21","children":[{
            "type":"frame","id":"root","name":"Root","width":100,"height":100,"layout":"none",
            "children":[
                {"type":"rectangle","id":"child-a","name":"First target","x":95,"width":20,"height":10,"fill":"#00FF00"},
                {"type":"rectangle","id":"child-b","name":"Second target","x":95,"y":20,"width":20,"height":10,"fill":"#0000FF"}
            ]
        }]});
    }
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
            | "node-references"
            | "ime-commit"
    ) {
        let path = if case.starts_with("user-") || matches!(case, "node-references" | "ime-commit")
        {
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
            // Keep exact IME anchors independent of the application's default pane ratios.
            "ime-commit" => "[tui.layout]\nnodes=22\npreview=48\nagent=30\n".to_owned(),
            "node-references" => "[tui.navigation]\nprevious_section=['F6','Alt+Up']\nnext_section=['F7','Alt+Down']\n".to_owned(),
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
                    text.push_str("[tui.layout]\nnodes=1\npreview=2\nagent=1\n");
                    text.push_str("[tui.navigation]\nnext_panel=['F6']\nprevious_panel=['F7']\nhelp=['F8','t']\ncommands=['F9']\nquit=['x']\npan_left=['a']\npan_right=['d']\npan_up=['w']\npan_down=['s']\nzoom_in=['z']\nzoom_out=['v']\n");
                }
                text
            }
        };
        fs::write(&path, text)?;
        configuration = Some(path);
    }
    if case != "startup-new" {
        write_json(&document, &design)?;
    }
    if case.starts_with("picker-") {
        fs::create_dir_all(root.join("nested/deep"))?;
        write_json(&root.join("nested/deep/selected.pen"), &design)?;
        write_json(&root.join("nested/deep/設定.pen"), &design)?;
        fs::write(root.join("broken.pen"), "not a document")?;
        if case == "picker-default" {
            let directory = root.join("user-settings");
            fs::create_dir_all(&directory)?;
            fs::write(
                directory.join("config.toml"),
                format!(
                    "[agents.codex]\nadapter={}\nadapter_args=['acp','auth-inspect-account']\n",
                    serde_json::to_string(&crate::fixture_support::helper().to_string_lossy())?
                ),
            )?;
            environment.insert("LAZYPENCIL_CONFIG_DIR".into(), directory.into_os_string());
        }
    }
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
                    "--expected-dependency-revision".as_ref(),
                    result["dependency_revision"].as_str().unwrap().as_ref(),
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
    let mut picker_open_offset = None;
    if case.starts_with("picker-") {
        h.visible("Open a .pen file")?;
        h.visible("nested/deep/selected.pen")?;
        for label in ["Nodes", "Preview", "Agent", "Agent prepared"] {
            h.visible(label)?;
        }
        assert!(!h.screen.text().contains("Loading preview"));
        for (width, height) in [(80, 24), (50, 16), (130, 34)] {
            h.resize(width, height)?;
            h.wait("resized picker footer", 10.0, |h| {
                let text = h.screen.text();
                text.contains("Ctrl+C") && text.contains("[Enter Open]")
            })?;
            h.visible("Open a .pen file")?;
        }
        assert!(!root.join("received-account-methods.jsonl").exists());
        assert!(files(root, |name| name == "session.json").is_empty());
        assert!(!root.join("sent-turn.txt").exists());
        if case == "picker-mouse" {
            picker_open_offset = Some(h.output.len());
            mouse::picker(&mut h)?;
        } else if case == "picker-cancel" {
            h.finish()?;
            return Ok(());
        }
        if matches!(
            case,
            "picker-default" | "picker-prepared" | "picker-loading" | "picker-stderr"
        ) {
            h.key(b"\x0c")?;
            h.key(b"design.pen")?;
            let blocked = if case == "picker-loading" {
                let directory = root.join("locks");
                fs::create_dir_all(&directory)?;
                let file = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(directory.join(format!(
                        "{}.lock",
                        hash(b"lazypencil-managed-assets-activity-v1")
                    )))?;
                fs2::FileExt::lock_exclusive(&file)?;
                Some(file)
            } else {
                None
            };
            picker_open_offset = Some(h.output.len());
            h.key(b"\r")?;
            if blocked.is_some() {
                h.wait("loading feedback before blocked file work", 2.0, |h| {
                    let text = h.screen.text();
                    text.contains("Loading nodes…")
                        && text.contains("Loading preview…")
                        && text.contains("design.pen")
                        && !text.contains("Type to filter files")
                        && !text.contains("Opening…")
                        && !text.contains("Open a .pen file")
                })?;
                assert!(files(root, |name| name == "session.json").is_empty());
                assert!(!root.join("sent-turn.txt").exists());
            }
            drop(blocked);
        } else if case == "picker-direct" {
            h.key(b"\x0c")?;
            h.key(b"nested/deep/selected.pen")?;
            picker_open_offset = Some(h.output.len());
            h.key(b"\r")?;
        } else if case != "picker-mouse" {
            h.key(b"/broken")?;
            h.key(b"\r")?;
            h.visible("Open a .pen file")?;
            h.visible("INVALID_DOCUMENT")?;
            h.key(b"\x1b")?;
            h.key(b"\x0c")?;
            h.key(b"nested")?;
            h.key(b"\r")?;
            h.visible("deep/selected.pen")?;
            h.key(if case == "picker-unicode" {
                "/設定".as_bytes()
            } else {
                b"/selected"
            })?;
            picker_open_offset = Some(h.output.len());
            h.key(b"\r")?;
        }
    }
    if case.starts_with("picker-") {
        h.wait("selected document workspace", 15.0, |h| {
            let text = h.screen.text();
            !text.contains("Open a .pen file")
                && text.lines().next().unwrap_or_default().contains(".pen")
        })?;
        assert_eq!(
            h.output
                .windows(b"\x1b[?1049h".len())
                .filter(|bytes| *bytes == b"\x1b[?1049h")
                .count(),
            1,
            "Entered the alternate screen again after selection"
        );
        assert!(
            !h.contains_bytes(b"\x1b[?1049l"),
            "Left the alternate screen during selection"
        );
    }
    h.wait("initial UI", 15.0, |h| {
        h.screen.find_text("›").is_some() && h.screen.text().contains("Agent")
    })?;
    if case != "startup-new" {
        h.visible(" pan")?;
        // The remaining interactions in these cases begin in the prompt.
        h.key(if case == "user-navigation-config" {
            b"\x1b[17~"
        } else {
            b"\t"
        })?;
    }
    let mut report = match case {
        "stderr-terminal" | "stderr-file" | "picker-stderr" => {
            // Real application diagnostics exercise descriptor-level capture,
            // rather than bypassing it by writing to the PTY from the observer.
            h.key(b"\x1b[<65;115;10M")?;
            h.paste("diagnostic-safe")?;
            h.draft("diagnostic-safe")?;
            assert!(
                h.screen
                    .text()
                    .lines()
                    .next()
                    .unwrap()
                    .contains("design.pen")
            );
            assert!(!h.contains_bytes(b"LAZYPENCIL_INPUT_PROFILE"));
            if case == "stderr-file" {
                h.wait("live redirected diagnostics", 5.0, |_| {
                    fs::read_to_string(root.join("terminal-errors.log"))
                        .unwrap_or_default()
                        .contains("LAZYPENCIL_INPUT_PROFILE")
                })?;
            }
            no_agent(&h);
            Value::Null
        }
        "startup-existing" | "startup-new" => {
            h.visible(" send")?;
            h.key("起動後の入力".as_bytes())?;
            h.draft("起動後の入力")?;
            no_agent(&h);
            assert_eq!(
                read_json(&h.document).unwrap(),
                if case == "startup-new" {
                    json!({"version":"2.22","children":[]})
                } else {
                    design.clone()
                }
            );
            Value::Null
        }
        case if case.starts_with("theme-") => {
            ui_theme(&mut h)?;
            Value::Null
        }
        "background-kitty"
        | "background-direct-kitty"
        | "background-sixel"
        | "background-halfblocks"
        | "background-fallback" => {
            terminal_background(&mut h)?;
            Value::Null
        }
        "selection-kitty" | "selection-sixel" | "selection-halfblocks" => {
            mouse::canvas_selection(&mut h)?;
            Value::Null
        }
        "mouse-kitty" | "mouse-sixel" | "mouse-halfblocks" | "mouse-disabled" => {
            mouse::workspace(&mut h)?;
            Value::Null
        }
        "picker-mouse" => Value::Null,
        "picker-default" | "picker-prepared" => {
            h.wait("prepared adapter session", 10.0, |_| {
                fs::read_to_string(root.join("received-account-methods.jsonl"))
                    .unwrap_or_default()
                    .contains("session/new")
            })?;
            let methods = fs::read_to_string(root.join("received-account-methods.jsonl"))?;
            assert_eq!(methods.lines().count(), 2, "{methods}");
            assert!(methods.contains("initialize") && methods.contains("session/new"));
            assert!(!root.join("sent-turn.txt").exists());
            Value::Null
        }
        "picker-loading" => {
            h.visible("design.pen")?;
            assert!(!root.join("sent-turn.txt").exists());
            Value::Null
        }
        "picker-explicit" | "picker-unicode" | "picker-direct" => {
            h.visible(if case == "picker-unicode" {
                "設定.pen"
            } else {
                "selected.pen"
            })?;
            assert!(!root.join("sent-turn.txt").exists());
            Value::Null
        }
        "agent-header-fast" | "agent-header-standard" => {
            let fast = case == "agent-header-fast";
            let correct_header = |h: &Harness| {
                let text = h.screen.text();
                let header = text.lines().next().unwrap_or_default();
                header.contains(" · fixture-model medium")
                    && !header.contains("model:")
                    && !header.contains("effort:")
                    && !header.contains("account:")
                    && !header.contains("session:")
                    && header.contains(" fast") == fast
            };
            h.wait(
                "configured model and speed in the top row",
                15.0,
                correct_header,
            )?;
            no_agent(&h);
            h.paste("Check the model and speed")?;
            h.key(b"\x13")?;
            h.wait("header turn completed", 15.0, |h| h.completed_turns(1))?;
            h.wait(
                "negotiated model and speed in the top row",
                15.0,
                correct_header,
            )?;
            let sessions = fs::read_to_string(root.join("received-settings-sessions.jsonl"))?;
            assert_eq!(
                sessions.lines().count(),
                1,
                "The startup connection must be reused"
            );
            h.resize(80, 24)?;
            h.wait(
                "model and speed remain visible after resizing",
                15.0,
                correct_header,
            )?;
            Value::Null
        }
        "agent-header-hang" => {
            h.visible(" · connecting · ")?;
            h.paste("Keep this draft while connecting")?;
            h.draft("Keep this draft while connecting")?;
            no_agent(&h);
            Value::Null
        }
        "account-startup" => {
            h.wait("startup session prepared without a prompt", 15.0, |h| {
                fs::read_to_string(h.root.join("received-account-methods.jsonl"))
                    .unwrap_or_default()
                    .contains("session/new")
            })?;
            h.visible("must-not-log@example.test")?;
            no_agent(&h);
            let methods = fs::read_to_string(root.join("received-account-methods.jsonl"))?;
            assert_eq!(
                methods
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    .collect::<Vec<_>>(),
                [
                    json!({"method":"initialize"}),
                    json!({"method":"session/new"})
                ]
            );
            fs::write(root.join("account-changed"), "changed")?;
            h.paste("Check the selected account")?;
            h.key(b"\x13")?;
            h.visible("other-account@example.test")?;
            h.wait("account turn complete", 15.0, |h| h.completed_turns(1))?;
            for saved in std::iter::once(state(&h)).chain(h.turns()) {
                let saved = saved.to_string();
                for private in [
                    "must-not-log@example.test",
                    "other-account@example.test",
                    "fixture-not-a-token",
                ] {
                    assert!(
                        !saved.contains(private),
                        "Display identity must stay out of saved events"
                    );
                }
            }
            Value::Null
        }
        "ime-commit" => ime_commit(&mut h)?,
        case if case.starts_with("preview-search") => {
            preview_search(&mut h)?;
            Value::Null
        }
        "node-references" => {
            node_references(&mut h, &design)?;
            Value::Null
        }
        "diagnostics" => {
            diagnostics(&mut h, &design)?;
            Value::Null
        }
        "user-navigation-config" => navigation(&mut h)?,
        "permission-modes" => {
            permission_modes(&mut h)?;
            Value::Null
        }
        "command-palette" => {
            command_palette(&mut h)?;
            Value::Null
        }
        "collapsible-panels" => {
            collapsible_panels(&mut h)?;
            Value::Null
        }
        "oversized-preview" => {
            oversized_preview(&mut h, &design)?;
            Value::Null
        }
        "slash-completion" => {
            conversations::completion(&mut h)?;
            Value::Null
        }
        "history-bottom" => {
            conversations::history_bottom(&mut h)?;
            Value::Null
        }
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
        "kitty-progressive" => {
            progressive_navigation(&mut h)?;
            Value::Null
        }
        "kitty-held" => {
            held_navigation(&mut h)?;
            Value::Null
        }
        "kitty-placements" | "kitty-placements-no-color" => {
            rectangular(&mut h)?;
            Value::Null
        }
        "queued-followups" => {
            queued_followups(&mut h)?;
            Value::Null
        }
        "steering" => {
            conversations::steering(&mut h)?;
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
    if matches!(case, "stderr-terminal" | "picker-stderr") {
        let leave = h
            .output
            .windows(b"\x1b[?1049l".len())
            .position(|bytes| bytes == b"\x1b[?1049l")
            .expect("Left the alternate screen");
        let diagnostic = h
            .output
            .windows(b"LAZYPENCIL_INPUT_PROFILE".len())
            .position(|bytes| bytes == b"LAZYPENCIL_INPUT_PROFILE")
            .expect("Replayed captured diagnostics");
        assert!(diagnostic > leave);
        assert!(h.contains_bytes(b"Conversation:"));
    }
    if case.starts_with("picker-") {
        let mut synchronized = false;
        let mut positions = 0;
        for offset in 0..h.output.len() {
            let bytes = &h.output[offset..];
            if bytes.starts_with(b"\x1b[?2026h") {
                synchronized = true;
            } else if bytes.starts_with(b"\x1b[?2026l") {
                synchronized = false;
            }
            if offset >= picker_open_offset.expect("Selected a document") {
                assert!(
                    !bytes.starts_with(b"\x1b[2J"),
                    "Cleared the reused workspace while opening"
                );
            }
            if let Some(csi) = bytes.strip_prefix(b"\x1b[") {
                let command = csi
                    .iter()
                    .copied()
                    .find(|byte| (0x40..=0x7e).contains(byte));
                if matches!(command, Some(b'H' | b'f')) {
                    assert!(synchronized, "Painted a partial picker/workspace update");
                    positions += 1;
                }
            }
        }
        assert!(positions > 0 && !synchronized);
    }
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
        assert_eq!(
            clears, 0,
            "Camera and modal updates must not erase the screen"
        );
        assert!(h.frames.transmitted_resources.len() >= 2);
        report["fullscreen_clears"] = clears.into();
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

fn preview_search(h: &mut Harness) -> Result {
    let original = fs::read(&h.document)?;
    let draft = "未送信の依頼";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\x1b[Z")?;
    h.visible("/ search")?;
    let start = h.output.len();
    h.key(b"/")?;
    h.visible("Enter keep")?;
    h.wait("search uses UTF-8 keyboard mode", 15.0, |h| {
        h.output[start..]
            .windows(b"\x1b[=5u".len())
            .any(|bytes| bytes == b"\x1b[=5u")
    })?;
    h.key("日本語".as_bytes())?;
    h.visible("/日本語")?;
    assert!(
        h.screen
            .find_text("Preview /日本語")
            .is_some_and(|(_, y)| y == 1)
    );
    h.visible("1/2")?;
    h.wait("IME cursor follows the Japanese query", 15.0, |h| {
        h.screen
            .find_text("/日本語")
            .is_some_and(|(x, y)| h.screen.cursor() == (x + 7, y))
    })?;
    h.key(b"\x08")?;
    h.hidden("/日本語")?;
    h.visible("/日本")?;
    h.key("語".as_bytes())?;
    h.visible("/日本語")?;
    h.key(b"\x01\x06")?;
    h.wait(
        "search cursor follows Emacs character movement",
        15.0,
        |h| {
            h.screen
                .find_text("/日本語")
                .is_some_and(|(x, y)| h.screen.cursor() == (x + 3, y))
        },
    )?;
    h.key(b"\x04")?;
    h.visible("/日語")?;
    h.visible("No matches")?;
    h.key(b"\x1b[95;5u")?;
    h.visible("/日本語")?;
    h.visible("1/2")?;
    h.key(b"\x1b[B")?;
    h.key(b"\r")?;
    h.visible("Esc clear search")?;
    h.visible("1/2")?;
    h.key(b"n")?;
    h.visible("2/2")?;
    h.key(b"N")?;
    h.visible("1/2")?;
    h.key(b"\x1b")?;
    h.hidden("/日本語")?;
    h.key(b"/")?;
    h.visible("Enter keep")?;
    h.paste("日本語の対象")?;
    h.visible("/日本語の対象")?;
    h.visible("1/2")?;
    h.key(b" q?rpn")?;
    h.visible("No matches")?;
    h.visible("/日本語の対象 q?rpn")?;
    h.key(b"\x1b")?;
    h.hidden("Enter keep")?;
    h.key(b"\x1b[Z/")?;
    h.visible("Nodes /")?;
    h.visible("Enter keep filter")?;
    h.paste("first")?;
    h.visible("Nodes /first")?;
    h.key(b"\x1b[104;5u")?;
    h.hidden("Nodes /first")?;
    h.visible("Nodes /firs")?;
    h.key(b"\x01\x0b")?;
    h.hidden("Nodes /firs")?;
    h.key(b"\x19t")?;
    h.visible("Nodes /first")?;
    h.key(b"\r")?;
    h.hidden("Enter keep filter")?;
    h.key(b"/")?;
    h.visible("Enter keep filter")?;
    h.key(b"\x1b")?;
    h.hidden("Nodes /first")?;
    h.key(b"\t")?;
    h.visible(" pan")?;
    h.key(b"\t")?;
    h.visible(" send")?;
    h.paste(" 続き")?;
    h.draft(&format!("{draft} 続き"))?;
    no_agent(h);
    assert_eq!(fs::read(&h.document)?, original);
    Ok(())
}

fn command_palette(h: &mut Harness) -> Result {
    let draft = "未送信の依頼はそのまま";
    let document = fs::read(&h.document)?;
    h.paste(draft)?;
    h.draft(draft)?;
    command(h, "Seek shader time")?;
    h.visible("Seek shader time")?;
    h.paste("NaN")?;
    h.key(b"\r")?;
    h.visible("INVALID_ARGUMENT")?;
    assert_eq!(state(h)["draft"], draft);
    h.key(b"\x7f\x7f\x7f1.25\r")?;
    h.hidden("[Enter Run]")?;
    assert_eq!(state(h)["draft"], draft);

    for (width, height) in [(130, 34), (50, 16)] {
        h.resize(width, height)?;
        command(h, "Attach image file")?;
        h.visible("[Enter Run]")?;
        h.paste("画像.png")?;
        h.wait("Unicode command input cursor", 15.0, |h| {
            h.screen
                .find_text("画像.png")
                .is_some_and(|(x, y)| h.screen.cursor() == (x + 8, y))
        })?;
        assert_eq!(state(h)["draft"], draft);
        h.save_screen(if width == 50 {
            "command-palette-small-screen.txt"
        } else {
            "command-palette-input-screen.txt"
        })?;
        h.key(b"\x1b")?;
        h.visible("Commands")?;
        h.hidden("[Enter Run]")?;
        h.key(b"\x1b[F")?;
        h.visible("GENERAL")?;
        h.visible("Quit")?;
        h.key(b"\x1b")?;
        h.hidden("Commands")?;
    }
    assert_eq!(fs::read(&h.document)?, document);
    assert_eq!(state(h)["draft"], draft);
    no_agent(h);
    Ok(())
}

fn node_references(h: &mut Harness, design: &Value) -> Result {
    h.visible("2 nodes")?;
    let draft = "この要素について相談したいです";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\tja")?;
    h.visible("References")?;
    h.draft(draft)?;
    no_agent(h);
    h.key(b"\x1b[17~")?;
    h.visible("Delete remove")?;
    h.save_screen("references-focused-screen.txt")?;
    h.key(b"\x1b[18~")?;
    h.draft(draft)?;
    h.key(b"\x1b[17~")?;
    h.key(b"\x1b[3~")?;
    h.hidden("References")?;
    h.draft(draft)?;
    h.key(b"\ta")?;
    h.visible("References")?;
    h.save_screen("references-prompt-screen.txt")?;
    h.key(b"\tj\t\t")?;
    h.paste("。余白を調整できますか？")?;
    h.key(b"\x13")?;
    h.wait("reference request completes", 15.0, |h| {
        h.completed_turns(1)
    })?;
    let received = read_json(&h.root.join("received-turn-1.json")).unwrap();
    let reference = received["text"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|text| {
            let text = text.as_str()?;
            let offset = text.find("{\"node_references\"")?;
            serde_json::from_str::<Value>(&text[offset..]).ok()
        })
        .expect("Explicit reference context reaches the adapter");
    assert_eq!(
        reference["node_references"][0]["target"]["address"]["node_id"],
        "first"
    );
    assert_eq!(reference["node_references"][0]["name"], "First target");
    h.hidden("References ·")?;
    h.paste("Continue the conversation")?;
    h.key(b"\x13")?;
    h.wait("next request completes", 15.0, |h| h.completed_turns(2))?;
    let next = read_json(&h.root.join("received-turn-2.json")).unwrap();
    assert!(
        next["text"]
            .as_array()
            .unwrap()
            .iter()
            .all(|text| !text.as_str().unwrap().contains("node_references"))
    );
    assert_eq!(&read_json(&h.document).unwrap(), design);
    Ok(())
}

fn diagnostics(h: &mut Harness, design: &Value) -> Result {
    let draft = "Keep the unsent draft";
    h.paste(draft)?;
    h.draft(draft)?;
    h.visible("2 diagnostics")?;
    h.key(b"\tjd")?;
    h.visible("Diagnostics")?;
    h.visible("Subtree: Root")?;
    h.visible("First target")?;
    h.key(b"j")?;
    h.visible("Second target")?;
    h.key(b"a")?;
    h.visible("References")?;
    h.draft(draft)?;
    no_agent(h);
    h.key(b"\t")?;
    h.key(b"\r")?;
    h.wait("diagnostic reveals its hidden target", 15.0, |h| {
        let text = h.screen.text();
        text.contains("First target") && text.contains("Second target") && !text.contains("Details")
    })?;
    assert_eq!(&read_json(&h.document).unwrap(), design);
    h.key(b"d")?;
    h.visible("Details")?;
    h.resize(50, 16)?;
    h.key(b"\x04")?;
    h.visible("Details")?;
    h.resize(130, 34)?;
    let mut repaired = design.clone();
    for child in repaired["children"][0]["children"].as_array_mut().unwrap() {
        child["x"] = 0.into();
    }
    write_json(&h.document, &repaired)?;
    h.visible("No diagnostics for node")?;
    h.key(b"d")?;
    h.hidden("No diagnostics for node")?;
    h.key(b"\t\t")?;
    h.draft(draft)?;
    assert_eq!(state(h)["draft"], draft);
    no_agent(h);
    h.key(b"\x13")?;
    h.wait("diagnostic reference reaches the agent", 15.0, |h| {
        h.completed_turns(1)
    })?;
    let received = read_json(&h.root.join("received-turn-1.json")).unwrap();
    let reference = received["text"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|text| {
            let text = text.as_str()?;
            let offset = text.find("{\"node_references\"")?;
            serde_json::from_str::<Value>(&text[offset..]).ok()
        })
        .expect("Diagnostic reference context");
    assert_eq!(
        reference["node_references"][0]["diagnostic"]["code"],
        "LAYOUT_OVERFLOW"
    );
    assert_eq!(
        reference["node_references"][0]["target"]["address"]["node_id"],
        "child-b"
    );
    h.paste("文書全体のレイアウト上の問題を確認して修正してください")?;
    h.key(b"\x13")?;
    h.wait(
        "document-wide diagnostic request reaches the agent",
        15.0,
        |h| h.completed_turns(2),
    )?;
    let all = read_json(&h.root.join("received-turn-2.json")).unwrap();
    assert!(
        all["text"].as_array().unwrap().iter().any(|text| text
            .as_str()
            .is_some_and(|text| text.contains("validate_document")
                && text.contains("next_cursor")
                && text.contains("snapshot_id")))
    );
    assert!(
        all["text"]
            .as_array()
            .unwrap()
            .iter()
            .all(|text| !text.as_str().unwrap().contains("\"node_references\""))
    );
    Ok(())
}

fn ime_commit(h: &mut Harness) -> Result<Value> {
    fn prompt_origin(h: &Harness) -> Option<(u16, u16)> {
        h.screen.find_text("›").map(|(x, y)| (x + 2, y))
    }
    // Model the documented Kitty pure-text event, not bracketed paste. With
    // report-all-keys but no associated text, an IME commit has key number zero
    // and its actual characters are absent from the byte stream.
    fn flags(bytes: &[u8]) -> Option<u16> {
        bytes
            .windows(3)
            .enumerate()
            .filter_map(|(start, prefix)| {
                if prefix != b"\x1b[>" && prefix != b"\x1b[=" {
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
    h.wait("negotiated prompt keyboard text mode", 15.0, |h| {
        // The focus-changing Tab is asynchronous. Model an IME commit only
        // after the terminal has received the prompt's UTF-8 keyboard mode.
        flags(&h.output).is_some_and(|flags| flags & 8 == 0)
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
                .is_some_and(|row| {
                    prompt_origin(h).is_some_and(|(x, _)| h.screen.cursor() == (x + 16, row as u16))
                })
        },
    )?;
    let first_cursor = h.screen.cursor();
    h.key(b"\x1b[D")?;
    h.wait("IME anchor follows editing in wide text", 15.0, |h| {
        h.screen.cursor() == (first_cursor.0 - 2, first_cursor.1)
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
    h.key(b"\x1b[13;2u")?;
    let second = "か\u{3099} 👩‍💻";
    h.key(&commit(negotiated, second))?;
    let mut expected = format!("{first}\n{second}");
    h.draft(&expected)?;
    no_agent(h);
    h.wait(
        "IME anchor follows a rendered combining/ZWJ cluster",
        15.0,
        |h| prompt_origin(h).is_some_and(|(x, top)| h.screen.cursor() == (x + 5, top + 1)),
    )?;
    let cluster = "👩‍💻".as_bytes();
    assert!(
        h.output
            .windows(cluster.len())
            .any(|bytes| bytes == cluster)
    );
    let second_cursor = h.screen.cursor();
    let mut resize_cursors = Vec::new();
    for (columns, rows) in [(80, 24), (50, 16), (145, 38)] {
        let previous = h.screen.cursor();
        let repaint_start = h.output.len();
        h.resize(columns, rows)?;
        h.wait("IME anchor follows the resized prompt", 15.0, |h| {
            let text = h.screen.text();
            let lines = text.lines().collect::<Vec<_>>();
            let Some((second_x, second_row)) = h.screen.find_text("か\u{3099}") else {
                return false;
            };
            // Resizing the observer can move its old cursor before the redraw
            // arrives. Require the new layout before accepting the IME anchor.
            // The first logical line can soft-wrap on a narrow Prompt: locate
            // the rendered second line instead of assuming it is one row down.
            prompt_origin(h).is_some_and(|(x, top)| {
                second_x == x
                    && second_row > top
                    && h.screen.cursor() == (x + 5, second_row)
                    && lines
                        .get(top as usize)
                        .is_some_and(|line| line.contains("日本語"))
                    && lines.get(second_row as usize).is_some_and(|line| {
                        // vt100 counts ZWJ components separately, so padded
                        // cells can overwrite one component in its model.
                        // Exact draft text and emitted emoji bytes are
                        // checked separately from this visible prefix.
                        line.contains("か\u{3099}") && line.contains('👩')
                    })
            }) && lines.get(rows as usize - 1).is_some_and(|line| {
                // Require the complete current Prompt actions after layout.
                line.starts_with("Ctrl+Enter send") && line.contains("F1 help")
            }) && h.screen.cursor().1 != previous.1
                && h.session()
                    .is_some_and(|session| session["draft"] == expected)
        })?;
        assert!(
            h.output[repaint_start..]
                .windows(cluster.len())
                .any(|bytes| bytes == cluster),
            "Emit the complete ZWJ cluster after resizing to {columns}x{rows}"
        );
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
        prompt_origin(h).is_some_and(|(x, _)| h.screen.cursor() == (x + 10, row))
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

fn ui_theme(h: &mut Harness) -> Result {
    encoded(h)?;
    let settings = fs::read(config(h).join("config.toml"))?;
    let original = fs::read(&h.document)?;
    let background = match h.case.as_str() {
        "theme-auto-dark" | "theme-dark-override" => "24;24;37",
        "theme-monochrome" => "245;245;245",
        "theme-unknown" => "24;24;37",
        _ => "242;244;247",
    };
    h.wait("theme input background", 15.0, |h| {
        h.contains_bytes(format!("48;2;{background}").as_bytes())
    })?;
    let canvas = match h.case.as_str() {
        "theme-auto-dark" | "theme-dark-override" => [30, 30, 46, 255],
        "theme-unknown" => [30, 30, 46, 255],
        "theme-monochrome" => [250, 250, 250, 255],
        _ => [250, 250, 252, 255],
    };
    assert_eq!(&h.frames.images[0].rgba()[..4], &canvas);
    command_input(h, "Theme", "light")?;
    h.visible("UI theme: Light")?;
    theme_canvas(h, [250, 250, 252, 255])?;
    command_input(h, "Theme", "dark")?;
    h.visible("UI theme: Dark")?;
    theme_canvas(h, [30, 30, 46, 255])?;
    // The shared command palette opens theme arguments without sending a prompt.
    command(h, "Theme")?;
    h.visible("[Enter Run]")?;
    h.paste("light")?;
    h.key(b"\r")?;
    h.visible("UI theme: Light")?;
    theme_canvas(h, [250, 250, 252, 255])?;
    assert_eq!(fs::read(config(h).join("config.toml"))?, settings);
    assert_eq!(fs::read(&h.document)?, original);
    no_agent(h);
    Ok(())
}

fn theme_canvas(h: &mut Harness, background: [u8; 4]) -> Result {
    h.wait(
        "preview follows the workspace theme background",
        15.0,
        |h| {
            h.frames.images.last().is_some_and(|image| {
                image.rgba()[..4] == background && image.has_color([255, 0, 0, 255])
            })
        },
    )
}

fn terminal_background(h: &mut Harness) -> Result {
    match h.image_case.as_str() {
        "kitty" => {
            encoded(h)?;
            assert!(
                h.contains_bytes(b"]11;?"),
                "Kitty also queries the terminal background"
            );
            let frame = &h.frames.images[0];
            assert!(frame.has_color([30, 30, 46, 255]));
            assert!(
                frame
                    .rgba()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[3] == 255),
                "Canvas must not depend on terminal image alpha compositing"
            );
            assert!(frame.has_color([255, 0, 0, 255]));
        }
        "sixel" => {
            encoded(h)?;
            let data = String::from_utf8_lossy(sixel(&h.output).unwrap());
            assert!(
                data.split(";2;").skip(1).any(|part| {
                    let components = part
                        .split(|c: char| !c.is_ascii_digit())
                        .take(3)
                        .filter_map(|value| value.parse::<u8>().ok())
                        .collect::<Vec<_>>();
                    components.len() == 3
                        && components
                            .iter()
                            .zip([12, 12, 18])
                            .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
                }),
                "Sixel palette must include the workspace theme background"
            );
        }
        _ => {
            let rgb = "30;30;46";
            h.wait("halfblocks canvas background", 15.0, |h| {
                h.contains_bytes(format!("48;2;{rgb}").as_bytes())
            })?;
            if h.case == "background-fallback" {
                h.paste("背景確認")?;
            }
        }
    }
    h.draft("背景確認")?;
    no_agent(h);
    Ok(())
}

fn encoded(h: &mut Harness) -> Result {
    if h.image_case == "kitty" {
        h.wait("complete Kitty image", 15.0, |h| {
            !h.frames.images.is_empty()
        })?;
        assert!(h.frames.images[0].has_color([255, 0, 0, 255]));
        h.wait("regular Kitty placement", 15.0, |h| {
            h.contains_bytes(b"_Ga=p,")
        })?;
        let id = *h.frames.transmitted_resources.iter().next().unwrap();
        assert!(
            h.contains_bytes(format!("a=p,i={id},").as_bytes()),
            "Kitty placement must reference the transmitted image ID even without color"
        );
        assert!(!h.contains_bytes("\u{10eeee}".as_bytes()));
    } else {
        h.wait("complete Sixel image", 15.0, |h| sixel(&h.output).is_some())?;
        let (red, total) = observer::sixel_red_pixels(sixel(&h.output).unwrap())?;
        assert!(
            red > total / 4,
            "Decoded Sixel must contain actual red design pixels"
        );
    }
    if matches!(h.case.as_str(), "auto-kitty" | "auto-kitty-raw") {
        assert_eq!(h.frames.compressed_frames > 0, h.case == "auto-kitty");
    }
    if matches!(
        h.case.as_str(),
        "auto-kitty" | "auto-kitty-raw" | "auto-sixel"
    ) {
        h.paste("自動判定後の日本語")?;
        h.draft("自動判定後の日本語")?;
    }
    no_agent(h);
    Ok(())
}
fn keyboard_flags(bytes: &[u8]) -> Option<u16> {
    bytes
        .windows(3)
        .enumerate()
        .filter_map(|(start, prefix)| {
            if prefix != b"\x1b[>" && prefix != b"\x1b[=" {
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

fn held_navigation(h: &mut Harness) -> Result {
    let saved = fs::read(&h.document)?;
    h.wait("initial held-key preview", 15.0, |h| {
        !h.frames.images.is_empty()
    })?;
    let initial = h.frames.images.last().unwrap().clone();
    assert_eq!(keyboard_flags(&h.output), Some(5));
    h.key(b"\t\t")?;
    h.wait(
        "Preview requests direct press, repeat and release reports",
        15.0,
        |h| keyboard_flags(&h.output) == Some(15),
    )?;
    let pan_inputs = |h: &Harness| {
        fs::read_to_string(h.root.join("terminal-errors.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                serde_json::from_str::<Value>(line.strip_prefix("LAZYPENCIL_INPUT_PROFILE ")?).ok()
            })
            .filter(|event| event["event"] == "input")
            .count()
    };
    let before = pan_inputs(h);
    // Multiplexers can still deliver legacy presses while mode changes are in
    // flight. Every press must survive, including two presses of the same key.
    h.key(b"hh\x1b[104;1:3ull\x1b[108;1:3ujj\x1b[106;1:3ukk\x1b[107;1:3u")?;
    h.wait("all eight batched presses are handled", 3.0, |h| {
        pan_inputs(h) >= before + 8
    })?;
    h.pump_for(0.2)?;
    assert_eq!(pan_inputs(h), before + 8);
    h.wait("balanced batched input restores the viewport", 15.0, |h| {
        h.frames.images.last() == Some(&initial)
    })?;
    let before = pan_inputs(h);
    let frames = h.frames.images.len();
    for (index, kind) in [1, 2, 2].into_iter().enumerate() {
        let previous = h.frames.images.len();
        h.key(format!("\x1b[104;1:{kind}u").as_bytes())?;
        h.wait(
            "rapid repeats publish complete frames before release",
            3.0,
            |h| pan_inputs(h) == before + index + 1 && h.frames.images.len() > previous,
        )?;
        h.pump_for(0.015)?;
    }
    h.key(b"\x1b[104;1:3u")?;
    h.wait("release presents the completed detailed frame", 15.0, |h| {
        h.frames.images.len() > frames
    })?;
    h.key(b"0")?;
    h.wait("fit restores the viewport after rapid repeats", 15.0, |h| {
        h.frames.images.last() == Some(&initial)
    })?;
    for letter in *b"hjkl" {
        let count = pan_inputs(h);
        for (index, kind) in [1, 2, 2].into_iter().enumerate() {
            let placements = h.frames.camera_placement_updates;
            let frames = h.frames.images.len();
            h.key(format!("\x1b[{};1:{}u", letter, kind).as_bytes())?;
            h.wait(
                "press and repeat each update the preview before release",
                3.0,
                |h| {
                    pan_inputs(h) == count + index + 1
                        && (h.frames.camera_placement_updates > placements
                            || h.frames.images.len() > frames)
                },
            )?;
            // Delayed text repetition must not invent additional movement.
            h.pump_for(0.12)?;
            assert_eq!(pan_inputs(h), count + index + 1);
        }
        h.key(format!("\x1b[{};1:3u", letter).as_bytes())?;
        h.pump_for(0.3)?;
        let stopped = h.frames.camera_placement_updates;
        let exact = h.frames.images.last().cloned();
        h.pump_for(0.2)?;
        assert_eq!(h.frames.camera_placement_updates, stopped);
        assert_eq!(h.frames.images.last(), exact.as_ref());
        h.key(b"?")?;
        h.visible("Help")?;
        h.wait("Help restores text-safe keyboard mode", 15.0, |h| {
            keyboard_flags(&h.output) == Some(5)
        })?;
        h.key(b"\x1b")?;
        h.hidden("Help")?;
        h.wait("closing Help restores direct navigation keys", 15.0, |h| {
            keyboard_flags(&h.output) == Some(15)
        })?;
        h.key(b"0")?;
        h.wait("fit restores the original viewport", 15.0, |h| {
            h.frames.images.last() == Some(&initial)
        })?;
    }
    h.key(b"\x1b[104;1:1u")?;
    h.wait("movement starts before focus loss", 3.0, |h| {
        h.frames.images.last() != Some(&initial)
    })?;
    h.key(b"\x1b[O")?;
    h.pump_for(0.3)?;
    let stopped = h.frames.camera_placement_updates;
    h.pump_for(0.2)?;
    assert_eq!(h.frames.camera_placement_updates, stopped);
    h.key(b"\x1b[I")?;
    h.key(b"\x1b[104;1:1u")?;
    h.key(b"\x1b[9;1:1u")?;
    h.wait("Prompt restores UTF-8 IME keyboard mode", 15.0, |h| {
        keyboard_flags(&h.output) == Some(5)
    })?;
    assert_eq!(
        h.frames.camera_placement_updates, 0,
        "Navigation must never project a cell-rounded cached viewport"
    );
    assert!(!h.contains_bytes(b"Cached preview"));
    h.paste("hjklは本文です")?;
    h.draft("hjklは本文です")?;
    assert_eq!(fs::read(&h.document)?, saved);
    no_agent(h);
    Ok(())
}

fn progressive_navigation(h: &mut Harness) -> Result {
    let saved = fs::read(&h.document)?;
    h.wait("initial complete scene", 15.0, |h| {
        !h.frames.images.is_empty()
    })?;
    let initial = h.frames.images.last().unwrap().clone();
    let frames = h.frames.images.len();
    h.key(b"\tj\r")?;
    h.wait(
        "selected red source excludes the blue destination",
        15.0,
        |h| {
            h.frames.images.len() > frames
                && h.frames.images.last().is_some_and(|image| {
                    image.has_color([255, 0, 0, 255]) && !image.has_color([0, 0, 255, 255])
                })
        },
    )?;
    h.key(b"\t")?;
    let frames = h.frames.images.len();
    for index in 0..20 {
        h.key(format!("\x1b[108;1:{}u", if index == 0 { 1 } else { 2 }).as_bytes())?;
        h.pump_for(0.025)?;
    }
    assert!(
        h.frames.images.len() >= frames + 2,
        "Held navigation must publish multiple fresh images before release"
    );
    assert!(
        h.frames.images[frames..]
            .iter()
            .any(|image| image.has_color([0, 0, 255, 255])),
        "Previously uncached blue destination must load before key release"
    );
    h.key(b"\x1b[108;1:3u")?;
    h.pump_for(0.3)?;
    // Fitting the document preserves selection. Clear its decoration before
    // comparing the restored viewport with the original unselected pixels.
    h.key(b"\x1b")?;
    h.pump_for(0.15)?;
    h.key(b"0")?;
    h.wait("fit restores the complete scene", 15.0, |h| {
        h.frames.images.last() == Some(&initial)
    })?;
    assert_eq!(fs::read(&h.document)?, saved);
    no_agent(h);
    Ok(())
}

fn rectangular(h: &mut Harness) -> Result {
    h.wait("rectangular Kitty preview", 15.0, |h| {
        !h.frames.images.is_empty() && h.contains_bytes(b"_Ga=p,")
    })?;
    assert!(h.last_color([255, 0, 0, 255]));
    let draft = "改行を含む\n下書きを保持";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\t\t")?;
    for key in [b"+".as_slice(), b"l", b"-"] {
        let before = h.frames.images.last().unwrap().clone();
        h.key(key)?;
        h.wait("camera changes exact rectangular pixels", 15.0, |h| {
            h.frames.images.last().is_some_and(|image| image != &before)
        })?;
    }
    let before_help = h.output.len();
    h.key(b"?")?;
    h.visible("Help")?;
    assert!(
        h.output[before_help..]
            .windows(10)
            .any(|bytes| bytes == b"a=d,d=i,i=")
    );
    let before_close = h.output.len();
    h.key(b"\x1b")?;
    h.hidden("Help")?;
    h.wait("image placement returns after the modal", 15.0, |h| {
        h.output[before_close..]
            .windows(5)
            .any(|bytes| bytes == b"a=p,i")
    })?;
    assert!(!h.contains_bytes(b"\x1b[2J"));
    for (width, height) in [(80, 24), (50, 16), (130, 34)] {
        let count = h.frames.images.len();
        h.resize(width, height)?;
        h.wait("rectangular image after resize", 15.0, |h| {
            h.frames.images.len() > count
        })?;
    }
    h.key(b"\t")?;
    h.paste("、続きを入力")?;
    h.draft(&format!("{draft}、続きを入力"))?;
    h.wait(
        "IME cursor remains at the end of the rectangular preview draft",
        15.0,
        |h| {
            h.screen
                .find_text("、続きを入力")
                .is_some_and(|(x, y)| h.screen.cursor() == (x + 12, y))
        },
    )?;
    assert!(!h.contains_bytes("\u{10eeee}".as_bytes()));
    assert!(!h.contains_bytes(b"U=1"));
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
                .any(|s| s.contains("1 node · 0 diagnostics"))
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
    h.wait("F5 submitted actual turn", 15.0, |h| h.completed_turns(1))?;
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
    h.wait("startup shortcut retained", 15.0, |h| h.completed_turns(2))?;
    assert_eq!(h.count("ready"), 1);
    Ok(())
}
fn agent(h: &mut Harness, design: &Value) -> Result {
    assert_eq!(state(h)["agent"], "claude");
    no_agent(h);
    h.paste("保存したエージェント設定で送信")?;
    h.key(b"\r")?;
    h.wait("configured agent completed turn", 15.0, |h| {
        h.completed_turns(1)
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
        h.completed_turns(2)
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
    h.draft(draft)?;
    no_agent(h);
    h.cli(&[
        "config".as_ref(),
        "trust".as_ref(),
        h.document.as_os_str(),
        "--executables".as_ref(),
    ])?;
    h.key(b"\x13")?;
    h.wait("project grant permits the pending turn", 15.0, |h| {
        h.completed_turns(1)
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
        h.completed_turns(2)
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

fn queued_followups(h: &mut Harness) -> Result {
    h.paste("First request")?;
    h.key(b"\r")?;
    h.wait("first request started", 15.0, |h| {
        h.root.join("started-turn-1").exists()
    })?;
    h.wait("loading indicator before the first response", 15.0, |h| {
        h.screen.text().contains("Working…")
    })?;
    assert_eq!(h.count("complete"), 0);
    h.wait(
        "loading indicator advances while the agent is silent",
        5.0,
        |h| {
            let text = h.screen.text();
            text.contains("Working…") && !text.contains("Working… 0s")
        },
    )?;
    for (index, text) in ["Second request", "Third request"].into_iter().enumerate() {
        h.paste(text)?;
        if index == 0 {
            h.key(b"\x1b[13;2u")?;
            h.paste("continued")?;
            h.wait("Shift+Enter keeps the active follow-up unsent", 15.0, |h| {
                files(&h.root.join("data"), |name| name == "prompt-outbox.json")
                    .iter()
                    .any(|path| {
                        read_json(path).is_some_and(|state| {
                            state["draft"] == "Second request\ncontinued"
                                && state["queued"].as_array().is_some_and(Vec::is_empty)
                        })
                    })
            })?;
            assert!(!h.screen.text().contains("1 queued"));
            assert_eq!(h.count("complete"), 0);
        }
        h.key(b"\r")?;
        h.wait("follow-up queued while working", 15.0, |h| {
            h.screen.text().contains(&format!("{} queued", index + 1))
        })?;
    }
    h.wait("all queued follow-ups are visible in order", 15.0, |h| {
        let text = h.screen.text();
        text.find("1. Second request continue")
            .zip(text.find("2. Third request"))
            .is_some_and(|(first, second)| first < second)
    })?;
    h.paste("Unsent next draft")?;
    h.wait("next draft is saved separately", 15.0, |h| {
        files(&h.root.join("data"), |name| name == "prompt-outbox.json")
            .iter()
            .any(|path| {
                read_json(path).is_some_and(|state| {
                    state["draft"] == "Unsent next draft"
                        && state["queued"]
                            .as_array()
                            .is_some_and(|queue| queue.len() == 2)
                })
            })
    })?;
    h.key(b"\x1b[1;3A")?;
    for turn in 1..=3 {
        fs::write(h.root.join(format!("release-turn-{turn}")), "release")?;
        if turn < 3 {
            h.wait("next queued request starts automatically", 15.0, |h| {
                h.root.join(format!("started-turn-{}", turn + 1)).exists()
            })?;
        }
    }
    h.wait("queue drained and editor restored", 15.0, |h| {
        h.completed_turns(3)
    })?;
    h.draft("Unsent next draft")?;
    h.paste(" appended")?;
    h.draft("Unsent next draft appended")?;
    h.key(b"\r")?;
    h.wait(
        "submission works after completing queued turns",
        15.0,
        |h| h.root.join("started-turn-4").exists(),
    )?;
    fs::write(h.root.join("release-turn-4"), "release")?;
    h.wait("fourth request completed", 15.0, |h| {
        h.completed_turns(4) && !h.screen.text().contains("Working…")
    })?;
    let prompts = fs::read_to_string(h.root.join("received-prompts.jsonl"))?;
    let prompts = prompts
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (prompt, expected) in prompts.iter().zip([
        "First request",
        "Second request\ncontinued",
        "Third request",
        "Unsent next draft appended",
    ]) {
        assert!(
            prompt["prompt"]
                .as_array()
                .unwrap()
                .iter()
                .any(|block| block["text"] == expected)
        );
    }
    assert_eq!(prompts.len(), 4);
    Ok(())
}

fn basic(h: &mut Harness) -> Result {
    let text = "日本語の入力\nhjkl + - q は本文です";
    h.paste(text)?;
    h.draft(text)?;
    no_agent(h);
    h.key(b"\r")?;
    h.wait("first completed turn", 15.0, |h| h.completed_turns(1))?;
    h.wait("visible turn completion", 15.0, |h| {
        h.screen.contains_words("Request", "complete")
    })?;
    h.paste("次の依頼")?;
    h.key(b"\x13")?;
    h.wait("second completed turn", 15.0, |h| h.completed_turns(2))?;
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
    h.key(b"\t\tll+0")?;
    for repetition in 0..8 {
        h.resize(145 + repetition % 2, 38)?;
        h.key(b"\x1bOP")?;
        h.visible("Help")?;
        h.key(b"\x1bOP")?;
        h.hidden("Help")?;
    }
    Ok(())
}
fn restore(h: &mut Harness, checkpoint: &Value) -> Result {
    h.paste("復元しても保持する下書き")?;
    h.draft("復元しても保持する下書き")?;
    command(h, "Browse and restore checkpoints")?;
    h.wait("checkpoint candidates", 15.0, |h| {
        h.screen.text().contains("first-checkpoint")
            && h.screen.text().contains("second-checkpoint")
    })?;
    h.key(b"\x1b[B\r")?;
    h.visible("[Enter Restore]")?;
    assert_eq!(
        read_json(&h.document).unwrap()["children"][0]["fill"],
        "#2563EB"
    );
    h.key(b"\x1b")?;
    h.wait("cancelled review", 15.0, |h| {
        h.screen.text().contains("Checkpoints") && !h.screen.text().contains("[Enter Restore]")
    })?;
    h.key(b"\r")?;
    h.visible("[Enter Restore]")?;
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
            h.screen.text().contains("[Enter Restore]")
                && h.screen.text().contains("Restore checkpoint #1?")
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
fn oversized_preview(h: &mut Harness, design: &Value) -> Result {
    h.wait("initial high-DPI preview", 15.0, |h| {
        h.frames.images.last().is_some_and(|frame| {
            (6500..=6900).contains(&frame.width) && frame.has_color([255, 0, 0, 255])
        })
    })?;
    let draft = "大きな画面でも保持する下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    for key in [b"\x1bn", b"\x1ba"] {
        h.frames.images.clear();
        let frames = h.frames.images.len();
        h.key(key)?;
        h.wait("bounded full-width preview", 15.0, |h| {
            h.frames.images.len() > frames
                && h.frames
                    .images
                    .last()
                    .is_some_and(|frame| frame.width <= 8192 && frame.has_color([255, 0, 0, 255]))
        })?;
        assert!(!h.screen.text().contains("Preview unavailable"));
    }
    let before = h.frames.images.last().unwrap().clone();
    h.frames.images.clear();
    h.key(b"l")?;
    h.wait("large viewport pan changes pixels", 15.0, |h| {
        h.frames.images.last().is_some_and(|frame| frame != &before)
    })?;
    h.key(b"h")?;
    h.wait("large viewport reverse pan restores pixels", 15.0, |h| {
        h.frames.images.last() == Some(&before)
    })?;
    h.frames.images.clear();
    h.resize(80, 30)?;
    h.wait("smaller viewport regains native resolution", 15.0, |h| {
        h.frames
            .images
            .last()
            .is_some_and(|frame| frame.width == 7800 && frame.has_color([255, 0, 0, 255]))
    })?;
    let retained = h.frames.images.last().unwrap().clone();
    let mut broken = design.clone();
    broken["version"] = "2.9".into();
    write_json(&h.document, &broken)?;
    h.visible("Showing previous preview")?;
    h.visible("UNSUPPORTED_VERSION")?;
    assert_eq!(h.frames.images.last(), Some(&retained));
    h.frames.images.clear();
    write_json(&h.document, design)?;
    h.wait("repaired preview replaces the stale state", 15.0, |h| {
        !h.screen.text().contains("Showing previous preview")
            && h.frames.images.last() == Some(&retained)
    })?;
    h.key(b"\x1ba")?;
    h.visible("Prompt")?;
    h.draft(draft)?;
    assert!(
        !h.screen.text().contains("UNSUPPORTED_VERSION"),
        "Preview failure leaked into Agent history"
    );
    no_agent(h);
    Ok(())
}

fn collapsible_panels(h: &mut Harness) -> Result {
    h.wait("initial complete panel preview", 15.0, |h| {
        h.frames
            .images
            .last()
            .is_some_and(|image| image.has_color([255, 0, 0, 255]))
    })?;
    let initial = h.frames.images.last().unwrap().clone();
    h.visible("Enter send")?;
    h.visible("Alt+Enter newline")?;
    h.visible("F1 help")?;
    let draft = "折りたたんでも保持する下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    let mut previous_width = initial.width;
    for (key, hint) in [
        (b"\x1bn", "Alt+N show Nodes"),
        (b"\x1ba", "Alt+A show Agent"),
    ] {
        h.key(key)?;
        h.visible(hint)?;
        h.wait("expanded complete panel preview", 15.0, |h| {
            h.frames.images.last().is_some_and(|image| {
                image.width > previous_width
                    && image.height == initial.height
                    && image.has_color([255, 0, 0, 255])
            })
        })?;
        previous_width = h.frames.images.last().unwrap().width;
    }
    h.key(b"\t\x1b[Zx")?;
    h.resize(50, 16)?;
    h.wait(
        "complete collapsed preview at minimum terminal size",
        15.0,
        |h| {
            h.frames.images.last().is_some_and(|image| {
                image.width < initial.width
                    && image.height < initial.height
                    && image.has_color([255, 0, 0, 255])
            })
        },
    )?;
    h.visible("Alt+A show Agent")?;
    h.visible("? help")?;
    h.resize(130, 34)?;
    for key in [b"\x1bOR", b"\x1bOQ"] {
        h.key(key)?;
    }
    h.visible("Agent")?;
    h.visible("Nodes")?;
    assert!(!h.screen.text().contains("hide"));
    h.wait("restored complete panel preview", 15.0, |h| {
        h.frames.images.last() == Some(&initial)
    })?;
    h.key(b"\t\t")?;
    h.paste(" 続き")?;
    h.draft(&format!("{draft} 続き"))?;
    no_agent(h);
    h.key(b"\x15")?;
    h.paste("/time invalid")?;
    h.key(b"\r")?;
    h.visible("Use the command palette")?;
    h.key(b"\t\tr")?;
    h.pump_for(4.2)?;
    assert!(
        h.screen.text().contains("Use the command palette"),
        "A preview refresh or notice timer cannot clear an unrelated error"
    );
    h.key(b"\t\x15")?;
    h.paste("/clear-queue")?;
    h.key(b"\r")?;
    h.visible("Prompt queue cleared")?;
    h.wait("notice expires and preview regains its row", 15.0, |h| {
        !h.screen.text().contains("Prompt queue cleared")
            && h.frames.images.last() == Some(&initial)
    })?;
    no_agent(h);
    Ok(())
}

fn navigation(h: &mut Harness) -> Result<Value> {
    h.wait("initial complete navigation preview", 15.0, |h| {
        !h.frames.images.is_empty()
    })?;
    let heading = h.screen.text().lines().nth(1).unwrap().to_owned();
    assert!(matches!(h.screen.find_text("Preview"), Some((36..=37, 1))));
    assert!(matches!(h.screen.find_text("Agent"), Some((101..=102, 1))));
    assert!(!heading.contains("hide"));
    let before_index = h.frames.images.len() - 1;
    let before = h.frames.images[before_index].clone();
    assert!(before.has_color([255, 0, 0, 255]));
    let text = "日本語の下書き\nx a d w s z v t は本文";
    h.paste(text)?;
    h.draft(text)?;
    h.key(b"\x1b[17~\x1b[17~h\x1b[19~")?;
    h.visible("Help")?;
    h.key(b"\x1b[6~\x1b[6~")?;
    h.wait("help content with the configured pan key", 15.0, |h| {
        h.screen.text().contains("a: Preview: pan left")
    })?;
    for _ in 0..64 {
        if h.screen.text().contains("/clear-images") {
            break;
        }
        let previous = h.screen.text();
        h.key(b"\x1b[6~")?;
        h.wait("next help section", 15.0, |h| h.screen.text() != previous)?;
    }
    h.visible("/clear-images")?;
    assert_eq!(h.frames.images.last(), Some(&before));
    h.key(b"\x1b[F")?;
    h.wait("help final wrapped lines", 15.0, |h| {
        h.screen
            .text()
            .contains("Clipboard access happens only on explicit paste.")
            && h.screen.text().contains("protocols do not")
            && h.screen.text().contains("provide clipboard access.")
    })?;
    h.key(b"\x1b[H")?;
    h.visible("switch Nodes, Preview, Prompt")?;
    h.key(b"\x1b")?;
    h.hidden("Help")?;
    let cameras = h.frames.camera_frames;
    let pan_started = Instant::now();
    h.key(b"a")?;
    h.wait("remapped pan actual pixels", 15.0, |h| {
        h.frames.images.last() != Some(&before)
    })?;
    // A complete frame may beat the deferred cached frame. Keep the actual
    // pixel and draft checks; latency is measured by the release PTY benchmark.
    let pan_feedback = json!({"cached":h.frames.camera_frames > cameras,"observer_elapsed_ms":pan_started.elapsed().as_secs_f64()*1000.0});
    let panned_index = h.frames.images.len() - 1;
    let panned = h.frames.images[panned_index].clone();
    let cameras = h.frames.camera_frames;
    let zoom_started = Instant::now();
    h.key(b"z")?;
    h.wait("remapped zoom actual pixels", 15.0, |h| {
        h.frames.images.last() != Some(&panned)
    })?;
    let zoom_feedback = json!({"cached":h.frames.camera_frames > cameras,"observer_elapsed_ms":zoom_started.elapsed().as_secs_f64()*1000.0});
    let zoomed_index = h.frames.images.len() - 1;
    let zoomed = h.frames.images[zoomed_index].clone();
    assert!(panned.has_color([255, 0, 0, 255]) && zoomed.has_color([255, 0, 0, 255]));
    h.key(b"\x1b[20~")?;
    h.wait("configured command palette", 15.0, |h| {
        h.screen.text().contains("Commands") && h.screen.text().contains("CONVERSATION")
    })?;
    h.key(b"x")?;
    h.wait("quit key remains search text", 15.0, |h| {
        h.screen.text().contains("Commands") && h.screen.text().contains('x')
    })?;
    h.key(b"\x1b")?;
    h.hidden("Commands")?;
    h.key(b"\x1b[17~")?;
    h.paste(" 続き")?;
    h.draft(&format!("{text} 続き"))?;
    fs::write(config(h), "[tui.navigation]\nnext_panel=['F10']\n")?;
    h.key(b"\x1b[17~\x1b[17~\x1b[19~")?;
    h.visible("Help")?;
    h.key(b"\x1b")?;
    h.hidden("Help")?;
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
        json!({"source_commit":source_commit(),"case":h.case,"frames":frames,"draft_sha256":hash(format!("{text} 続き")),"help_end_reached":true,"startup_keys_retained":true,"feedback":{"pan":pan_feedback,"zoom":zoom_feedback},"turns":h.turns().len()}),
    )
}

fn permission_modes(h: &mut Harness) -> Result {
    let before = fs::read(&h.document)?;
    let draft = "権限を変更しても下書きを保持";
    h.visible("read-only")?;
    h.paste(draft)?;
    h.draft(draft)?;
    command(h, "Agent permissions")?;
    h.visible("Agent permissions")?;
    h.key(b"\x1b[B\r")?;
    h.visible("Apply permissions")?;
    h.key(b"\r")?;
    h.visible("Permissions changed to workspace-write")?;
    h.draft(draft)?;
    h.save_screen("permission-mode-applied-screen.txt")?;
    command(h, "Agent permissions")?;
    h.visible("Agent permissions")?;
    h.key(b"\x1b[B\r\r")?;
    h.visible("Allow full access?")?;
    h.key(b"\r")?;
    h.visible("Apply permissions")?;
    h.key(b"\x1b")?;
    h.hidden("Apply permissions")?;
    h.visible("workspace-write")?;
    h.resize(80, 24)?;
    command(h, "Agent permissions")?;
    h.visible("Agent permissions")?;
    h.save_screen("permission-mode-compact-screen.txt")?;
    h.key(b"\x1b")?;
    h.hidden("Agent permissions")?;
    h.resize(130, 34)?;
    command(h, "Agent permissions")?;
    h.visible("Agent permissions")?;
    h.key(b"\r")?;
    h.visible("Apply permissions")?;
    h.key(b"\x1b[B\r")?;
    h.visible("Review permission defaults")?;
    h.visible("Trust & apply")?;
    assert!(!h.root.join("lazypencil.toml").exists());
    h.key(b"\r")?;
    h.visible("Permissions changed to workspace-write")?;
    assert!(fs::read_to_string(h.root.join("lazypencil.toml"))?.contains("workspace-write"));
    h.draft(draft)?;
    assert_eq!(fs::read(&h.document)?, before);
    no_agent(h);
    Ok(())
}
