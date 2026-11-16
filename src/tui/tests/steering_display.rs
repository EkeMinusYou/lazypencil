#[test]
fn mid_turn_messages_keep_user_styling_separate_from_streamed_replies() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "echo");
    app.append_user("First request");
    app.append_agent("Checking the design.");
    for (id, text, reply) in [
        (
            "first",
            "Keep the proportions.\nChange only the colors.",
            "Understood.",
        ),
        ("second", "Use blue.", "Applying blue."),
    ] {
        app.outbox.steering = Some(outbox::SteeringPrompt {
            id: id.into(),
            text: text.into(),
        });
        app.message(&json!({"type":"steering_result","id":id,"outcome":"injected"}));
        app.message(&json!({"type":"update","notification":{"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":reply}
        }}}));
    }
    let buffer = test_artifacts::snapshot(&mut app, "mid-turn-messages");
    let rows = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(!rows.iter().any(|row| row.contains("You (mid-turn):")));
    let row = |text: &str| rows.iter().position(|row| row.contains(text)).unwrap() as u16;
    let initial = row("First request");
    let x = (0..buffer.area.width)
        .find(|&x| buffer[(x, initial)].symbol() == "›")
        .unwrap();
    for text in [
        "First request",
        "Keep the proportions.",
        "Change only the colors.",
        "Use blue.",
    ] {
        assert_eq!(
            buffer[(x, row(text))].bg,
            theme::colors().user_background,
            "{text}"
        );
    }
    for text in ["Keep the proportions.", "Use blue."] {
        assert_eq!(buffer[(x, row(text))].symbol(), "›");
    }
    for text in ["Checking the design.", "Understood.", "Applying blue."] {
        assert_ne!(
            buffer[(x, row(text))].bg,
            theme::colors().user_background,
            "{text}"
        );
    }
    assert!(row("Change only the colors.") + 1 < row("Understood."));
    assert!(row("Use blue.") + 1 < row("Applying blue."));
}

#[test]
fn mid_turn_and_live_agent_messages_keep_literal_markers_in_typed_records() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "echo");
    let literal = "Agent:\nYou: literal\nRequest complete\n[0 images]\n[completed] literal";
    app.outbox.steering = Some(outbox::SteeringPrompt {
        id: "literal".into(),
        text: literal.into(),
    });
    app.message(&json!({"type":"steering_result","id":"literal","outcome":"injected"}));
    app.message(&json!({"type":"update","notification":{"update":{
        "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":literal}
    }}}));
    let entries = app.log.iter().collect::<Vec<_>>();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].kind, transcript::Kind::User);
    assert_eq!(entries[1].kind, transcript::Kind::Agent);
    assert!(entries.iter().all(|entry| entry.text == literal));
}
