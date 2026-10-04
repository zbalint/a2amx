//! Seam 1: incremental rendering, damage tracking, and terminal viewport behavior.

use a2amx::emulator::{Attrs, Emulator, Modes, MouseMode, Screen, Scroll, Size};

const SIZE: Size = Size { cols: 20, rows: 5 };

fn assert_same_screen(source: &Emulator, mirror: &Emulator) {
    let expected = source.screen();
    let actual = mirror.screen();
    assert_eq!(expected.size, actual.size);
    assert_eq!(expected.cells, actual.cells);
    assert_eq!(expected.cursor, actual.cursor);
    let mut expected_modes = expected.modes;
    let mut actual_modes = actual.modes;
    expected_modes.alt_screen = false;
    actual_modes.alt_screen = false;
    assert_eq!(expected_modes, actual_modes);
}

fn contains_escape(bytes: &[u8], escape: &[u8]) -> bool {
    bytes.windows(escape.len()).any(|window| window == escape)
}
fn row_text(screen: &Screen, row: u16) -> String {
    (0..screen.size.cols)
        .map(|col| screen.cell(row, col).ch)
        .collect()
}

#[test]
fn full_render_never_switches_the_client_alternate_screen() {
    let mut emulator = Emulator::new(SIZE);
    emulator.feed(b"main\x1b[?1049halt");

    let render = emulator.render_full();

    assert!(!contains_escape(&render, b"\x1b[?1049h"));
    assert!(!contains_escape(&render, b"\x1b[?1049l"));
}

#[test]
fn full_render_preserves_the_last_column_without_erasing_it() {
    let mut emulator = Emulator::new(Size { cols: 10, rows: 3 });
    emulator.feed(b"abcdefghij");

    let render = emulator.render_full();

    assert!(
        contains_escape(&render, b"\x1b[1;1H\x1b[0mabcdefghij\x1b[0m\x1b[2;1H"),
        "rendered bytes: {:?}",
        String::from_utf8_lossy(&render)
    );
}

#[test]
fn full_render_keeps_clearing_after_a_short_row() {
    let mut emulator = Emulator::new(Size { cols: 10, rows: 3 });
    emulator.feed(b"abcdefghi");

    let render = emulator.render_full();

    assert!(
        contains_escape(&render, b"\x1b[1;1H\x1b[0mabcdefghi\x1b[0m\x1b[K"),
        "rendered bytes: {:?}",
        String::from_utf8_lossy(&render)
    );
}

#[test]
fn full_render_preserves_the_last_column_after_a_wide_glyph() {
    let mut emulator = Emulator::new(Size { cols: 10, rows: 3 });
    emulator.feed("abcdefgh漢".as_bytes());

    let render = emulator.render_full();

    assert!(
        contains_escape(
            &render,
            "\x1b[1;1H\x1b[0mabcdefgh漢\x1b[0m\x1b[2;1H".as_bytes()
        ),
        "rendered bytes: {:?}",
        String::from_utf8_lossy(&render)
    );
}

#[test]
fn incremental_render_mirrors_content_cursor_modes_and_buffer_switches() {
    let mut source = Emulator::new(SIZE);
    let mut mirror = Emulator::new(SIZE);

    mirror.feed(&source.render_full());
    assert_same_screen(&source, &mirror);

    for bytes in [
        b"hello".as_slice(),
        b"\x1b[31mred\x1b[0m".as_slice(),
        b"\x1b[3;7H".as_slice(),
        b"\x1b[2K".as_slice(),
        b"\x1b[2;4r\x1b[4;1Hscroll\r\n".as_slice(),
        b"\x1b[?1049halt\x1b[?2004h".as_slice(),
        b"\x1b[?1049l".as_slice(),
    ] {
        source.feed(bytes);
        mirror.feed(&source.render_update());
        assert_same_screen(&source, &mirror);
    }
}

#[test]
fn resize_requires_a_new_size_full_render() {
    let mut source = Emulator::new(SIZE);
    source.feed(b"before resize");
    source.render_full();
    let resized = Size { cols: 12, rows: 3 };
    source.resize(resized);

    let mut mirror = Emulator::new(resized);
    mirror.feed(&source.render_full());

    assert_same_screen(&source, &mirror);
}

#[test]
fn no_change_and_scrolled_viewport_pause_incremental_updates() {
    let mut emulator = Emulator::new(SIZE);
    let mut mirror = Emulator::new(SIZE);
    emulator.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
    mirror.feed(&emulator.render_full());
    assert!(emulator.render_update().is_empty());

    emulator.scroll(Scroll::PageUp);
    assert!(emulator.is_scrolled());
    emulator.feed(b"\r\nfour");
    assert!(emulator.render_update().is_empty());

    emulator.scroll(Scroll::Bottom);
    assert!(!emulator.is_scrolled());
    mirror.feed(&emulator.render_update());
    assert_same_screen(&emulator, &mirror);
}

#[test]
fn page_up_reveals_literal_history_and_bottom_restores_live_rows() {
    let mut emulator = Emulator::new(SIZE);
    emulator.feed(
        b"row-00\r\nrow-01\r\nrow-02\r\nrow-03\r\nrow-04\r\nrow-05\r\nrow-06\r\nrow-07\r\nrow-08\r\nrow-09\r\nrow-10\r\nrow-11\r\nrow-12\r\nrow-13\r\nrow-14\r\nrow-15\r\nrow-16\r\nrow-17\r\nrow-18\r\nrow-19\r\nrow-20\r\nrow-21\r\nrow-22\r\nrow-23\r\nrow-24\r\nrow-25\r\nrow-26\r\nrow-27\r\nrow-28\r\nrow-29",
    );
    emulator.render_full();

    emulator.scroll(Scroll::PageUp);
    let history = emulator.screen();
    assert!(emulator.is_scrolled());
    assert!(!history.cursor.visible);
    let expected_history = [
        "row-20              ",
        "row-21              ",
        "row-22              ",
        "row-23              ",
        "row-24              ",
    ];
    for (row, expected) in expected_history.iter().enumerate() {
        assert_eq!(row_text(&history, row as u16), *expected);
    }

    emulator.scroll(Scroll::Bottom);
    let live = emulator.screen();
    assert!(!emulator.is_scrolled());
    let expected_live = [
        "row-25              ",
        "row-26              ",
        "row-27              ",
        "row-28              ",
        "row-29              ",
    ];
    for (row, expected) in expected_live.iter().enumerate() {
        assert_eq!(row_text(&live, row as u16), *expected);
    }
}

#[test]
fn scroll_variants_obey_history_and_live_boundaries() {
    let mut emulator = Emulator::new(SIZE);
    emulator.feed(b"row-00\r\nrow-01\r\nrow-02\r\nrow-03\r\nrow-04\r\nrow-05\r\nrow-06\r\nrow-07\r\nrow-08\r\nrow-09");
    emulator.render_full();

    for scroll in [Scroll::LineDown, Scroll::PageDown, Scroll::Bottom] {
        emulator.scroll(scroll);
        assert!(!emulator.is_scrolled());
        assert!(emulator.render_update().is_empty());
    }

    emulator.scroll(Scroll::Top);
    assert!(emulator.is_scrolled());
    emulator.render_full();
    for scroll in [Scroll::Top, Scroll::LineUp, Scroll::PageUp] {
        emulator.scroll(scroll);
        assert!(emulator.is_scrolled());
        assert!(emulator.render_update().is_empty());
    }

    emulator.scroll(Scroll::LineDown);
    assert!(emulator.is_scrolled());
    assert!(emulator.render_update().is_empty());
    emulator.scroll(Scroll::PageDown);
    assert!(!emulator.is_scrolled());
    assert!(!emulator.render_update().is_empty());
    emulator.scroll(Scroll::Bottom);
    assert!(emulator.render_update().is_empty());
}

#[test]
fn cursor_and_mode_only_transitions_are_rendered_without_screen_damage() {
    let mut source = Emulator::new(SIZE);
    let mut mirror = Emulator::new(SIZE);
    mirror.feed(&source.render_full());

    source.feed(b"\x1b[?25l\x1b[?2004h\x1b[?1h\x1b=\x1b[?1004h\x1b[?1002h\x1b[?1006h");
    let update = source.render_update();
    assert!(contains_escape(&update, b"\x1b[?25l"));
    assert!(contains_escape(&update, b"\x1b[?2004h"));
    assert!(contains_escape(&update, b"\x1b[?1006h"));
    assert!(!contains_escape(&update, b"\x1b[?1049"));
    mirror.feed(&update);
    assert_same_screen(&source, &mirror);
    assert_eq!(
        source.screen().modes,
        Modes {
            alt_screen: false,
            bracketed_paste: true,
            app_cursor: true,
            app_keypad: true,
            focus_reporting: true,
            mouse: MouseMode::Drag,
            mouse_sgr: true,
        }
    );
}

#[test]
fn mouse_mode_transitions_cover_click_motion_and_off() {
    let mut source = Emulator::new(SIZE);
    let mut mirror = Emulator::new(SIZE);
    mirror.feed(&source.render_full());

    for (bytes, expected, marker) in [
        (
            b"\x1b[?1000h".as_slice(),
            MouseMode::Click,
            b"\x1b[?1000h".as_slice(),
        ),
        (
            b"\x1b[?1003h".as_slice(),
            MouseMode::Motion,
            b"\x1b[?1003h".as_slice(),
        ),
        (
            b"\x1b[?1003l".as_slice(),
            MouseMode::Off,
            b"\x1b[?1003l".as_slice(),
        ),
    ] {
        source.feed(bytes);
        let update = source.render_update();
        assert!(contains_escape(&update, marker));
        mirror.feed(&update);
        assert_same_screen(&source, &mirror);
        assert_eq!(source.screen().modes.mouse, expected);
    }
}

#[test]
fn exact_cursor_cell_damage_survives_incremental_rendering() {
    let mut source = Emulator::new(SIZE);
    let mut mirror = Emulator::new(SIZE);
    mirror.feed(&source.render_full());

    source.feed(b"A");
    source.feed(b"\x1b[D");
    let write_update = source.render_update();
    mirror.feed(&write_update);
    assert_same_screen(&source, &mirror);

    source.feed(b"\x1b[1X");
    let erase_update = source.render_update();
    mirror.feed(&erase_update);
    assert_same_screen(&source, &mirror);

    source.feed(b"A");
    mirror.feed(&source.render_full());
    source.feed("\u{301}".as_bytes());
    let combining_update = source.render_update();
    assert!(contains_escape(&combining_update, "\u{301}".as_bytes()));
    mirror.feed(&combining_update);
    assert_same_screen(&source, &mirror);
}

#[test]
fn snapshot_does_not_consume_damage() {
    let mut emulator = Emulator::new(SIZE);
    emulator.render_full();
    emulator.feed(b"changed");
    let _snapshot = emulator.snapshot();
    assert!(!emulator.render_update().is_empty());
}

#[test]
fn render_preserves_wide_and_combining_cells_and_drops_side_effects() {
    let mut source = Emulator::new(SIZE);
    source.feed("A\u{301}漢B".as_bytes());
    source.feed(b"\x1b]52;c;aGVsbG8=\x07\x1b]8;;https://example.invalid\x07");
    let full = source.render_full();

    assert!(contains_escape(&full, "\u{301}".as_bytes()));
    assert!(!contains_escape(&full, b"\x1b]52;"));
    assert!(!contains_escape(&full, b"\x1b]8;"));

    let mut mirror = Emulator::new(SIZE);
    mirror.feed(&full);
    assert_same_screen(&source, &mirror);
    assert!(source.screen().cell(0, 1).attrs.contains(Attrs::WIDE));
    assert!(
        source
            .screen()
            .cell(0, 2)
            .attrs
            .contains(Attrs::WIDE_SPACER)
    );
}

#[test]
fn every_underline_variant_maps_to_the_public_underline_bit() {
    let mut source = Emulator::new(SIZE);
    source.feed(b"\x1b[4mA\x1b[0m\x1b[4:2mB\x1b[0m\x1b[4:3mC\x1b[0m\x1b[4:4mD\x1b[0m\x1b[4:5mE");
    let screen = source.screen();
    for col in 0..5 {
        assert!(screen.cell(0, col).attrs.contains(Attrs::UNDERLINE));
    }

    let mut mirror = Emulator::new(SIZE);
    mirror.feed(&source.render_full());
    assert_same_screen(&source, &mirror);
}

#[test]
fn unsupported_query_and_title_side_effects_are_ignored() {
    let mut emulator = Emulator::new(SIZE);
    assert_eq!(emulator.feed(b"\x1b]10;?\x07\x1b]11;?\x07"), b"");
    assert_eq!(emulator.feed(b"\x1b[14t\x1b[16t\x1b]52;c;?\x07"), b"");
    assert_eq!(emulator.feed(b"\x1b[18t"), b"\x1b[8;5;20t");
    assert_eq!(emulator.feed(b"\x1b]0;hidden title\x07\x07"), b"");
    assert_eq!(emulator.screen().cell(0, 0).ch, ' ');
}

#[test]
fn combining_updates_include_wide_bases_and_the_right_margin() {
    for input in ["漢", "\x1b[1;19H漢"] {
        let mut source = Emulator::new(SIZE);
        let mut mirror = Emulator::new(SIZE);
        source.feed(input.as_bytes());
        mirror.feed(&source.render_full());
        source.feed("\u{301}".as_bytes());
        let update = source.render_update();
        assert!(contains_escape(&update, "\u{301}".as_bytes()));
        mirror.feed(&update);
        assert_same_screen(&source, &mirror);
    }
}
