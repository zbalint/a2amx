use a2amx::prefix::{Action, Command, PrefixMachine, parse_prefix};

fn normalized(actions: impl IntoIterator<Item = Action>) -> Vec<Action> {
    let mut out = Vec::new();
    for action in actions {
        match action {
            Action::Forward(bytes) if bytes.is_empty() => {}
            Action::Forward(bytes) => match out.last_mut() {
                Some(Action::Forward(previous)) => previous.extend(bytes),
                _ => out.push(Action::Forward(bytes)),
            },
            Action::Command(command) => out.push(Action::Command(command)),
        }
    }
    out
}

#[test]
fn parses_the_complete_accepted_prefix_set() {
    let accepted = [
        ("c-B", 0x02),
        ("c-space", 0x00),
        ("C-a", 0x01),
        ("C-Z", 0x1a),
        ("c-C", 0x03),
        ("C-\\", 0x1c),
        ("c-]", 0x1d),
        ("C-^", 0x1e),
        ("c-_", 0x1f),
    ];

    for (spec, expected) in accepted {
        assert_eq!(parse_prefix(spec).ok(), Some(expected), "{spec}");
    }
}

#[test]
fn rejects_non_prefix_controls_with_named_reasons() {
    let rejected = [
        ("C-i", "Tab"),
        ("c-I", "Tab"),
        ("C-j", "LF"),
        ("c-J", "LF"),
        ("C-m", "CR"),
        ("C-M", "CR"),
        ("C-[", "ESC"),
        ("c-[", "ESC"),
    ];
    for (spec, reason) in rejected {
        let error = parse_prefix(spec).err();
        assert!(error.is_some(), "{spec} was accepted");
        let message = error.map_or_else(String::new, |error| error.to_string());
        assert!(message.contains(reason), "{spec}: {message}");
    }

    for spec in ["", "C-@", "C-?", "C-1", "prefix", "C-space "] {
        assert!(parse_prefix(spec).is_err(), "{spec:?} was accepted");
    }
}

#[test]
fn recognizes_each_command() {
    let mut machine = PrefixMachine::new(0);
    assert_eq!(
        machine.feed(&[0, b'd']),
        vec![Action::Command(Command::Detach)]
    );
    assert_eq!(
        machine.feed(&[0, b'w']),
        vec![Action::Command(Command::SessionPicker)]
    );
    assert_eq!(
        machine.feed(&[0, b'[']),
        vec![Action::Command(Command::ScrollMode)]
    );
    assert_eq!(
        machine.feed(&[0, b'r']),
        vec![Action::Command(Command::Release)]
    );
}

#[test]
fn release_survives_split_reads_and_is_protected_inside_paste() {
    let mut machine = PrefixMachine::new(0);
    assert!(machine.feed(&[0]).is_empty());
    assert_eq!(machine.feed(b"r"), vec![Action::Command(Command::Release)]);

    let mut machine = PrefixMachine::new(0);
    let paste = b"\x1b[200~\0r\x1b[201~";
    assert_eq!(machine.feed(paste), vec![Action::Forward(paste.to_vec())]);
    assert_eq!(
        machine.feed(&[0, b'r']),
        vec![Action::Command(Command::Release)]
    );
}

#[test]
fn literal_prefix_and_unknown_key_have_their_distinct_effects() {
    let mut machine = PrefixMachine::new(0);
    assert_eq!(machine.feed(&[0, 0]), vec![Action::Forward(vec![0])]);
    assert_eq!(
        machine.feed(&[b'a', 0, b'x', b'b']),
        vec![Action::Forward(b"ab".to_vec())]
    );
}

#[test]
fn adjacent_forwarded_bytes_coalesce_without_delaying_them() {
    let mut machine = PrefixMachine::new(0);
    assert_eq!(machine.feed(b"abc"), vec![Action::Forward(b"abc".to_vec())]);
    assert_eq!(
        machine.feed(&[b'x', 0, b'q', b'y']),
        vec![Action::Forward(b"xy".to_vec())]
    );
}

#[test]
fn prefix_state_survives_read_boundaries() {
    let mut machine = PrefixMachine::new(0);
    assert!(machine.feed(&[0]).is_empty());
    assert_eq!(
        machine.feed(b"drest"),
        vec![
            Action::Command(Command::Detach),
            Action::Forward(b"rest".to_vec()),
        ]
    );
}

#[test]
fn bracketed_paste_forwards_prefix_and_command_bytes() {
    let mut machine = PrefixMachine::new(0);
    let input = b"\x1b[200~before\0d\0w\0[after\x1b[201~\0d";
    assert_eq!(
        machine.feed(input),
        vec![
            Action::Forward(input[..input.len() - 2].to_vec()),
            Action::Command(Command::Detach),
        ]
    );
}

#[test]
fn paste_markers_are_forwarded_immediately_and_split_safe_at_every_boundary() {
    let input = b"left\x1b[200~\0d\x1b[201~right";
    let expected = vec![Action::Forward(input.to_vec())];

    for boundary in 0..=input.len() {
        let mut machine = PrefixMachine::new(0);
        let mut got = Vec::new();
        got.extend(machine.feed(&input[..boundary]));
        got.extend(machine.feed(&input[boundary..]));
        assert_eq!(normalized(got), expected, "split at {boundary}");
    }
}

#[test]
fn unterminated_paste_remains_in_paste_mode_across_reads() {
    let mut machine = PrefixMachine::new(0);
    assert_eq!(
        machine.feed(b"\x1b[200~\0d"),
        vec![Action::Forward(b"\x1b[200~\0d".to_vec())]
    );
    assert_eq!(
        machine.feed(b"\0w\x1b[201~\0d"),
        vec![
            Action::Forward(b"\0w\x1b[201~".to_vec()),
            Action::Command(Command::Detach),
        ]
    );
}

#[test]
fn byte_at_a_time_and_single_read_have_equal_normalized_actions() {
    let input = b"hello\0x\0\0\x1b[200~\0d\0w\x1b[201~tail\0[done";
    let mut one_read = PrefixMachine::new(0);
    let expected = normalized(one_read.feed(input));

    let mut bytewise = PrefixMachine::new(0);
    let mut actions = Vec::new();
    for byte in input {
        actions.extend(bytewise.feed(std::slice::from_ref(byte)));
    }
    assert_eq!(normalized(actions), expected);
}

#[test]
fn status_line_command_survives_split_reads_and_flushes_input() {
    let mut machine = PrefixMachine::new(2);
    assert_eq!(
        machine.feed(&[2, b's']),
        vec![Action::Command(Command::StatusLine)]
    );
    assert!(machine.feed(&[2]).is_empty());
    assert_eq!(
        machine.feed(b"s"),
        vec![Action::Command(Command::StatusLine)]
    );
    assert_eq!(
        machine.feed(&[b'x', 2, b's']),
        vec![
            Action::Forward(b"x".to_vec()),
            Action::Command(Command::StatusLine)
        ]
    );
}
