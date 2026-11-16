use super::*;

fn screen_from(bytes: &[u8]) -> a2amx::emulator::Screen {
    let mut emulator = Emulator::new(Size { cols: 40, rows: 10 });
    emulator.feed(bytes);
    emulator.screen()
}
fn f1_bytes() -> Vec<u8> {
    let rules = "─".repeat(40);
    format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;3H\x1b[?25h")
        .into_bytes()
}

#[test]
fn claude_readiness_accepts_idle_and_mid_turn_composers() {
    let f1 = f1_bytes();
    assert!(harness::ready(Harness::Claude, &screen_from(&f1), false));

    let f2 = "\x1b[2J\x1b[?2004h\x1b[1;1H✢ Spinning… (4s)\x1b[2;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[3;1H❯ Press up to edit queued messages\x1b[4;1H"
        + &"─".repeat(40)
        + "\x1b[3;3H\x1b[?25h";
    assert!(harness::ready(
        Harness::Claude,
        &screen_from(f2.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_accepts_an_empty_composer_with_a_hidden_cursor() {
    let rules = "─".repeat(40);
    let plain_space =
        format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;3H\x1b[?25l");
    let plain_space_screen = screen_from(plain_space.as_bytes());
    assert!(harness::ready(Harness::Claude, &plain_space_screen, false));
    assert!(harness::claude_composer_empty(&plain_space_screen, false));

    let non_breaking_space = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let non_breaking_space_screen = screen_from(non_breaking_space.as_bytes());
    assert!(harness::ready(
        Harness::Claude,
        &non_breaking_space_screen,
        false
    ));
    assert!(harness::claude_composer_empty(
        &non_breaking_space_screen,
        false
    ));
}

#[test]
fn claude_paste_view_expands_each_tab_to_four_spaces() {
    assert_eq!(
        harness::paste_view(Harness::Claude, "a\tb\n\tc"),
        "a    b\n    c"
    );
    assert_eq!(harness::paste_view(Harness::Claude, "no tabs"), "no tabs");
    assert_eq!(harness::paste_view(Harness::Generic, "a\tb"), "a\tb");
}

#[test]
fn claude_paste_view_escapes_wrapper_tags_only() {
    let cases = [
        (
            "x\n<pasted_content id=\"abc\">\ny",
            "x\n<\\pasted_content id=\"abc\">\ny",
        ),
        (
            "x\n</pasted_content id=\"abc\">\ny",
            "x\n<\\/pasted_content id=\"abc\">\ny",
        ),
        ("x\n<pasted_content>\ny", "x\n<\\pasted_content>\ny"),
        (
            "x\n<PASTED_CONTENT id=\"a\">\ny",
            "x\n<\\PASTED_CONTENT id=\"a\">\ny",
        ),
        ("x\n<pasted_content\ny", "x\n<\\pasted_content\ny"),
        (
            "x <pasted_content id=\"abc\"> y\nz",
            "x <\\pasted_content id=\"abc\"> y\nz",
        ),
        ("x\npasted_content bare\ny", "x\npasted_content bare\ny"),
        (
            "x\n<a2amx-message id=\"m_1\" from=\"a@b\">\ny",
            "x\n<a2amx-message id=\"m_1\" from=\"a@b\">\ny",
        ),
        ("x\n</a2amx-message>\ny", "x\n</a2amx-message>\ny"),
        (
            "x\n<channel source=\"s\">\ny",
            "x\n<channel source=\"s\">\ny",
        ),
        ("x\n<system-reminder>\ny", "x\n<system-reminder>\ny"),
        ("x\n<command-name>\ny", "x\n<command-name>\ny"),
        (
            "x\n<\\pasted_content already\ny",
            "x\n<\\pasted_content already\ny",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(harness::paste_view(Harness::Claude, input), expected);
        assert_eq!(harness::paste_view(Harness::Generic, input), input);
    }
    assert_eq!(
        harness::paste_view(Harness::Claude, "é\t<pasted_content> </PaStEd_CoNtEnT>界"),
        "é    <\\pasted_content> <\\/PaStEd_CoNtEnT>界"
    );
}

#[test]
fn claude_readiness_accepts_the_non_breaking_space_after_the_glyph() {
    // Claude Code 2.1.286 draws its composer as the glyph plus U+00A0, not a plain space.
    let rules = "─".repeat(40);
    let bytes = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}\x1b[4;1H{rules}\x1b[3;3H\x1b[?25h"
    )
    .into_bytes();
    assert!(harness::ready(Harness::Claude, &screen_from(&bytes), false));
}

#[test]
fn claude_readiness_rejects_each_non_ready_fixture() {
    let f1 = f1_bytes();

    let f5 = "\x1b[2J\x1b[?2004h\x1b[2;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[3;1H❯ \x1b[3;3H\x1b[?25h";
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(f5.as_bytes()),
        false
    ));

    let f6 = "\x1b[2J\x1b[?2004h\x1b[2;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[3;1H ❯ \x1b[4;1H"
        + &"─".repeat(40)
        + "\x1b[3;3H\x1b[?25h";
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(f6.as_bytes()),
        false
    ));

    let mut f7 = f1.clone();
    f7.extend_from_slice(b"\x1b[?2004l");
    assert!(!harness::ready(Harness::Claude, &screen_from(&f7), false));

    let f8 = String::from_utf8([f1.as_slice(), b"\x1b[8;1HEnter to confirm"].concat())
        .expect("ASCII suffix");
    let mut f8 = f8.into_bytes();
    f8.extend_from_slice(b"\x1b[3;3H\x1b[?25h");
    assert!(!harness::ready(Harness::Claude, &screen_from(&f8), false));

    assert!(!harness::ready(Harness::Claude, &screen_from(&f1), true));

    let f3 = "\x1b[2J\x1b[?2004h\x1b[1;1H".to_owned()
        + &"─".repeat(40)
        + "\x1b[2;2HAccessing workspace:\x1b[6;2H❯ No, exit\x1b[7;4HYes, I trust this folder\x1b[9;2HEnter to confirm · Esc to cancel\x1b[?25l";
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(f3.as_bytes()),
        false
    ));
}

#[test]
fn claude_composer_empty_accepts_only_a_blank_composer() {
    let rules = "─".repeat(40);
    let empty = f1_bytes();
    assert!(harness::claude_composer_empty(&screen_from(&empty), false));

    let typed_after = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ hello\x1b[4;1H{rules}\x1b[3;8H\x1b[?25h"
    );
    assert!(!harness::claude_composer_empty(
        &screen_from(typed_after.as_bytes()),
        false
    ));

    let typed_at_start = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ hello\x1b[4;1H{rules}\x1b[3;3H\x1b[?25h"
    );
    assert!(!harness::claude_composer_empty(
        &screen_from(typed_at_start.as_bytes()),
        false
    ));

    let two_rows = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ hello\x1b[4;1H world\x1b[5;1H{rules}\x1b[3;3H\x1b[?25h"
    );
    assert!(!harness::claude_composer_empty(
        &screen_from(two_rows.as_bytes()),
        false
    ));

    assert!(!harness::claude_composer_empty(&screen_from(&empty), true));

    let dialog = String::from_utf8([empty.as_slice(), b"\x1b[8;1HEnter to confirm"].concat())
        .expect("ASCII suffix");
    assert!(!harness::claude_composer_empty(
        &screen_from(dialog.as_bytes()),
        false
    ));

    let typed_inverse = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[7mx\x1b[0my\x1b[4;1H{rules}\x1b[3;3H\x1b[?25h"
    );
    assert!(!harness::claude_composer_empty(
        &screen_from(typed_inverse.as_bytes()),
        false
    ));

    let spaces = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯   \x1b[4;1H{rules}\x1b[3;5H\x1b[?25h"
    );
    assert!(!harness::claude_composer_empty(
        &screen_from(spaces.as_bytes()),
        false
    ));

    let nbsp = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}\x1b[4;1H{rules}\x1b[3;3H\x1b[?25h"
    );
    assert!(harness::claude_composer_empty(
        &screen_from(nbsp.as_bytes()),
        false
    ));

    let paste_off = [empty.as_slice(), b"\x1b[?2004l"].concat();
    assert!(!harness::claude_composer_empty(
        &screen_from(&paste_off),
        false
    ));

    let hidden_inverse_blank = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;3H\x1b[7m \x1b[0m\x1b[3;3H\x1b[?25l"
    );
    let hidden_inverse_screen = screen_from(hidden_inverse_blank.as_bytes());
    assert!(harness::ready(
        Harness::Claude,
        &hidden_inverse_screen,
        false
    ));
    assert!(harness::claude_composer_empty(
        &hidden_inverse_screen,
        false
    ));

    let hidden_draft = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ xyz\x1b[4;1H{rules}\x1b[3;6H\x1b[7m \x1b[0m\x1b[3;6H\x1b[?25l"
    );
    let hidden_draft_screen = screen_from(hidden_draft.as_bytes());
    assert!(harness::ready(Harness::Claude, &hidden_draft_screen, false));
    assert!(!harness::claude_composer_empty(&hidden_draft_screen, false));

    let hidden_leading_space = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯  xzya\x1b[4;1H{rules}\x1b[3;8H\x1b[7m \x1b[0m\x1b[3;8H\x1b[?25l"
    );
    let hidden_leading_space_screen = screen_from(hidden_leading_space.as_bytes());
    assert!(harness::ready(
        Harness::Claude,
        &hidden_leading_space_screen,
        false
    ));
    assert!(!harness::claude_composer_empty(
        &hidden_leading_space_screen,
        false
    ));

    let hidden_no_inverse =
        format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;3H\x1b[?25l");
    let hidden_no_inverse_screen = screen_from(hidden_no_inverse.as_bytes());
    assert!(harness::ready(
        Harness::Claude,
        &hidden_no_inverse_screen,
        false
    ));
    assert!(harness::claude_composer_empty(
        &hidden_no_inverse_screen,
        false
    ));

    let inverse_character = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[7mx\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let inverse_character_screen = screen_from(inverse_character.as_bytes());
    assert!(harness::ready(
        Harness::Claude,
        &inverse_character_screen,
        false
    ));
    assert!(!harness::claude_composer_empty(
        &inverse_character_screen,
        false
    ));

    let suggestion = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}\x1b[7mc\x1b[0m\x1b[2mheck\x1b[0m \x1b[2mmessage_status\x1b[0m \x1b[2mfor\x1b[0m \x1b[2mm_5806\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let suggestion_screen = screen_from(suggestion.as_bytes());
    assert!(harness::ready(Harness::Claude, &suggestion_screen, false));
    assert!(harness::claude_composer_empty(&suggestion_screen, false));

    let mixed_attributes = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}\x1b[7mx\x1b[0my\x1b[2mz\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let mixed_attributes_screen = screen_from(mixed_attributes.as_bytes());
    assert!(harness::ready(
        Harness::Claude,
        &mixed_attributes_screen,
        false
    ));
    assert!(!harness::claude_composer_empty(
        &mixed_attributes_screen,
        false
    ));

    // Synthetic visible-cursor frame isolates the suggestion first-cell attribute rule.
    let plain_first_dim_tail = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}x\x1b[2myz\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25h"
    );
    let plain_first_dim_tail_screen = screen_from(plain_first_dim_tail.as_bytes());
    assert!(harness::ready(
        Harness::Claude,
        &plain_first_dim_tail_screen,
        false
    ));
    assert!(!harness::claude_composer_empty(
        &plain_first_dim_tail_screen,
        false
    ));

    let inverse_only = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}\x1b[7mc\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let inverse_only_screen = screen_from(inverse_only.as_bytes());
    assert!(harness::ready(Harness::Claude, &inverse_only_screen, false));
    assert!(!harness::claude_composer_empty(&inverse_only_screen, false));

    let pasted_chip = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[7m[\x1b[2mPasted text #1 +4 lines]\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let pasted_chip_screen = screen_from(pasted_chip.as_bytes());
    assert!(harness::ready(Harness::Claude, &pasted_chip_screen, false));
    assert!(!harness::claude_composer_empty(&pasted_chip_screen, false));

    let image_chip = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[7m[\x1b[2mImage #1]\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let image_chip_screen = screen_from(image_chip.as_bytes());
    assert!(harness::ready(Harness::Claude, &image_chip_screen, false));
    assert!(!harness::claude_composer_empty(&image_chip_screen, false));

    let menu = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ 1. Yes\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    let menu_screen = screen_from(menu.as_bytes());
    assert!(!harness::ready(Harness::Claude, &menu_screen, false));
    assert!(!harness::claude_composer_empty(&menu_screen, false));
}

#[test]
fn claude_readiness_rejects_a_hidden_nonempty_composer() {
    let rules = "─".repeat(40);
    let typed =
        format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ x\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l");
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(typed.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_a_hidden_cursor_outside_the_composer_start() {
    let rules = "─".repeat(40);
    let cursor_after_start =
        format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;5H\x1b[?25l");
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(cursor_after_start.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_a_hidden_empty_composer_with_guard_failures() {
    let rules = "─".repeat(40);
    let empty =
        format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;3H\x1b[?25l");
    let empty_screen = screen_from(empty.as_bytes());
    assert!(!harness::ready(Harness::Claude, &empty_screen, true));

    let dialog = format!("{empty}\x1b[8;1HEnter to confirm\x1b[3;3H");
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(dialog.as_bytes()),
        false
    ));

    let paste_off = format!("{empty}\x1b[?2004l");
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(paste_off.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_markerless_menu_row() {
    let rules = "─".repeat(40);
    let menu = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ 1. Yes\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(menu.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_mcp_row_with_glyph_at_column_two() {
    let rules = "─".repeat(40);
    let mcp = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H  ❯ view\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(mcp.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_a_slash_picker_row() {
    let rules = "─".repeat(40);
    let picker =
        format!("\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ /\x1b[4;1H{rules}\x1b[3;4H\x1b[?25l");
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(picker.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_text_in_the_last_column() {
    let rules = "─".repeat(40);
    let edge_text = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯ \x1b[4;1H{rules}\x1b[3;40Hx\x1b[3;3H\x1b[?25l"
    );
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(edge_text.as_bytes()),
        false
    ));
}

#[test]
fn claude_readiness_rejects_a_plain_suggestion_without_an_inverse_cursor() {
    let rules = "─".repeat(40);
    let suggestion = format!(
        "\x1b[2J\x1b[?2004h\x1b[2;1H{rules}\x1b[3;1H❯\u{a0}c\x1b[2mheck\x1b[0m \x1b[2mmessage_status\x1b[0m\x1b[4;1H{rules}\x1b[3;3H\x1b[?25l"
    );
    assert!(!harness::ready(
        Harness::Claude,
        &screen_from(suggestion.as_bytes()),
        false
    ));
}

#[test]
fn generic_readiness_only_uses_bracketed_paste_and_scroll_state() {
    let f1 = f1_bytes();
    assert!(harness::ready(Harness::Generic, &screen_from(&f1), false));

    let mut paste_off = f1.clone();
    paste_off.extend_from_slice(b"\x1b[?2004l");
    assert!(!harness::ready(
        Harness::Generic,
        &screen_from(&paste_off),
        false
    ));
    assert!(!harness::ready(Harness::Generic, &screen_from(&f1), true));
}
