use a2amx::picker::{PickerAction, PickerParser, scroll_into_view, target};

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
