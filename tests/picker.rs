use a2amx::picker::{
    EntryState, Line, Outcome, PickerAction, PickerEntry, PickerParser, PickerView,
    scroll_into_view, target,
};

#[test]
fn scroll_into_view_uses_the_smallest_visible_window() {
    let cases = [
        ((0, 0, 5, 12), 0),
        ((0, 5, 5, 12), 1),
        ((1, 0, 5, 12), 0),
        ((3, 5, 5, 12), 3),
        ((0, 11, 5, 12), 7),
        ((9, 3, 5, 12), 3),
        ((0, 0, 0, 12), 0),
        ((4, 2, 5, 5), 0),
        ((10, 11, 5, 12), 7),
    ];

    for ((offset, selection, height, total), expected) in cases {
        assert_eq!(
            scroll_into_view(offset, selection, height, total),
            expected,
            "offset={offset}, selection={selection}, height={height}, total={total}"
        );
    }
}

#[test]
fn parser_maps_picker_keys_and_retains_split_sequences() {
    let sequences = [
        (b"\x1b[H".as_slice(), PickerAction::Home),
        (b"\x1bOH".as_slice(), PickerAction::Home),
        (b"\x1b[1~".as_slice(), PickerAction::Home),
        (b"\x1b[F".as_slice(), PickerAction::End),
        (b"\x1bOF".as_slice(), PickerAction::End),
        (b"\x1b[4~".as_slice(), PickerAction::End),
        (b"\x1b[5~".as_slice(), PickerAction::Page(-1)),
        (b"\x1b[6~".as_slice(), PickerAction::Page(1)),
    ];

    for (sequence, expected) in sequences {
        let mut parser = PickerParser::default();
        let mut actions = Vec::new();
        for byte in sequence {
            actions.extend(parser.feed(&[*byte], false));
        }
        assert_eq!(actions, vec![expected], "sequence={sequence:?}");
    }

    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1b[5", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"~", false), vec![PickerAction::Page(-1)]);
}

#[test]
fn parser_preserves_existing_keys_and_recovers_from_unknown_sequences() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1b[A", false), vec![PickerAction::Move(-1)]);
    assert_eq!(parser.feed(b"j", false), vec![PickerAction::Move(1)]);
    assert_eq!(parser.feed(b"\r", false), vec![PickerAction::Select]);

    assert!(parser.feed(b"\x1b[9~", false).is_empty());
    assert_eq!(parser.feed(b"j", false), vec![PickerAction::Move(1)]);
}

#[test]
fn parser_maps_an_alone_escape_to_cancel() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(&[0x1b], true), vec![PickerAction::Cancel]);
}

#[test]
fn target_clamps_moves_pages_and_absolute_keys() {
    let cases = [
        ((0, PickerAction::Move(1), 5, 12), 1),
        ((0, PickerAction::Move(-1), 5, 12), 0),
        ((11, PickerAction::Move(1), 5, 12), 11),
        ((0, PickerAction::Page(1), 5, 12), 4),
        ((10, PickerAction::Page(1), 5, 12), 11),
        ((6, PickerAction::Page(-1), 5, 12), 2),
        ((2, PickerAction::Page(-1), 5, 12), 0),
        ((3, PickerAction::Page(1), 0, 12), 4),
        ((3, PickerAction::Home, 5, 12), 0),
        ((3, PickerAction::End, 5, 12), 11),
        ((0, PickerAction::End, 5, 0), 0),
        ((7, PickerAction::Select, 5, 12), 7),
        // A selection past the end (the list shrank) is bounded before moving.
        ((7, PickerAction::Move(1), 5, 3), 2),
        ((7, PickerAction::Move(-1), 5, 3), 1),
        ((7, PickerAction::Page(-1), 5, 3), 0),
    ];

    for ((selection, action, height, total), expected) in cases {
        assert_eq!(
            target(selection, action, height, total),
            expected,
            "selection={selection}, action={action:?}, height={height}, total={total}"
        );
    }
}

#[test]
fn parser_handles_text_after_a_pending_escape() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1bj", false), vec![PickerAction::Move(1)]);
}

#[test]
fn parser_maps_fold_filter_and_filter_text_keys() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b" ", false), vec![PickerAction::Toggle]);
    assert_eq!(parser.feed(b"h", false), vec![PickerAction::Fold]);
    assert_eq!(parser.feed(b"l", false), vec![PickerAction::Unfold]);
    assert_eq!(parser.feed(b"/", false), vec![PickerAction::StartFilter]);
    assert_eq!(parser.feed(b"\x1b[D", false), vec![PickerAction::Fold]);
    assert_eq!(parser.feed(b"\x1b[C", false), vec![PickerAction::Unfold]);
    assert_eq!(parser.feed(b"\x1bOD", false), vec![PickerAction::Fold]);
    assert_eq!(parser.feed(b"\x1bOC", false), vec![PickerAction::Unfold]);

    parser.set_filter_mode(true);
    for byte in *b"qjkhl/x " {
        assert_eq!(parser.feed(&[byte], false), vec![PickerAction::Char(byte)]);
    }
    assert_eq!(parser.feed(&[0x7f], false), vec![PickerAction::Backspace]);
    assert_eq!(parser.feed(&[0x08], false), vec![PickerAction::Backspace]);
    assert_eq!(parser.feed(b"\r", false), vec![PickerAction::Select]);
    assert_eq!(parser.feed(b"\x1b[A", false), vec![PickerAction::Move(-1)]);
    assert_eq!(parser.feed(b"\x1b", true), vec![PickerAction::Cancel]);
    assert_eq!(parser.feed(b"\x1bb", false), vec![PickerAction::Char(b'b')]);

    parser.set_filter_mode(false);
    assert_eq!(parser.feed(b"\x1bj", false), vec![PickerAction::Move(1)]);
    assert_eq!(parser.feed(b"\x1b[A", false), vec![PickerAction::Move(-1)]);
}

fn picker_entry(
    id: &str,
    name: &str,
    team: Option<&str>,
    private: bool,
    state: EntryState,
    held: bool,
) -> PickerEntry {
    PickerEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        team: team.map(str::to_owned),
        private,
        state,
        held,
    }
}

fn picker_fixture() -> Vec<PickerEntry> {
    vec![
        picker_entry("s1", "base", None, false, EntryState::Idle, false),
        picker_entry(
            "s2",
            "al-1",
            Some("alpha"),
            true,
            EntryState::Working,
            false,
        ),
        picker_entry("s3", "al-2", Some("alpha"), true, EntryState::Idle, true),
        picker_entry(
            "s4",
            "be-1",
            Some("beta"),
            false,
            EntryState::Working,
            false,
        ),
        picker_entry("s5", "be-2", Some("beta"), false, EntryState::Exited, false),
        picker_entry("s6", "ga-1", Some("gamma"), false, EntryState::Idle, false),
    ]
}

#[test]
fn picker_view_initializes_folded_team_sections_and_current_selection() {
    let view = PickerView::new(picker_fixture(), "s1");
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("alpha".into()),
                text: "[+] alpha (private) (2): 1 working, 1 idle, 1 held".into(),
            },
            Line::Header {
                key: Some("beta".into()),
                text: "[+] beta (2): 1 working, 1 exited".into(),
            },
            Line::Header {
                key: Some("gamma".into()),
                text: "[+] gamma (1): 1 idle".into(),
            },
            Line::Header {
                key: None,
                text: "[-] (no team) (1)".into(),
            },
            Line::Session { id: "s1".into() },
        ]
    );
    assert_eq!(view.selected_line(), Some(4));
    assert_eq!(view.selected_session(), Some("s1"));
}

#[test]
fn picker_view_toggles_headers_and_folds_from_a_session() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    assert!(view.apply(PickerAction::Home, 5).is_none());
    assert!(view.apply(PickerAction::Select, 5).is_none());
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("alpha".into()),
                text: "[-] alpha (private) (2)".into(),
            },
            Line::Session { id: "s2".into() },
            Line::Session { id: "s3".into() },
            Line::Header {
                key: Some("beta".into()),
                text: "[+] beta (2): 1 working, 1 exited".into(),
            },
            Line::Header {
                key: Some("gamma".into()),
                text: "[+] gamma (1): 1 idle".into(),
            },
            Line::Header {
                key: None,
                text: "[-] (no team) (1)".into(),
            },
            Line::Session { id: "s1".into() },
        ]
    );
    assert_eq!(view.selected_line(), Some(0));
    assert!(view.apply(PickerAction::Move(1), 5).is_none());
    assert_eq!(view.selected_session(), Some("s2"));
    let before = view.lines();
    assert!(view.apply(PickerAction::Unfold, 5).is_none());
    assert!(view.apply(PickerAction::Toggle, 5).is_none());
    assert_eq!(view.lines(), before);
    assert!(view.apply(PickerAction::Fold, 5).is_none());
    assert_eq!(view.selected_line(), Some(0));
    assert_eq!(
        view.lines()[0],
        Line::Header {
            key: Some("alpha".into()),
            text: "[+] alpha (private) (2): 1 working, 1 idle, 1 held".into(),
        }
    );
}

#[test]
fn picker_view_expands_small_fleets_and_stays_flat_without_teams() {
    let small = picker_fixture()
        .into_iter()
        .filter(|entry| matches!(entry.team.as_deref(), Some("alpha" | "beta")))
        .collect();
    let view = PickerView::new(small, "s2");
    assert!(view.lines().iter().all(|line| match line {
        Line::Header { text, .. } => !text.starts_with("[+]"),
        Line::Session { .. } => true,
    }));

    let flat = vec![
        picker_entry("s1", "base", None, false, EntryState::Idle, false),
        picker_entry("s7", "solo", None, false, EntryState::Unknown, false),
    ];
    let view = PickerView::new(flat, "s1");
    assert_eq!(
        view.lines(),
        vec![
            Line::Session { id: "s1".into() },
            Line::Session { id: "s7".into() },
        ]
    );
}

#[test]
fn picker_view_opens_the_current_team_when_many_teams_are_folded() {
    let view = PickerView::new(picker_fixture(), "s5");
    assert_eq!(view.selected_session(), Some("s5"));
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("alpha".into()),
                text: "[+] alpha (private) (2): 1 working, 1 idle, 1 held".into(),
            },
            Line::Header {
                key: Some("beta".into()),
                text: "[-] beta (2)".into(),
            },
            Line::Session { id: "s4".into() },
            Line::Session { id: "s5".into() },
            Line::Header {
                key: Some("gamma".into()),
                text: "[+] gamma (1): 1 idle".into(),
            },
            Line::Header {
                key: None,
                text: "[+] (no team) (1): 1 idle".into(),
            },
        ]
    );
}

#[test]
fn picker_view_filters_by_session_name_and_team_then_cancels() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    view.apply(PickerAction::StartFilter, 5);
    for byte in b"be-2" {
        view.apply(PickerAction::Char(*byte), 5);
    }
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("beta".into()),
                text: "[-] beta (1)".into(),
            },
            Line::Session { id: "s5".into() },
        ]
    );
    assert_eq!(view.selected_session(), Some("s5"));
    view.apply(PickerAction::Backspace, 5);
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("beta".into()),
                text: "[-] beta (2)".into(),
            },
            Line::Session { id: "s4".into() },
            Line::Session { id: "s5".into() },
        ]
    );
    assert_eq!(view.selected_session(), Some("s5"));
    assert_eq!(view.apply(PickerAction::Cancel, 5), None);
    assert!(!view.filter_mode());
    assert_eq!(view.query(), "");
    assert_eq!(view.selected_line(), Some(1));
    assert_eq!(view.apply(PickerAction::Cancel, 5), Some(Outcome::Cancel));
}

#[test]
fn picker_view_filter_matches_team_case_insensitively_and_handles_empty_results() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    view.apply(PickerAction::StartFilter, 5);
    for byte in b"alpha" {
        view.apply(PickerAction::Char(*byte), 5);
    }
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("alpha".into()),
                text: "[-] alpha (private) (2)".into(),
            },
            Line::Session { id: "s2".into() },
            Line::Session { id: "s3".into() },
        ]
    );
    view.apply(PickerAction::Backspace, 5);
    view.apply(PickerAction::Backspace, 5);
    view.apply(PickerAction::Backspace, 5);
    view.apply(PickerAction::Backspace, 5);
    view.apply(PickerAction::Backspace, 5);
    for byte in b"GA" {
        view.apply(PickerAction::Char(*byte), 5);
    }
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("gamma".into()),
                text: "[-] gamma (1)".into(),
            },
            Line::Session { id: "s6".into() },
        ]
    );
    view.apply(PickerAction::Backspace, 5);
    view.apply(PickerAction::Backspace, 5);
    for byte in b"zz" {
        view.apply(PickerAction::Char(*byte), 5);
    }
    assert!(view.lines().is_empty());
    assert_eq!(view.selected_line(), None);
}

#[test]
fn picker_view_selects_sessions_and_cancels_without_filter() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    assert_eq!(
        view.apply(PickerAction::Select, 5),
        Some(Outcome::Attach("s1".into()))
    );
    assert_eq!(view.apply(PickerAction::Cancel, 5), Some(Outcome::Cancel));
}

#[test]
fn picker_view_replace_preserves_folds_and_falls_back_by_identity() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    view.apply(PickerAction::Home, 5);
    view.apply(PickerAction::Select, 5);
    view.apply(PickerAction::Move(1), 5);
    view.apply(PickerAction::Move(1), 5);
    assert_eq!(view.selected_session(), Some("s3"));
    let without_s3 = picker_fixture()
        .into_iter()
        .filter(|entry| entry.id != "s3")
        .collect();
    view.replace(without_s3);
    assert_eq!(
        view.lines()[0],
        Line::Header {
            key: Some("alpha".into()),
            text: "[-] alpha (private) (1)".into(),
        }
    );
    assert_eq!(view.selected_line(), Some(2));
    assert_eq!(
        view.lines()[2],
        Line::Header {
            key: Some("beta".into()),
            text: "[+] beta (2): 1 working, 1 exited".into(),
        }
    );

    let mut view = PickerView::new(picker_fixture(), "s1");
    view.apply(PickerAction::Home, 5);
    view.apply(PickerAction::Select, 5);
    view.apply(PickerAction::Move(1), 5);
    view.apply(PickerAction::Move(1), 5);
    view.replace(picker_fixture());
    assert_eq!(view.selected_session(), Some("s3"));
    assert_eq!(view.selected_line(), Some(2));
}
