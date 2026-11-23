use super::*;

fn agy_screen_from(bytes: &[u8]) -> a2amx::emulator::Screen {
    agy_screen_from_size(bytes, 40)
}

fn agy_screen_from_size(bytes: &[u8], rows: u16) -> a2amx::emulator::Screen {
    let mut emulator = Emulator::new(Size { cols: 120, rows });
    emulator.feed(bytes);
    emulator.screen()
}

fn agy_composer_with_rows(
    top: u16,
    body: &[&str],
    cursor_col: u16,
    rows: &[(u16, &str)],
) -> Vec<u8> {
    let mut bytes = agy_composer_bytes(top, body, cursor_col, true);
    for (row, text) in rows {
        bytes.extend_from_slice(format!("\x1b[{};1H{text}", row + 1).as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "\x1b[{};{}H\x1b[?25h",
            top + 1 + body.len() as u16,
            cursor_col + 1
        )
        .as_bytes(),
    );
    bytes
}

fn agy_composer_bytes(top: u16, body: &[&str], cursor_col: u16, cursor_visible: bool) -> Vec<u8> {
    let rules = "─".repeat(120);
    let mut bytes = format!("\x1b[2J\x1b[?2004h\x1b[{};1H{rules}", top + 1).into_bytes();
    for (offset, row) in body.iter().enumerate() {
        let row = if offset == 0 {
            format!("> {row}")
        } else {
            format!("  {row}")
        };
        bytes.extend_from_slice(format!("\x1b[{};1H{row}", top + 2 + offset as u16).as_bytes());
    }
    let bottom = top + 1 + body.len() as u16;
    bytes.extend_from_slice(format!("\x1b[{};1H{rules}", bottom + 1).as_bytes());
    bytes.extend_from_slice(
        format!(
            "\x1b[{};{}H\x1b[?25{}",
            top + 1 + body.len() as u16,
            cursor_col + 1,
            if cursor_visible { 'h' } else { 'l' }
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn agy_readiness_accepts_composer_shapes_and_pasted_content() {
    for top in [8, 24, 36] {
        let bytes = agy_composer_bytes(top, &[""], 2, true);
        assert!(harness::ready(
            Harness::Agy,
            &agy_screen_from(&bytes),
            false
        ));
    }

    for placeholder in [
        "Auto mode: edits & safe commands auto-approved (shift+tab to cycle)",
        "Plan mode: research & plan only (shift+tab to cycle)",
    ] {
        let bytes = agy_composer_bytes(8, &[placeholder], 2, true);
        assert!(harness::ready(
            Harness::Agy,
            &agy_screen_from(&bytes),
            false
        ));
    }

    let cases = [
        vec!["one line draft"],
        vec![
            "line one",
            "line two",
            "line three",
            "line four",
            "line five",
            "line six",
        ],
        vec!["[Pasted text #1 +40 lines]"],
        vec![
            "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">",
            "From another agent, not your user: send_message(...)",
            "",
            "A long pasted operator sentence that remains visible in the composer.",
            "</a2amx-message>",
        ],
        vec!["/clear"],
        vec![">"],
    ];
    for body in cases {
        let bytes = agy_composer_bytes(8, &body, 2, true);
        assert!(harness::ready(
            Harness::Agy,
            &agy_screen_from(&bytes),
            false
        ));
    }

    let opening_line = format!("{}  {}", "a".repeat(113), "b".repeat(47));
    let sentence_line = format!(
        "{} {} {}",
        "c".repeat(110),
        "d".repeat(115),
        "e".repeat(102)
    );
    assert_eq!(opening_line.len(), 162);
    assert_eq!(sentence_line.len(), 329);
    let envelope_rows = [
        opening_line[..113].to_owned(),
        opening_line[115..].to_owned(),
        sentence_line[..110].to_owned(),
        sentence_line[111..226].to_owned(),
        sentence_line[227..].to_owned(),
        "close".to_owned(),
        "short".to_owned(),
        "done".to_owned(),
    ];
    assert_eq!(
        envelope_rows.iter().map(String::len).collect::<Vec<_>>(),
        [113, 47, 110, 115, 102, 5, 5, 4]
    );
    let envelope_refs = envelope_rows.iter().map(String::as_str).collect::<Vec<_>>();
    let envelope = agy_composer_bytes(8, &envelope_refs, 6, true);
    assert!(harness::ready(
        Harness::Agy,
        &agy_screen_from(&envelope),
        false
    ));

    let clear = agy_composer_with_rows(8, &["/clear"], 2, &[(11, "1. confirm")]);
    let clear_screen = agy_screen_from(&clear);
    assert!(
        clear_screen
            .text_lines()
            .iter()
            .any(|line| line.contains("1. confirm"))
    );
    assert!(harness::ready(Harness::Agy, &clear_screen, false));

    let working = agy_composer_with_rows(2, &["draft"], 7, &[(0, "⣷  Working...")]);
    let working_screen = agy_screen_from(&working);
    assert!(
        working_screen
            .text_lines()
            .iter()
            .any(|line| line.contains("Working..."))
    );
    assert!(harness::ready(Harness::Agy, &working_screen, false));

    let queued = agy_composer_with_rows(
        3,
        &[""],
        2,
        &[
            (0, "⣷  Working..."),
            (1, "▸ queued while working (alt+m to send immediately)"),
        ],
    );
    let queued_screen = agy_screen_from(&queued);
    assert!(
        queued_screen
            .text_lines()
            .iter()
            .any(|line| line.contains("alt+m to send immediately"))
    );
    assert!(harness::ready(Harness::Agy, &queued_screen, false));
}

#[test]
fn agy_readiness_rejects_cards_dialogs_blanks_and_malformed_composers() {
    let rules = "─".repeat(120);
    let card = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1HMCP\x1b[4;1H> 1. Yes, allow tool call\x1b[4;3H\x1b[?25h"
    );
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(card.as_bytes()),
        false
    ));

    let shell_card =
        b"\x1b[2J\x1b[?2004h\x1b[3;1HCommand\x1b[4;1H> 1. Yes, run command\x1b[4;3H\x1b[?25h";
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(shell_card),
        false
    ));

    let dialog = b"\x1b[2J\x1b[?2004h\x1b[3;1HDo you trust the contents of this project?\x1b[4;1H> Yes, I trust this folder\x1b[4;3H\x1b[?25h";
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(dialog),
        false
    ));

    let missing_top = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H> 1. Yes, allow tool call\x1b[3;1H{rules}\x1b[2;3H\x1b[?25h"
    );
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(missing_top.as_bytes()),
        false
    ));

    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(b"\x1b[2J\x1b[?2004h\x1b[?25h"),
        false
    ));

    let hidden = agy_composer_bytes(8, &[""], 2, false);
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(&hidden),
        false
    ));

    let paste_off = agy_composer_bytes(8, &[""], 2, true)
        .into_iter()
        .chain(b"\x1b[?2004l".iter().copied())
        .collect::<Vec<_>>();
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(&paste_off),
        false
    ));
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(&agy_composer_bytes(8, &[""], 2, true)),
        true
    ));

    let echo_rules = "─".repeat(60);
    let echo_box = format!(
        "\x1b[2J\x1b[?2004h\x1b[5;1H{echo_rules}\x1b[6;1H> \x1b[7;1H{echo_rules}\x1b[6;3H\x1b[?25h"
    );
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(echo_box.as_bytes()),
        false
    ));

    let bad_continuation = format!(
        "\x1b[2J\x1b[?2004h\x1b[9;1H{rules}\x1b[10;1H> first\x1b[11;1H second\x1b[12;1H{rules}\x1b[10;3H\x1b[?25h"
    );
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(bad_continuation.as_bytes()),
        false
    ));
    let rules = "─".repeat(120);
    let too_far = format!(
        "\x1b[2J\x1b[?2004h\x1b[1;1H{rules}\x1b[2;1H> \x1b[47;1H{rules}\x1b[24;3H\x1b[?25h"
    );
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from_size(too_far.as_bytes(), 50),
        false
    ));

    let mut no_space = agy_composer_bytes(8, &[""], 2, true);
    let marker = no_space
        .windows(2)
        .position(|window| window == b"> ")
        .expect("composer marker");
    no_space[marker + 1] = b'x';
    assert!(!harness::ready(
        Harness::Agy,
        &agy_screen_from(&no_space),
        false
    ));
}

#[test]
fn agy_composer_idle_only_accepts_empty_known_modes() {
    for placeholder in [
        "",
        "Auto mode: edits & safe commands auto-approved (shift+tab to cycle)",
        "Plan mode: research & plan only (shift+tab to cycle)",
    ] {
        let bytes = agy_composer_bytes(8, &[placeholder], 2, true);
        assert!(harness::agy_composer_idle(&agy_screen_from(&bytes), false));
    }

    for (body, col) in [
        (vec!["draft"], 7),
        (vec!["one line draft"], 2),
        (
            vec![
                "line one",
                "line two",
                "line three",
                "line four",
                "line five",
                "line six",
            ],
            2,
        ),
        (vec!["[Pasted text #1 +40 lines]"], 2),
        (
            vec!["Auto mode: edits & safe commands auto-approved (shift+tab to cycle)x"],
            2,
        ),
        (
            vec!["Auto mode: edits & safe commands auto-approved (shift+tab to cycle)"],
            72,
        ),
    ] {
        let bytes = agy_composer_bytes(8, &body, col, true);
        assert!(!harness::agy_composer_idle(&agy_screen_from(&bytes), false));
    }
    let card = b"\x1b[2J\x1b[?2004h\x1b[3;1HMCP\x1b[4;1H> 1. Yes\x1b[4;3H\x1b[?25h";
    assert!(!harness::agy_composer_idle(&agy_screen_from(card), false));
    assert!(!harness::agy_composer_idle(
        &agy_screen_from(b"\x1b[2J\x1b[?2004h\x1b[?25h"),
        false
    ));
    let paste_off = agy_composer_bytes(8, &[""], 2, true)
        .into_iter()
        .chain(b"\x1b[?2004l".iter().copied())
        .collect::<Vec<_>>();
    assert!(!harness::agy_composer_idle(
        &agy_screen_from(&paste_off),
        false
    ));
}

#[test]
fn agy_turn_in_flight_detects_spinner_and_queued_rows_above_composer() {
    // 1. Spinner row directly above the rule.
    let spinner = agy_composer_with_rows(3, &[""], 2, &[(2, "⣷  Working...")]);
    assert!(harness::agy_turn_in_flight(&agy_screen_from(&spinner)));

    // 2. Queued row directly above the rule, and two queued rows with spinner above them.
    let queued =
        agy_composer_with_rows(3, &[""], 2, &[(2, "▸ queued (alt+m to send immediately)")]);
    assert!(harness::agy_turn_in_flight(&agy_screen_from(&queued)));

    let queued_stack = agy_composer_with_rows(
        3,
        &[""],
        2,
        &[
            (0, "⣷  Working..."),
            (1, "▸ first (alt+m to send immediately)"),
            (2, "▸ second (alt+m to send immediately)"),
        ],
    );
    assert!(harness::agy_turn_in_flight(&agy_screen_from(&queued_stack)));

    // 3. Blank row, transcript text row, or spinner two rows above with blank row between.
    let blank = agy_composer_with_rows(3, &[""], 2, &[]);
    assert!(!harness::agy_turn_in_flight(&agy_screen_from(&blank)));

    let transcript = agy_composer_with_rows(3, &[""], 2, &[(2, "  some text")]);
    assert!(!harness::agy_turn_in_flight(&agy_screen_from(&transcript)));

    let spinner_too_high = agy_composer_with_rows(3, &[""], 2, &[(1, "⣷  Working...")]);
    assert!(!harness::agy_turn_in_flight(&agy_screen_from(
        &spinner_too_high
    )));

    // 4. Non-matching prefixes and Braille block edges.
    for row in ["⣷x", "*  Working...", "▸queued"] {
        let bytes = agy_composer_with_rows(3, &[""], 2, &[(2, row)]);
        assert!(
            !harness::agy_turn_in_flight(&agy_screen_from(&bytes)),
            "row {row:?} should not match"
        );
    }
    for (ch, expected) in [
        ('\u{2800}', true),
        ('\u{28ff}', true),
        ('\u{27ff}', false),
        ('\u{2900}', false),
    ] {
        let row = format!("{ch} edge");
        let bytes = agy_composer_with_rows(3, &[""], 2, &[(2, &row)]);
        assert_eq!(
            harness::agy_turn_in_flight(&agy_screen_from(&bytes)),
            expected,
            "block edge U+{:04X}",
            ch as u32
        );
    }

    // 5. No composer (cursor hidden) and composer top rule on row 0.
    let mut no_composer = agy_composer_bytes(3, &[""], 2, false);
    no_composer.extend_from_slice("\x1b[3;1H⣷  Working...\x1b[5;3H".as_bytes());
    assert!(!harness::agy_turn_in_flight(&agy_screen_from(&no_composer)));

    let top_row_zero = agy_composer_with_rows(0, &[""], 2, &[]);
    assert!(!harness::agy_turn_in_flight(&agy_screen_from(
        &top_row_zero
    )));
}

#[test]
fn agy_composer_idle_rejects_each_cursor_mismatch_and_non_blank_placeholder_tail() {
    let two_row_body = agy_composer_bytes(8, &["", ""], 2, true);
    assert!(!harness::agy_composer_idle(
        &agy_screen_from(&two_row_body),
        false
    ));

    let rules = "─".repeat(120);
    let cursor_on_bottom_rule = format!(
        "\x1b[2J\x1b[?2004h\x1b[9;1H{rules}\x1b[10;1H> \x1b[11;1H{rules}\x1b[11;3H\x1b[?25h"
    );
    assert!(!harness::agy_composer_idle(
        &agy_screen_from(cursor_on_bottom_rule.as_bytes()),
        false
    ));

    let cursor_col_three = agy_composer_bytes(8, &[""], 3, true);
    assert!(!harness::agy_composer_idle(
        &agy_screen_from(&cursor_col_three),
        false
    ));

    for placeholder in [
        "Auto mode: edits & safe commands auto-approved (shift+tab to cycle)",
        "Plan mode: research & plan only (shift+tab to cycle)",
    ] {
        let with_tail = format!("{placeholder}!");
        let bytes = agy_composer_bytes(8, &[&with_tail], 2, true);
        assert!(
            !harness::agy_composer_idle(&agy_screen_from(&bytes), false),
            "placeholder {placeholder}"
        );
    }
}
