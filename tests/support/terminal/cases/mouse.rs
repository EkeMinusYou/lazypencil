use super::*;

fn event(h: &mut Harness, button: u8, point: (u16, u16), released: bool) -> Result {
    h.key(
        format!(
            "\x1b[<{button};{};{}{}",
            point.0 + 1,
            point.1 + 1,
            if released { 'm' } else { 'M' }
        )
        .as_bytes(),
    )
}

fn click(h: &mut Harness, point: (u16, u16)) -> Result {
    event(h, 0, point, false)?;
    // Redrawing between press and release must not discard a valid click.
    h.pump_for(0.08)?;
    event(h, 0, point, true)
}

fn button(h: &mut Harness, label: &str) -> Result {
    if label == "[Menu]" {
        let point = h
            .screen
            .find_text("[Menu]")
            .or_else(|| h.screen.find_text("[...]"))
            .ok_or("Missing view menu")?;
        return click(h, point);
    }
    if matches!(label, "[N]" | "[A]") && h.screen.find_text(label).is_none() {
        button(h, "[Menu]")?;
        h.visible("View controls")?;
    }
    h.visible(label)?;
    let modal_toggle = matches!(label, "[N]" | "[A]") && h.screen.text().contains("View controls");
    let point = h.screen.find_text(label).ok_or("Missing mouse button")?;
    click(h, point)?;
    if modal_toggle {
        // Wait for the toggle, rather than mistaking panes hidden behind the
        // modal for panes that the application has already collapsed.
        h.hidden("View controls")?;
    }
    Ok(())
}

fn zoom(h: &Harness) -> Option<u32> {
    let text = h.screen.text();
    let prefix = text.split('%').next()?;
    let digits = prefix
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.chars().rev().collect::<String>().parse().ok()
}

pub fn picker(h: &mut Harness) -> Result {
    let point = h
        .screen
        .find_text("selected.pen")
        .ok_or("Missing picker row")?;
    click(h, point)?;
    h.pump_for(0.12)?;
    assert!(h.screen.text().contains("Find a .pen file…"));
    assert!(files(&h.root, |name| name == "session.json").is_empty());
    button(h, "[Enter Open]")?;
    h.hidden("Find a .pen file…")
}

pub fn workspace(h: &mut Harness) -> Result {
    h.visible("Mouse row 0")?;
    h.wait("initial mouse camera", 15.0, |h| zoom(h).is_some())?;
    let original = fs::read(&h.document)?;
    let draft = "マウス操作中も残る下書き";
    h.paste(draft)?;
    h.draft(draft)?;
    let preview = h.screen.find_text("Preview").ok_or("Missing Preview")?;
    let point = (preview.0 + 15, preview.1 + 8);
    if h.case == "mouse-disabled" {
        assert!(!h.contains_bytes(b"\x1b[?1006h"));
        let before = zoom(h);
        event(h, 80, point, false)?;
        h.pump_for(0.2)?;
        assert_eq!(zoom(h), before);
        h.draft(draft)?;
        return Ok(());
    }
    assert!(h.contains_bytes(b"\x1b[?1006h"));
    if h.case == "mouse-kitty" {
        let initial = h
            .frames
            .images
            .last()
            .ok_or("Missing rendered preview")?
            .clone();
        event(h, 65, point, false)?;
        h.wait(
            "unmodified trackpad scrolling changes rendered pixels",
            15.0,
            |h| {
                h.frames
                    .images
                    .last()
                    .is_some_and(|image| image.rgba() != initial.rgba())
            },
        )?;
    }
    let before = zoom(h);
    event(h, 80, point, false)?;
    h.wait("Ctrl wheel zooms the actual preview", 15.0, |h| {
        zoom(h).is_some_and(|zoom| Some(zoom) != before)
    })?;
    // A trackpad burst must preserve every step and the following text input.
    h.key(
        format!("\x1b[<67;{};{}M", point.0 + 1, point.1 + 1)
            .repeat(256)
            .as_bytes(),
    )?;
    h.paste(" 続き")?;
    h.draft(&format!("{draft} 続き"))?;
    let row = h
        .screen
        .find_text("Mouse row 0")
        .ok_or("Missing node row")?;
    event(h, 65, row, false)?;
    h.hidden("Mouse row 0")?;
    h.visible("Mouse row 2")?;
    let row = h
        .screen
        .find_text("Mouse row 2")
        .ok_or("Missing scrolled row")?;
    click(h, row)?;
    h.pump_for(0.1)?;
    assert!(!h.screen.text().contains("References ·"));
    h.key(b"f")?;
    h.wait("clicked node fits", 15.0, |h| {
        zoom(h).is_some_and(|zoom| zoom > 100)
    })?;
    button(h, "[Fit]")?;
    h.wait("Fit restores document camera", 15.0, |h| {
        zoom(h).is_some_and(|zoom| zoom < 100)
    })?;
    click(h, point)?;
    h.key(b"\x1b[32;1:1u")?;
    event(h, 0, point, false)?;
    event(h, 32, (point.0 + 4, point.1 + 2), false)?;
    event(h, 0, (point.0 + 4, point.1 + 2), true)?;
    h.key(b"\x1b[32;1:3u")?;
    button(h, "[N]")?;
    h.hidden("▤ Nodes")?;
    button(h, "[N]")?;
    h.visible("▤ Nodes")?;
    button(h, "[Menu]")?;
    h.visible("View controls")?;
    button(h, "[Help]")?;
    h.visible("Help")?;
    button(h, "[Esc]")?;
    h.hidden("Help")?;
    h.resize(50, 16)?;
    h.visible("[...]")?;
    button(h, "[Menu]")?;
    h.visible("View controls")?;
    h.key(b"\x1b")?;
    h.hidden("View controls")?;
    if h.case == "mouse-kitty" {
        let trace = fs::read_to_string(h.root.join("terminal-errors.log"))?;
        assert!(
            trace
                .lines()
                .filter_map(|line| {
                    serde_json::from_str::<Value>(line.strip_prefix("LAZYPENCIL_INPUT_PROFILE ")?)
                        .ok()
                })
                .any(|event| event["event"] == "pointer"
                    && event["kind"] == "ScrollDown"
                    && event["modifiers"] == 0
                    && event["target"] == "Canvas"
                    && event["scope_valid"] == true)
        );
    }
    assert_eq!(state(h)["draft"], format!("{draft} 続き"));
    assert_eq!(fs::read(&h.document)?, original);
    assert!(!h.root.join("sent-turn.txt").exists());
    Ok(())
}

pub fn canvas_selection(h: &mut Harness) -> Result {
    h.visible("Left canvas shape")?;
    h.wait("selection camera ready", 15.0, |h| zoom(h).is_some())?;
    let original = fs::read(&h.document)?;
    h.paste("Selection keeps this draft")?;
    h.draft("Selection keeps this draft")?;
    let row = h
        .screen
        .find_text("Left canvas shape")
        .ok_or("Missing shape row")?;
    click(h, row)?;
    h.key(b"f")?;
    h.wait("node fit completes", 15.0, |h| {
        zoom(h).is_some_and(|z| z > 300)
    })?;
    if h.case == "selection-kitty" {
        h.wait("node fit outline appears", 15.0, |h| {
            h.frames
                .images
                .last()
                .is_some_and(|f| f.has_color([137, 180, 250, 255]))
        })?;
    }
    let fitted_zoom = zoom(h);
    let frames_before_clear = h.frames.images.len();
    h.key(b"\x1b")?;
    h.pump_for(0.15)?;
    assert_eq!(
        zoom(h),
        fitted_zoom,
        "Clearing the outline must keep the fitted camera"
    );
    if h.case == "selection-kitty" {
        h.wait("node escape removes the fit outline", 15.0, |h| {
            h.frames.images.len() > frames_before_clear
                && h.frames
                    .images
                    .last()
                    .is_some_and(|f| !f.has_color([137, 180, 250, 255]))
        })?;
    }
    click(h, row)?;
    h.key(b"f")?;
    let preview = h.screen.find_text("Preview").ok_or("Missing Preview")?;
    let agent = h.screen.find_text("Agent").ok_or("Missing Agent")?;
    let center = ((preview.0 + agent.0 - 4) / 2, 16);
    click(h, (center.0, preview.1 + 2))?;
    h.visible("Selection cleared")?;
    h.key(b"]")?;
    h.visible("1 node selected · Left canvas shape")?;
    h.wait("fit selection", 15.0, |h| zoom(h).is_some_and(|z| z > 300))?;
    let frames = h.frames.images.len();
    let camera_frames = h.frames.camera_frames;
    click(h, center)?;
    h.visible("1 node selected")?;
    if h.case == "selection-kitty" {
        h.wait("selection outline is rendered", 15.0, |h| {
            // A completed exact frame can supersede temporary camera feedback.
            (h.frames.images.len() > frames
                && h.frames
                    .images
                    .last()
                    .is_some_and(|f| f.has_color([137, 180, 250, 255])))
                || (h.frames.camera_frames > camera_frames
                    && h.frames
                        .last_camera
                        .as_ref()
                        .is_some_and(|f| f.has_color([137, 180, 250, 255])))
        })?;
    }
    let parent_zoom = zoom(h).ok_or("Missing parent zoom")?;
    let frames = h.frames.images.len();
    h.key(b"[")?;
    h.visible("1 node selected · Canvas child")?;
    h.wait("child selection fits the camera", 15.0, |h| {
        zoom(h).is_some_and(|z| z > parent_zoom)
    })?;
    if h.case == "selection-kitty" {
        h.wait("child outline is rendered", 15.0, |h| {
            h.frames.images.len() > frames
                && h.frames
                    .images
                    .last()
                    .is_some_and(|f| f.has_color([137, 180, 250, 255]))
        })?;
    }
    h.key(b"]")?;
    h.visible("1 node selected · Left canvas shape")?;
    h.wait("parent selection fits the camera", 15.0, |h| {
        zoom(h) == Some(parent_zoom)
    })?;
    h.key(b"0")?;
    h.wait("fit full document", 15.0, |h| {
        zoom(h).is_some_and(|z| z < 300)
    })?;
    let start = (preview.0 - 2, preview.1 + 2);
    let end = (agent.0 - 6, 29);
    if h.case == "selection-kitty" {
        // Inspect actual pixels before mouse-up, including a drag over empty space.
        click(h, start)?;
        h.visible("Selection cleared")?;
        let frames = h.frames.camera_frames;
        event(h, 0, start, false)?;
        let corner = (start.0 + 8, start.1 + 4);
        event(h, 32, corner, false)?;
        h.wait("marquee border appears before release", 15.0, |h| {
            h.frames.camera_frames > frames
                && h.frames
                    .last_camera
                    .as_ref()
                    .is_some_and(|f| f.has_color([137, 180, 250, 255]))
        })?;
        let first = h.frames.last_camera.as_ref().unwrap().rgba().to_vec();
        let corner = (corner.0 + 5, corner.1 + 2);
        event(h, 32, corner, false)?;
        h.wait(
            "marquee border follows the drag before release",
            15.0,
            |h| {
                h.frames
                    .last_camera
                    .as_ref()
                    .is_some_and(|f| f.has_color([137, 180, 250, 255]) && f.rgba() != first)
            },
        )?;
        event(h, 0, corner, true)?;
        h.wait("empty marquee disappears on release", 15.0, |h| {
            h.frames
                .images
                .last()
                .is_some_and(|f| !f.has_color([137, 180, 250, 255]))
        })?;
    }
    for (from, to) in [(start, end), (end, start)] {
        event(h, 0, from, false)?;
        event(h, 32, to, false)?;
        event(h, 0, to, true)?;
        h.visible("3 nodes selected")?;
    }
    h.key(b"\x1b")?;
    h.pump_for(0.15)?;
    if h.case == "selection-kitty" {
        let old_width = h.frames.images.last().unwrap().width;
        h.resize(247, 57)?;
        h.wait("large selection viewport", 15.0, |h| {
            h.frames
                .images
                .last()
                .is_some_and(|frame| frame.width > old_width)
        })?;
        let preview = h
            .screen
            .find_text("Preview")
            .ok_or("Missing resized Preview")?;
        let agent = h.screen.find_text("Agent").ok_or("Missing resized Agent")?;
        let origin = (preview.0 - 2, preview.1 + 2);
        let end = (agent.0 - 6, 50);
        event(h, 0, origin, false)?;
        let mut burst = String::new();
        for index in 0..128 {
            let point = if index % 2 == 0 {
                end
            } else {
                (end.0 - 4, end.1 - 4)
            };
            burst.push_str(&format!("\x1b[<32;{};{}M", point.0 + 1, point.1 + 1));
        }
        h.key(burst.as_bytes())?;
        event(h, 0, end, true)?;
        h.key(b"\x1bOP")?;
        h.wait(
            "large drag bursts keep command input responsive",
            3.0,
            |h| h.screen.text().contains("Help"),
        )?;
        h.key(b"\x1b")?;
    }
    assert_eq!(fs::read(&h.document)?, original);
    assert_eq!(state(h)["draft"], "Selection keeps this draft");
    assert!(!h.root.join("sent-turn.txt").exists());
    Ok(())
}
