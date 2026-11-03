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
    let point = h.screen.find_text(label).ok_or("Missing mouse button")?;
    click(h, point)
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
        .find_text("nested/deep/selected.pen")
        .ok_or("Missing picker row")?;
    click(h, point)?;
    h.pump_for(0.12)?;
    assert!(h.screen.text().contains("Open a .pen file"));
    assert!(files(&h.root, |name| name == "session.json").is_empty());
    button(h, "[Open]")?;
    h.hidden("Open a .pen file")
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
    h.hidden("Nodes")?;
    button(h, "[N]")?;
    h.visible("Nodes")?;
    button(h, "[Menu]")?;
    h.visible("View controls")?;
    button(h, "[Help]")?;
    h.visible("Help ·")?;
    button(h, "[Close]")?;
    h.hidden("Help ·")?;
    h.resize(50, 16)?;
    h.visible("[...]")?;
    button(h, "[Menu]")?;
    h.visible("View controls")?;
    h.key(b"\x1b")?;
    h.hidden("View controls")?;
    assert_eq!(state(h)["draft"], format!("{draft} 続き"));
    assert_eq!(fs::read(&h.document)?, original);
    assert!(!h.root.join("sent-turn.txt").exists());
    Ok(())
}
