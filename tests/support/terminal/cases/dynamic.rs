use super::*;
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];

fn children(h: &Harness) -> Vec<i32> {
    let result = command_output::output(
        Command::new("ps").args(["-axo", "pid=,ppid="]),
        Duration::from_secs(30),
        1024 * 1024,
    )
    .expect("Observe process children");
    assert!(!result.timed_out && result.status.success());
    String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid = words.next()?.parse().ok()?;
            let parent: u32 = words.next()?.parse().ok()?;
            (parent == h.child.id()).then_some(pid)
        })
        .collect()
}
fn timeline(h: &Harness) -> f64 {
    let text = h.screen.text();
    text.split("t=")
        .skip(1)
        .find_map(|tail| tail.split_once('s').and_then(|(n, _)| n.parse().ok()))
        .unwrap_or(-1.0)
}
fn dynamic_pixels(h: &Harness, page: [u8; 4], shader: [u8; 4]) -> bool {
    [page, GREEN, shader]
        .into_iter()
        .all(|color| h.last_color(color))
}
fn paused(h: &Harness) -> bool {
    !h.screen.text().to_lowercase().contains("playing")
        && !h.screen.text().contains("finishing requested frame")
}
fn frame_cap(h: &Harness, first: usize, started: Instant) {
    assert!(
        h.frames.images.len() - first
            <= (started.elapsed().as_secs_f64() * 2.0).ceil() as usize + 6,
        "Playback respects frame-rate cap"
    );
}

pub fn page(h: &mut Harness) -> Result {
    let document = fs::read(&h.document)?;
    h.wait("initial browser-only red pixels", 15.0, |h| {
        h.last_color(RED)
    })?;
    let started = Instant::now();
    let first = h.frames.images.len();
    h.paste("/play 2")?;
    h.key(b"\r")?;
    h.wait("retained browser-only DOM advances to blue", 15.0, |h| {
        h.last_color(BLUE) && h.screen.text().contains("playing")
    })?;
    let draft = "ページの再生中も残る下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\t ")?;
    h.wait("browser-only playback pause", 15.0, paused)?;
    frame_cap(h, first, started);
    let frames = h.frames.images.len();
    h.pump_for(2.2)?;
    assert!(
        h.frames.images.len() <= frames + 1,
        "Paused pages must not be periodically captured"
    );
    assert!(h.last_color(BLUE));
    assert_eq!(state(h)["draft"], draft);
    no_agent(h);
    assert_eq!(fs::read(&h.document)?, document);
    assert!(
        h.frames.images.iter().any(|f| f.has_color(GREEN)),
        "Retained intermediate frame must be green"
    );
    write_json(
        &h.root.join("page-playback.json"),
        &json!({"source_commit":source_commit(),"case":"page-playback","complete_frames":h.frames.images.len(),"frames_during_playback":frames-first,"elapsed_seconds":started.elapsed().as_secs_f64(),"frame_rate_cap":2,"paused_frame_delta":h.frames.images.len()-frames,"document_unchanged":true,"draft_preserved":true,"agent_session_started":false,"observed_colors":["red","green","blue"]}),
    )
}

pub fn idle(h: &mut Harness) -> Result {
    let document = fs::read(&h.document)?;
    // Throttle process observations without changing the actual idle-expiry deadline.
    let mut sampled = Instant::now() - Duration::from_secs(1);
    let mut pids = Vec::new();
    let mut observed = |h: &Harness| {
        if sampled.elapsed() >= Duration::from_millis(200) {
            pids = children(h);
            sampled = Instant::now();
        }
        pids.clone()
    };
    h.wait("three workers and actual RGB pixels", 15.0, |h| {
        observed(h).len() == 3 && dynamic_pixels(h, RED, BLUE)
    })?;
    let original = observed(h);
    let mut frames = h.frames.images.len();
    h.key(b"\tr")?;
    h.wait("explicit refresh preserves live page DOM", 15.0, |h| {
        h.frames.images.len() > frames && dynamic_pixels(h, MAGENTA, BLUE)
    })?;
    assert_eq!(observed(h), original);
    h.key(b"\x1b[Z")?;
    frames = h.frames.images.len();
    h.paste("/time 1")?;
    h.key(b"\r")?;
    h.wait("time command changes actual shader pixels", 15.0, |h| {
        h.frames.images.len() > frames
            && h.last_color(YELLOW)
            && h.screen.text().contains("t=1.000s")
    })?;
    assert_eq!(observed(h), original, "Seeking reuses existing runtimes");
    assert_eq!(state(h)["draft"], "");
    frames = h.frames.images.len();
    h.pump_for(2.2)?;
    assert_eq!(
        h.frames.images.len(),
        frames,
        "Unchanged time must not repaint"
    );
    h.paste("/time 0")?;
    h.key(b"\r")?;
    h.wait("time command restores shader pixels", 15.0, |h| {
        h.frames.images.len() > frames
            && dynamic_pixels(h, MAGENTA, BLUE)
            && h.screen.text().contains("t=0.000s")
    })?;
    assert_eq!(observed(h), original);
    frames = h.frames.images.len();
    let started = Instant::now();
    h.paste("/play 2")?;
    h.key(b"\r")?;
    h.wait("playback advances shader pixels and clock", 15.0, |h| {
        timeline(h) >= 1.0
            && h.screen.text().contains("playing")
            && dynamic_pixels(h, MAGENTA, YELLOW)
    })?;
    let draft = "待機と更新の後も残る下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    h.key(b"\t ")?;
    h.wait("Space pauses and completes requested frame", 15.0, |h| {
        paused(h) && timeline(h) >= 1.0
    })?;
    assert_eq!(
        observed(h),
        original,
        "Automatic frames reuse runtime processes"
    );
    frame_cap(h, frames, started);
    let paused_time = timeline(h);
    let paused_frames = h.frames.images.len();
    h.pump_for(2.2)?;
    assert_eq!(timeline(h), paused_time);
    assert!(h.frames.images.len() <= paused_frames + 1);
    assert!(dynamic_pixels(h, MAGENTA, YELLOW));
    let previous = observed(h);
    h.wait("automatic idle process cleanup", 75.0, |h| {
        observed(h).is_empty()
    })?;
    for pid in &previous {
        assert_eq!(
            unsafe { libc::kill(*pid, 0) },
            -1,
            "Idle process must be reaped"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
    assert!(dynamic_pixels(h, MAGENTA, YELLOW));
    assert_eq!(state(h)["draft"], draft);
    frames = h.frames.images.len();
    h.key(b"\tr")?;
    h.wait("refresh after cleanup at paused time", 15.0, |h| {
        observed(h).len() == 3 && h.frames.images.len() > frames && dynamic_pixels(h, RED, YELLOW)
    })?;
    assert!(observed(h).iter().all(|pid| !previous.contains(pid)));
    assert_eq!(fs::read(&h.document)?, document);
    assert_eq!(state(h)["draft"], draft);
    no_agent(h);
    Ok(())
}

pub fn script(h: &mut Harness) -> Result {
    let document = fs::read(&h.document)?;
    h.wait("configured script timeout", 15.0, |h| {
        status(h).starts_with("RENDER_TIMEOUT:")
    })?;
    let draft = "制限を変更しても残る下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    let path = config(h).to_owned();
    let configuration =
        fs::read_to_string(&path)?.replace("\ntimeout_ms=100\n", "\ntimeout_ms=2000\n");
    fs::write(&path, &configuration)?;
    let source = h.root.join("generated.js");
    fs::write(
        &source,
        "/** @schema 2.21 */ let total=0;for(let i=0;i<5000000;i++)total+=i;return [{type:'rectangle',width:180,height:90,fill:total>0?'#00FF00':'#FF0000'}];",
    )?;
    let source_bytes = fs::read(&source)?;
    let green =
        |h: &Harness| status(h).starts_with("2 nodes · 0 diagnostics ·") && h.last_color(GREEN);
    h.wait("recovered script preview", 15.0, green)?;
    let frames = h.frames.images.len();
    let configuration = configuration.replace("heap_mib=8", "heap_mib=12");
    fs::write(
        &path,
        configuration.replace("\ntimeout_ms=2000\n", "\ntimeout_ms=1\n"),
    )?;
    h.wait("policy-only timeout invalidates preview", 15.0, |h| {
        status(h).starts_with("RENDER_TIMEOUT:")
    })?;
    fs::write(&path, configuration)?;
    h.wait("policy-only repair restores preview", 15.0, |h| {
        h.frames.images.len() > frames && green(h)
    })?;
    assert_eq!(fs::read(source)?, source_bytes);
    h.wait("zero-idle script cleanup", 15.0, |h| children(h).is_empty())?;
    let layout = h.cli(&["layout".as_ref(), h.document.as_os_str()])?;
    let limits = &layout["script_executions"][0]["limits"];
    assert_eq!(limits["heap_mib"], 12);
    assert_eq!(limits["timeout_ms"], 2000);
    assert_eq!(limits["idle_seconds"], 0);
    assert_eq!(state(h)["draft"], draft);
    no_agent(h);
    assert_eq!(fs::read(&h.document)?, document);
    Ok(())
}
