//! Seam 1: emulator + snapshot renderer, through the public `Emulator` interface.
//!
//! Round-trip rule: feed bytes to an emulator, render its snapshot, feed the
//! snapshot to a fresh emulator of the same size, and the two screens are equal.
//! A few tests also pin literal expected cells, so the round trip cannot pass
//! by both sides being equally wrong.

use a2amx::emulator::{Attrs, Color, Emulator, Size};

const SIZE: Size = Size { cols: 20, rows: 5 };

fn roundtrip(input: &[u8]) -> (Emulator, Emulator) {
    let mut original = Emulator::new(SIZE);
    original.feed(input);
    let mut restored = Emulator::new(SIZE);
    restored.feed(&original.snapshot());
    (original, restored)
}

#[test]
fn feed_places_text_and_moves_the_cursor() {
    let mut emu = Emulator::new(SIZE);
    emu.feed(b"hi\r\nyo");
    let screen = emu.screen();
    assert_eq!(screen.cell(0, 0).ch, 'h');
    assert_eq!(screen.cell(0, 1).ch, 'i');
    assert_eq!(screen.cell(1, 0).ch, 'y');
    assert_eq!(screen.cell(1, 1).ch, 'o');
    assert_eq!((screen.cursor.row, screen.cursor.col), (1, 2));
}

#[test]
fn feed_answers_terminal_queries_from_one_place() {
    let mut emu = Emulator::new(SIZE);
    // Device attributes, then a cursor position report after "hello".
    assert_eq!(emu.feed(b"\x1b[c"), b"\x1b[?6c");
    assert_eq!(emu.feed(b"hello\x1b[6n"), b"\x1b[1;6R");
    // Kitty keyboard query is unanswered: harnesses fall back to legacy keys.
    assert_eq!(emu.feed(b"\x1b[?u"), b"");
}

#[test]
fn snapshot_roundtrips_plain_text() {
    let (original, restored) = roundtrip(b"hello\r\nworld");
    assert_eq!(original.screen(), restored.screen());
}

#[test]
fn snapshot_roundtrips_colors_and_attributes() {
    let input = b"\x1b[1;31mred\x1b[0m \x1b[4;38;2;10;20;30mrgb\x1b[0m \x1b[7;48;5;200minv\x1b[0m";
    let (original, restored) = roundtrip(input);
    let screen = restored.screen();
    assert_eq!(screen.cell(0, 0).fg, Color::Indexed(1));
    assert!(screen.cell(0, 0).attrs.contains(Attrs::BOLD));
    assert_eq!(screen.cell(0, 4).fg, Color::Rgb(10, 20, 30));
    assert!(screen.cell(0, 4).attrs.contains(Attrs::UNDERLINE));
    assert_eq!(screen.cell(0, 8).bg, Color::Indexed(200));
    assert!(screen.cell(0, 8).attrs.contains(Attrs::INVERSE));
    assert_eq!(original.screen(), screen);
}

#[test]
fn snapshot_roundtrips_wide_characters_and_cursor_position() {
    let (original, restored) = roundtrip("a漢b\x1b[3;7H".as_bytes());
    let screen = restored.screen();
    assert_eq!(screen.cell(0, 1).ch, '漢');
    assert!(screen.cell(0, 1).attrs.contains(Attrs::WIDE));
    assert!(screen.cell(0, 2).attrs.contains(Attrs::WIDE_SPACER));
    assert_eq!(screen.cell(0, 3).ch, 'b');
    assert_eq!((screen.cursor.row, screen.cursor.col), (2, 6));
    assert_eq!(original.screen(), screen);
}

#[test]
fn snapshot_roundtrips_alt_screen_and_input_modes() {
    let (original, restored) = roundtrip(b"main\x1b[?1049h\x1b[?2004h\x1b[?1h\x1b[?1004halt");
    let screen = restored.screen();
    assert!(screen.modes.alt_screen);
    assert!(screen.modes.bracketed_paste);
    assert!(screen.modes.app_cursor);
    assert!(screen.modes.focus_reporting);
    assert_eq!(screen.cell(0, 0).ch, 'a');
    assert_eq!(original.screen(), screen);
}

#[test]
fn snapshot_does_not_replay_side_effects() {
    let mut emu = Emulator::new(SIZE);
    // OSC 52 clipboard write, OSC 8 hyperlink, OSC 0 title, then visible text.
    emu.feed(b"\x1b]52;c;aGVsbG8=\x07\x1b]8;;http://x.invalid\x07link\x1b]8;;\x07\x1b]0;t\x07");
    let snap = emu.snapshot();
    assert!(!snap.windows(6).any(|w| w == b"\x1b]52;c"));
    assert!(!snap.windows(4).any(|w| w == b"\x1b]8;"));
}
