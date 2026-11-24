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
fn parser_maps_normal_q_to_cancel() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"q", false), vec![PickerAction::Cancel]);
}

#[test]
fn parser_maps_normal_k_to_move_up() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"k", false), vec![PickerAction::Move(-1)]);
}

#[test]
fn parser_maps_csi_down_to_move_down() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1b[B", false), vec![PickerAction::Move(1)]);
}

#[test]
fn parser_maps_ss3_up_to_move_up() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1bOA", false), vec![PickerAction::Move(-1)]);
}

#[test]
fn parser_maps_ss3_down_to_move_down() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1bOB", false), vec![PickerAction::Move(1)]);
}

#[test]
fn parser_drops_unmapped_normal_bytes() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"x\t", false), Vec::<PickerAction>::new());
}

#[test]
fn parser_unknown_three_byte_escape_swallows_the_next_byte() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1b[Zj", false), Vec::<PickerAction>::new());
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

#[test]
fn parser_handles_split_escape_restart_and_escape_alone_reset() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1b", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"[", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"A", false), vec![PickerAction::Move(-1)]);

    assert_eq!(parser.feed(b"\x1b[", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"A", false), vec![PickerAction::Move(-1)]);

    assert_eq!(parser.feed(b"\x1bx", false), Vec::<PickerAction>::new());
    assert_eq!(
        parser.feed(b"\x1b\x1b[A", false),
        vec![PickerAction::Move(-1)]
    );
    assert_eq!(
        parser.feed(b"\x1b[\x1b[A", false),
        Vec::<PickerAction>::new()
    );

    assert_eq!(parser.feed(b"\x1b[5x", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"j", false), vec![PickerAction::Move(1)]);

    assert_eq!(parser.feed(b"\x1b", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"\x1b", true), vec![PickerAction::Cancel]);
    assert_eq!(parser.feed(b"[A", false), Vec::<PickerAction>::new());

    assert_eq!(
        parser.feed(b"jk\r", false),
        vec![
            PickerAction::Move(1),
            PickerAction::Move(-1),
            PickerAction::Select,
        ]
    );

    parser.set_filter_mode(true);
    assert_eq!(
        parser.feed(&[0x1f, 0x20, 0x7e, 0x80], false),
        vec![PickerAction::Char(0x20), PickerAction::Char(0x7e)]
    );
}

#[test]
fn picker_view_apply_guards_empty_selection_filter_mode_and_header_select() {
    let mut empty = PickerView::new(vec![], "s1");
    for action in [
        PickerAction::Move(1),
        PickerAction::Page(1),
        PickerAction::Home,
        PickerAction::End,
        PickerAction::Select,
    ] {
        assert_eq!(empty.apply(action, 5), None);
        assert_eq!(empty.lines(), Vec::<Line>::new());
        assert_eq!(empty.selected_line(), None);
        assert_eq!(empty.selected_session(), None);
    }

    let mut view = PickerView::new(picker_fixture(), "s1");
    assert_eq!(view.apply(PickerAction::Char(b'a'), 5), None);
    assert_eq!(view.apply(PickerAction::Backspace, 5), None);
    assert!(!view.filter_mode());
    assert_eq!(view.query(), "");
    assert_eq!(view.selected_session(), Some("s1"));

    assert_eq!(view.apply(PickerAction::StartFilter, 5), None);
    assert!(view.filter_mode());
    assert_eq!(view.apply(PickerAction::Backspace, 5), None);
    assert_eq!(view.query(), "");
    assert_eq!(view.selected_session(), Some("s1"));

    assert_eq!(view.apply(PickerAction::Char(b'a'), 5), None);
    assert_eq!(view.apply(PickerAction::Char(b'l'), 5), None);
    assert_eq!(view.query(), "al");
    assert_eq!(view.apply(PickerAction::Home, 5), None);
    assert_eq!(view.selected_line(), Some(0));
    assert_eq!(view.selected_session(), None);
    let filtered_lines = view.lines();
    assert_eq!(view.apply(PickerAction::Select, 5), None);
    assert_eq!(view.lines(), filtered_lines);

    assert_eq!(view.apply(PickerAction::StartFilter, 5), None);
    assert!(view.filter_mode());
    assert_eq!(view.query(), "");
}

#[test]
fn picker_view_backspace_to_empty_uses_reconcile_empty_with_folded_team() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    view.apply(PickerAction::StartFilter, 5);
    view.apply(PickerAction::Char(b'a'), 5);
    view.apply(PickerAction::Char(b'l'), 5);
    assert_eq!(view.selected_session(), Some("s2"));
    assert_eq!(view.apply(PickerAction::Backspace, 5), None);
    assert_eq!(view.query(), "a");
    assert_eq!(view.selected_session(), Some("s2"));

    assert_eq!(view.apply(PickerAction::Backspace, 5), None);
    assert!(view.filter_mode());
    assert_eq!(view.query(), "");
    assert_eq!(view.selected_line(), Some(0));
    assert_eq!(view.selected_session(), None);
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
}

#[test]
fn picker_view_fold_actions_on_headers_no_team_unsectioned_and_filtered() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    view.apply(PickerAction::Home, 5);
    let folded_lines = view.lines();
    view.apply(PickerAction::Fold, 5);
    assert_eq!(view.lines(), folded_lines);

    view.apply(PickerAction::Unfold, 5);
    assert_eq!(
        view.lines()[0],
        Line::Header {
            key: Some("alpha".into()),
            text: "[-] alpha (private) (2)".into(),
        }
    );
    let expanded_lines = view.lines();
    view.apply(PickerAction::Unfold, 5);
    assert_eq!(view.lines(), expanded_lines);

    view.apply(PickerAction::Toggle, 5);
    assert_eq!(view.lines(), folded_lines);
    view.apply(PickerAction::Toggle, 5);
    assert_eq!(view.lines(), expanded_lines);

    let mut view = PickerView::new(picker_fixture(), "s1");
    assert_eq!(view.selected_session(), Some("s1"));
    view.apply(PickerAction::Fold, 5);
    assert_eq!(view.selected_line(), Some(3));
    assert_eq!(view.selected_session(), None);
    assert_eq!(
        view.lines()[3],
        Line::Header {
            key: None,
            text: "[+] (no team) (1): 1 idle".into(),
        }
    );

    let flat = vec![
        picker_entry("s1", "base", None, false, EntryState::Idle, false),
        picker_entry("s7", "solo", None, false, EntryState::Unknown, false),
    ];
    let mut flat_view = PickerView::new(flat, "s1");
    let flat_before = flat_view.lines();
    flat_view.apply(PickerAction::Fold, 5);
    assert_eq!(flat_view.selected_session(), Some("s1"));
    assert_eq!(flat_view.lines(), flat_before);

    let mut filtered = PickerView::new(picker_fixture(), "s1");
    filtered.apply(PickerAction::StartFilter, 5);
    filtered.apply(PickerAction::Char(b'b'), 5);
    filtered.apply(PickerAction::Char(b'e'), 5);
    filtered.apply(PickerAction::Home, 5);
    let filtered_before = filtered.lines();
    filtered.apply(PickerAction::Toggle, 5);
    filtered.apply(PickerAction::Fold, 5);
    filtered.apply(PickerAction::Unfold, 5);
    assert_eq!(filtered.lines(), filtered_before);
    filtered.apply(PickerAction::Move(1), 5);
    assert_eq!(filtered.selected_session(), Some("s4"));
    filtered.apply(PickerAction::Fold, 5);
    assert_eq!(filtered.selected_session(), Some("s4"));
    assert_eq!(filtered.lines(), filtered_before);
}

#[test]
fn picker_view_reconcile_empty_handles_clamping_vanished_header_empty_and_none() {
    let mut view = PickerView::new(picker_fixture(), "s1");
    assert_eq!(view.selected_line(), Some(4));
    let without_s1: Vec<_> = picker_fixture()
        .into_iter()
        .filter(|entry| entry.id != "s1")
        .collect();
    view.replace(without_s1);
    assert_eq!(view.selected_line(), Some(2));
    assert_eq!(view.selected_session(), None);

    let mut view = PickerView::new(picker_fixture(), "s1");
    view.apply(PickerAction::Home, 5);
    view.apply(PickerAction::Move(1), 5);
    view.apply(PickerAction::Move(1), 5);
    assert_eq!(view.selected_line(), Some(2));
    let without_s6: Vec<_> = picker_fixture()
        .into_iter()
        .filter(|entry| entry.id != "s6")
        .collect();
    view.replace(without_s6);
    assert_eq!(view.selected_line(), Some(2));
    assert_eq!(
        view.lines()[2],
        Line::Header {
            key: None,
            text: "[-] (no team) (1)".into(),
        }
    );

    view.replace(vec![]);
    assert_eq!(view.lines(), Vec::<Line>::new());
    assert_eq!(view.selected_line(), None);
    assert_eq!(view.selected_session(), None);

    view.replace(picker_fixture());
    assert_eq!(view.selected_line(), Some(0));
    assert_eq!(view.selected_session(), None);
}

#[test]
fn picker_view_header_text_covers_busy_unknown_exited_held_and_private_no_team() {
    let entries = vec![
        picker_entry("t1-w", "w", Some("t1"), false, EntryState::Working, false),
        picker_entry("t1-b1", "b1", Some("t1"), false, EntryState::Busy, false),
        picker_entry("t1-b2", "b2", Some("t1"), false, EntryState::Busy, false),
        picker_entry("t2-u1", "u1", Some("t2"), false, EntryState::Unknown, false),
        picker_entry("t2-u2", "u2", Some("t2"), false, EntryState::Unknown, false),
        picker_entry("t3-e1", "e1", Some("t3"), false, EntryState::Exited, true),
        picker_entry("solo", "solo", None, true, EntryState::Idle, false),
    ];
    let mut view = PickerView::new(entries, "t1-w");
    assert_eq!(
        view.lines(),
        vec![
            Line::Header {
                key: Some("t1".into()),
                text: "[-] t1 (3)".into(),
            },
            Line::Session { id: "t1-w".into() },
            Line::Session { id: "t1-b1".into() },
            Line::Session { id: "t1-b2".into() },
            Line::Header {
                key: Some("t2".into()),
                text: "[+] t2 (2)".into(),
            },
            Line::Header {
                key: Some("t3".into()),
                text: "[+] t3 (1): 1 exited".into(),
            },
            Line::Header {
                key: None,
                text: "[+] (no team) (1): 1 idle".into(),
            },
        ]
    );
    view.apply(PickerAction::Fold, 5);
    assert_eq!(
        view.lines()[0],
        Line::Header {
            key: Some("t1".into()),
            text: "[+] t1 (3): 1 working, 2 busy".into(),
        }
    );
}

#[test]
fn picker_view_toggle_and_unfold_on_session_leave_view_unchanged() {
    let mut toggle_view = PickerView::new(picker_fixture(), "s1");
    let before_toggle = toggle_view.lines();
    assert_eq!(toggle_view.apply(PickerAction::Toggle, 5), None);
    assert_eq!(toggle_view.lines(), before_toggle);
    assert_eq!(toggle_view.selected_line(), Some(4));
    assert_eq!(toggle_view.selected_session(), Some("s1"));

    let mut unfold_view = PickerView::new(picker_fixture(), "s1");
    let before_unfold = unfold_view.lines();
    assert_eq!(unfold_view.apply(PickerAction::Unfold, 5), None);
    assert_eq!(unfold_view.lines(), before_unfold);
    assert_eq!(unfold_view.selected_line(), Some(4));
    assert_eq!(unfold_view.selected_session(), Some("s1"));
}

fn refilter_empty_selection_with_lines() -> (PickerView, Vec<Line>) {
    let mut view = PickerView::new(picker_fixture(), "s1");
    let expected_lines = view.lines();
    assert_eq!(view.apply(PickerAction::StartFilter, 5), None);
    assert_eq!(view.apply(PickerAction::Char(b'z'), 5), None);
    assert_eq!(view.apply(PickerAction::Char(b'z'), 5), None);
    assert_eq!(view.lines(), Vec::<Line>::new());
    assert_eq!(view.selected_line(), None);
    assert_eq!(view.apply(PickerAction::StartFilter, 5), None);
    assert_eq!(view.lines(), expected_lines);
    assert_eq!(view.selected_line(), None);
    (view, expected_lines)
}

#[test]
fn picker_view_move_with_lines_and_no_selection_stays_unselected() {
    let (mut view, expected_lines) = refilter_empty_selection_with_lines();
    assert_eq!(view.apply(PickerAction::Move(1), 5), None);
    assert_eq!(view.lines(), expected_lines);
    assert_eq!(view.selected_line(), None);
    assert_eq!(view.selected_session(), None);
}

#[test]
fn picker_view_backspace_with_empty_query_and_no_selection_stays_unselected() {
    let (mut view, expected_lines) = refilter_empty_selection_with_lines();
    assert_eq!(view.apply(PickerAction::Backspace, 5), None);
    assert_eq!(view.lines(), expected_lines);
    assert_eq!(view.selected_line(), None);
    assert_eq!(view.selected_session(), None);
}

#[test]
fn parser_escape_while_three_bytes_pending_clears_buffer_instead_of_restarting() {
    let mut parser = PickerParser::default();
    assert_eq!(parser.feed(b"\x1b[Z", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"\x1b", false), Vec::<PickerAction>::new());
    assert_eq!(parser.feed(b"j", false), vec![PickerAction::Move(1)]);
}
