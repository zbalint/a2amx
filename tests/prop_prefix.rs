use a2amx::prefix::{Action, Command, PrefixMachine, parse_prefix};
use proptest::prelude::*;
use std::sync::LazyLock;

static PREFIXES: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let mut specs = vec!["c-space".to_owned()];
    specs.extend(('a'..='z').map(|letter| format!("c-{letter}")));
    specs.extend(["c-\\", "c-]", "c-^", "c-_"].map(str::to_owned));
    specs
        .into_iter()
        .filter_map(|spec| parse_prefix(&spec).ok())
        .collect()
});

fn prefixes() -> impl Strategy<Value = u8> {
    prop::sample::select(PREFIXES.clone())
}

fn biased_input(prefix: u8) -> impl Strategy<Value = Vec<u8>> {
    let alphabet = prop_oneof![
        2 => any::<u8>(),
        3 => Just(prefix),
        1 => Just(0x1b),
        1 => Just(b'['),
        1 => Just(b'2'),
        1 => Just(b'0'),
        1 => Just(b'1'),
        1 => Just(b'~'),
        1 => Just(b'd'),
        1 => Just(b'w'),
        1 => Just(b's'),
        1 => Just(b'r'),
    ];
    let random = prop::collection::vec(alphabet, 0..=64);
    let spliced = (
        prop::collection::vec(any::<u8>(), 0..=12),
        prop::collection::vec(any::<u8>(), 0..=12),
        prop::collection::vec(any::<u8>(), 0..=12),
        prop::sample::select(vec![b'd', b'w', b'[', b's', b'r']),
    )
        .prop_map(move |(before, middle, after, command)| {
            let mut input = before;
            input.extend_from_slice(b"\x1b[200~");
            input.extend_from_slice(&middle);
            input.extend_from_slice(b"\x1b[201~");
            input.extend_from_slice(&[prefix, command]);
            input.extend_from_slice(&after);
            input
        });
    prop_oneof![3 => random, 2 => spliced]
}

fn non_escape_bytes() -> impl Strategy<Value = u8> {
    any::<u8>().prop_map(|byte| if byte == 0x1b { 0 } else { byte })
}

fn biased_paste_body(prefix: u8) -> impl Strategy<Value = Vec<u8>> {
    let random = prop::collection::vec(non_escape_bytes(), 0..=64);
    let spliced = (
        prop::collection::vec(non_escape_bytes(), 0..=20),
        prop::sample::select(vec![b'd', b'w', b'[', b's', b'r']),
        prop::collection::vec(non_escape_bytes(), 0..=20),
    )
        .prop_map(move |(before, command, after)| {
            let mut body = before;
            body.extend_from_slice(&[prefix, command]);
            body.extend_from_slice(&after);
            body
        });
    prop_oneof![3 => random, 2 => spliced]
}

fn split_cases() -> impl Strategy<Value = (u8, Vec<u8>, Vec<u16>)> {
    prefixes().prop_flat_map(|prefix| {
        (
            Just(prefix),
            biased_input(prefix),
            prop::collection::vec(any::<u16>(), 0..=32),
        )
    })
}

fn paste_cases() -> impl Strategy<Value = (u8, Vec<u8>)> {
    prefixes().prop_flat_map(|prefix| (Just(prefix), biased_paste_body(prefix)))
}
fn partial_paste_cases() -> impl Strategy<Value = (u8, usize, Vec<u8>)> {
    prefixes().prop_flat_map(|prefix| (Just(prefix), 1usize..=5, biased_paste_body(prefix)))
}

fn normalized(actions: impl IntoIterator<Item = Action>) -> Vec<Action> {
    let mut output = Vec::new();
    for action in actions {
        match action {
            Action::Forward(bytes) if bytes.is_empty() => {}
            Action::Forward(bytes) => match output.last_mut() {
                Some(Action::Forward(previous)) => previous.extend(bytes),
                _ => output.push(Action::Forward(bytes)),
            },
            Action::Command(command) => output.push(Action::Command(command)),
        }
    }
    output
}

fn feed_in_chunks(prefix: u8, input: &[u8], cuts: &[u16]) -> Vec<Action> {
    let mut boundaries = cuts
        .iter()
        .map(|cut| usize::from(*cut) % (input.len() + 1))
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    let mut machine = PrefixMachine::new(prefix);
    let mut output = Vec::new();
    let mut start = 0;
    for boundary in boundaries.into_iter().chain(std::iter::once(input.len())) {
        output.extend(machine.feed(&input[start..boundary]));
        start = boundary;
    }
    output
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn any_read_split_gives_the_same_actions(case in split_cases()) {
        let (prefix, input, cuts) = case;
        let split = normalized(feed_in_chunks(prefix, &input, &cuts));
        let mut whole_machine = PrefixMachine::new(prefix);
        let whole = normalized(whole_machine.feed(&input));
        prop_assert_eq!(split, whole);
    }

    #[test]
    fn input_without_the_prefix_is_forwarded_unchanged(
        prefix in prefixes(),
        input in prop::collection::vec(any::<u8>(), 0..=64),
    ) {
        prop_assume!(!input.contains(&prefix));
        let mut machine = PrefixMachine::new(prefix);
        let actions = normalized(machine.feed(&input));
        if input.is_empty() {
            prop_assert!(actions.is_empty());
        } else {
            prop_assert_eq!(actions, vec![Action::Forward(input)]);
        }
    }

    #[test]
    fn prefix_twice_is_one_literal_prefix(
        prefix in prefixes(),
        a in any::<u8>(),
        b in any::<u8>(),
    ) {
        prop_assume!(a != prefix && b != prefix);
        let mut machine = PrefixMachine::new(prefix);
        prop_assert_eq!(
            normalized(machine.feed(&[a, prefix, prefix, b])),
            vec![Action::Forward(vec![a, prefix, b])]
        );
    }

    #[test]
    fn a_bracketed_paste_is_never_interpreted(case in paste_cases()) {
        let (prefix, body) = case;
        let mut input = b"\x1b[200~".to_vec();
        input.extend_from_slice(&body);
        input.extend_from_slice(b"\x1b[201~");
        let mut machine = PrefixMachine::new(prefix);
        prop_assert_eq!(normalized(machine.feed(&input)), vec![Action::Forward(input)]);
    }
    #[test]
    fn a_partial_paste_marker_does_not_hide_the_real_one(
        case in partial_paste_cases()
    ) {
        let (prefix, partial_length, body) = case;
        let marker = b"\x1b[200~";
        let mut input = marker[..partial_length].to_vec();
        input.extend_from_slice(marker);
        input.extend_from_slice(&body);
        input.extend_from_slice(b"\x1b[201~");
        let mut machine = PrefixMachine::new(prefix);
        prop_assert_eq!(normalized(machine.feed(&input)), vec![Action::Forward(input)]);
    }

}

#[test]
fn command_table_is_exact_for_each_leading_forward_length() {
    for leading_len in 0..=8 {
        let leading = vec![b'x'; leading_len];
        for (key, command) in [
            (b'd', Command::Detach),
            (b'w', Command::SessionPicker),
            (b'[', Command::ScrollMode),
            (b's', Command::StatusLine),
            (b'r', Command::Release),
        ] {
            let mut input = leading.clone();
            input.extend_from_slice(&[0x02, key]);
            let mut machine = PrefixMachine::new(0x02);
            let expected = if leading.is_empty() {
                vec![Action::Command(command)]
            } else {
                vec![Action::Forward(leading.clone()), Action::Command(command)]
            };
            assert_eq!(normalized(machine.feed(&input)), expected);
        }
    }
}
