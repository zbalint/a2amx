//! The A2AMX-owned terminal emulator interface.
//!
//! `alacritty_terminal` (pinned) sits behind this module and nothing outside it
//! may name an alacritty type, so the emulator can be replaced. The module owns
//! the screen model, the replies to terminal queries, and the snapshot serializer
//! that redraws a client from the model (docs/architecture.md, "Terminal behavior").

/// Terminal dimensions in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Cell attribute bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attrs(pub u16);

impl Attrs {
    pub const BOLD: u16 = 1 << 0;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const INVERSE: u16 = 1 << 4;
    pub const HIDDEN: u16 = 1 << 5;
    pub const STRIKEOUT: u16 = 1 << 6;
    /// First cell of a double-width character.
    pub const WIDE: u16 = 1 << 7;
    /// Trailing cell occupied by the right half of a double-width character.
    pub const WIDE_SPACER: u16 = 1 << 8;

    pub fn contains(self, bit: u16) -> bool {
        self.0 & bit == bit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// Input-affecting terminal modes a reattaching client must be put back into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    pub alt_screen: bool,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub focus_reporting: bool,
    pub mouse_reporting: bool,
}

/// The visible screen of the active buffer, in row-major order (`rows * cols` cells).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub size: Size,
    pub cells: Vec<Cell>,
    pub cursor: Cursor,
    pub modes: Modes,
}

impl Screen {
    pub fn cell(&self, row: u16, col: u16) -> &Cell {
        &self.cells[usize::from(row) * usize::from(self.size.cols) + usize::from(col)]
    }
}

pub struct Emulator {
    size: Size,
}

impl Emulator {
    /// Emulator with `TERM=xterm-256color` semantics, kitty keyboard protocol off,
    /// and bounded scrollback.
    pub fn new(size: Size) -> Self {
        todo!()
    }

    /// Apply PTY output to the screen model. Returns the bytes the emulator answers
    /// to terminal queries (for example DA, DSR); the caller writes them to the PTY.
    /// This is the only place query replies are produced.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        todo!()
    }

    pub fn resize(&mut self, size: Size) {
        todo!()
    }

    pub fn screen(&self) -> Screen {
        todo!()
    }

    /// Escape sequences that redraw a blank terminal of this emulator's size into
    /// the current screen: contents, attributes, cursor, and input modes. Feeding
    /// the result to a fresh emulator of the same size reproduces `screen()`.
    /// Graphics, hyperlinks, and clipboard writes are deliberately not emitted.
    pub fn snapshot(&self) -> Vec<u8> {
        todo!()
    }
}
