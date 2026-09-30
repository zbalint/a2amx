//! The A2AMX-owned terminal emulator interface.
//!
//! `alacritty_terminal` (pinned) sits behind this module and nothing outside it
//! may name an alacritty type, so the emulator can be replaced. The module owns
//! the screen model, the replies to terminal queries, and the snapshot serializer
//! that redraws a client from the model (docs/architecture.md, "Terminal behavior").

use std::sync::{Arc, Mutex};

use alacritty_terminal::term::cell::{Cell as AlacrittyCell, Flags};
use alacritty_terminal::vte::ansi::{Color as AlacrittyColor, NamedColor};
use alacritty_terminal::{event, grid, index, term, vte};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    Off,
    Click,
    Drag,
    Motion,
}

/// Input-affecting terminal modes a reattaching client must be put back into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    pub alt_screen: bool,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub focus_reporting: bool,
    pub mouse: MouseMode,
    pub mouse_sgr: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Top,
    Bottom,
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

#[derive(Clone, Default)]
struct ReplyListener {
    replies: Arc<Mutex<Vec<u8>>>,
}

impl ReplyListener {
    fn clear(&self) {
        let mut replies = self
            .replies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        replies.clear();
    }

    fn take(&self) -> Vec<u8> {
        let mut replies = self
            .replies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *replies)
    }
}

impl event::EventListener for ReplyListener {
    fn send_event(&self, event: event::Event) {
        // shortcut: color/pixel-size queries and clipboard loads have no reply; bell, title,
        // clipboard, hyperlink, and other non-screen effects are deliberately dropped.
        if let event::Event::PtyWrite(reply) = event {
            let mut replies = self
                .replies
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            replies.extend_from_slice(reply.as_bytes());
        }
    }
}

#[derive(Clone, Copy)]
struct TermSize {
    cols: usize,
    rows: usize,
}

impl grid::Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

impl From<Size> for TermSize {
    fn from(size: Size) -> Self {
        Self {
            cols: usize::from(size.cols),
            rows: usize::from(size.rows),
        }
    }
}

pub struct Emulator {
    size: Size,
    term: term::Term<ReplyListener>,
    parser: vte::ansi::Processor,
    listener: ReplyListener,
    last_modes: Modes,
    last_cursor: Cursor,
    last_raw_cursor: index::Point,
    last_raw_cursor_cell: AlacrittyCell,
    last_preceding_cells: [Option<AlacrittyCell>; 2],
    rendered: bool,
}

impl Emulator {
    /// Emulator with `TERM=xterm-256color` semantics, kitty keyboard protocol off,
    /// and bounded scrollback.
    pub fn new(size: Size) -> Self {
        let dimensions = TermSize::from(size);
        let config = term::Config {
            kitty_keyboard: false,
            scrolling_history: 10000,
            ..term::Config::default()
        };
        let listener = ReplyListener::default();
        let terminal = term::Term::new(config, &dimensions, listener.clone());

        let default_cursor: index::Point = index::Point::default();
        Self {
            size,
            term: terminal,
            parser: vte::ansi::Processor::new(),
            listener,
            last_modes: Modes::default(),
            last_cursor: Cursor {
                row: 0,
                col: 0,
                visible: true,
            },
            last_raw_cursor: default_cursor,
            last_raw_cursor_cell: AlacrittyCell::default(),
            last_preceding_cells: [None, None],
            rendered: false,
        }
    }

    /// Apply PTY output to the screen model. Returns the bytes the emulator answers
    /// to terminal queries (for example DA, DSR); the caller writes them to the PTY.
    /// This is the only place query replies are produced.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.listener.clear();
        self.parser.advance(&mut self.term, bytes);
        self.listener.take()
    }

    pub fn resize(&mut self, size: Size) {
        self.term.resize(TermSize::from(size));
        self.size = size;
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn screen(&self) -> Screen {
        let cols = usize::from(self.size.cols);
        let rows = usize::from(self.size.rows);
        let mut cells = Vec::with_capacity(cols.saturating_mul(rows));
        for row in 0..rows {
            for col in 0..cols {
                cells.push(public_cell(self.cell_at(row, col)));
            }
        }

        Screen {
            size: self.size,
            cells,
            cursor: self.current_cursor(),
            modes: self.current_modes(),
        }
    }

    /// Moves the displayed viewport through the active buffer's scrollback history.
    pub fn scroll(&mut self, scroll: Scroll) {
        let scroll = match scroll {
            Scroll::LineUp => grid::Scroll::Delta(1),
            Scroll::LineDown => grid::Scroll::Delta(-1),
            Scroll::PageUp => grid::Scroll::PageUp,
            Scroll::PageDown => grid::Scroll::PageDown,
            Scroll::Top => grid::Scroll::Top,
            Scroll::Bottom => grid::Scroll::Bottom,
        };
        self.term.scroll_display(scroll);
    }

    pub fn is_scrolled(&self) -> bool {
        self.term.grid().display_offset() != 0
    }

    /// Escape sequences that redraw a blank terminal of this emulator's size into
    /// the current screen: contents, attributes, cursor, and input modes. Feeding
    /// the result to a fresh emulator of the same size reproduces `screen()`.
    /// Graphics, hyperlinks, and clipboard writes are deliberately not emitted.
    pub fn snapshot(&self) -> Vec<u8> {
        self.render_screen(true)
    }

    /// Render the complete screen into the client's already-active outer alternate screen.
    pub fn render_full(&mut self) -> Vec<u8> {
        let render = self.render_screen(false);
        // Advance alacritty's cursor-damage baseline before clearing it.
        let _ = self.term.damage();
        self.term.reset_damage();
        self.record_render_state(self.current_modes(), self.current_cursor());
        render
    }

    /// Render only the lines and state changed since the previous render.
    pub fn render_update(&mut self) -> Vec<u8> {
        if !self.rendered {
            return self.render_full();
        }

        // A historical viewport is intentionally static while the client is reading it.
        // Damage remains pending so that returning to the live bottom can be resynchronized.
        if self.is_scrolled() {
            return Vec::new();
        }

        let current_modes = self.current_modes();
        let current_cursor = self.current_cursor();
        let damaged_lines = self.collect_damage(current_cursor);
        let cursor_changed = current_cursor != self.last_cursor;
        let modes_changed = current_modes != self.last_modes;
        if damaged_lines.is_empty() && !cursor_changed && !modes_changed {
            return Vec::new();
        }

        let has_damaged_lines = !damaged_lines.is_empty();

        let mut render = Vec::new();
        for row in damaged_lines {
            self.render_line(&mut render, row);
        }
        if has_damaged_lines || cursor_changed || modes_changed {
            append_cursor(&mut render, current_cursor);
        }
        append_mode_changes(&mut render, self.last_modes, current_modes, false);

        self.record_render_state(current_modes, current_cursor);
        render
    }
    fn record_render_state(&mut self, modes: Modes, cursor: Cursor) {
        let raw_cursor = self.term.grid().cursor.point;
        let raw_cursor_cell = self.term.grid()[raw_cursor].clone();
        self.last_modes = modes;
        self.last_cursor = cursor;
        self.last_raw_cursor = raw_cursor;
        self.last_raw_cursor_cell = raw_cursor_cell;
        self.last_preceding_cells = std::array::from_fn(|index| {
            raw_cursor.column.0.checked_sub(index + 1).map(|col| {
                self.term.grid()[index::Point::new(raw_cursor.line, index::Column(col))].clone()
            })
        });
        self.rendered = true;
    }

    fn cell_at(&self, row: usize, col: usize) -> &AlacrittyCell {
        let point = term::viewport_to_point(
            self.term.grid().display_offset(),
            index::Point::new(row, index::Column(col)),
        );
        &self.term.grid()[point]
    }

    fn current_modes(&self) -> Modes {
        let mode = *self.term.mode();
        // shortcut: mouse modes are collapsed to the four public MouseMode states.
        let mouse = if mode.contains(term::TermMode::MOUSE_MOTION) {
            MouseMode::Motion
        } else if mode.contains(term::TermMode::MOUSE_DRAG) {
            MouseMode::Drag
        } else if mode.contains(term::TermMode::MOUSE_REPORT_CLICK) {
            MouseMode::Click
        } else {
            MouseMode::Off
        };

        Modes {
            alt_screen: mode.contains(term::TermMode::ALT_SCREEN),
            bracketed_paste: mode.contains(term::TermMode::BRACKETED_PASTE),
            app_cursor: mode.contains(term::TermMode::APP_CURSOR),
            app_keypad: mode.contains(term::TermMode::APP_KEYPAD),
            focus_reporting: mode.contains(term::TermMode::FOCUS_IN_OUT),
            mouse,
            mouse_sgr: mode.contains(term::TermMode::SGR_MOUSE),
        }
    }

    fn current_cursor(&self) -> Cursor {
        let mut point = self.term.grid().cursor.point;
        if self.term.grid()[point]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER)
        {
            point.column = index::Column(point.column.0.saturating_sub(1));
        }

        let offset = self.term.grid().display_offset();
        let viewport_point = term::point_to_viewport(offset, point);
        let (row, col) = match viewport_point {
            Some(point)
                if point.line < usize::from(self.size.rows)
                    && point.column.0 < usize::from(self.size.cols) =>
            {
                (point.line as u16, point.column.0 as u16)
            }
            _ => (0, 0),
        };

        Cursor {
            row,
            col,
            // shortcut: cursor style and blink are intentionally not forwarded.
            visible: offset == 0 && self.term.mode().contains(term::TermMode::SHOW_CURSOR),
        }
    }

    fn collect_damage(&mut self, current_cursor: Cursor) -> Vec<usize> {
        let raw_cursor = self.term.grid().cursor.point;
        let cursor_changed = current_cursor != self.last_cursor;
        // Term adds synthetic cursor damage but appends combining marks to a base
        // up to two columns left without damage. Cache only this bounded neighborhood.
        let raw_cell_unchanged = raw_cursor == self.last_raw_cursor
            && self.term.grid()[raw_cursor] == self.last_raw_cursor_cell
            && self.last_preceding_cells.iter().enumerate().all(
                |(index, previous)| match raw_cursor.column.0.checked_sub(index + 1) {
                    Some(col) => {
                        previous.as_ref()
                            == Some(
                                &self.term.grid()
                                    [index::Point::new(raw_cursor.line, index::Column(col))],
                            )
                    }
                    None => previous.is_none(),
                },
            );
        let lines: Vec<usize> = match self.term.damage() {
            term::TermDamage::Full => (0..usize::from(self.size.rows)).collect(),
            term::TermDamage::Partial(bounds) => bounds
                .filter_map(|bound| {
                    // Term::damage always includes the cursor cell. Ignore that synthetic
                    // point when neither the cursor nor the underlying line changed.
                    if !cursor_changed
                        && raw_cell_unchanged
                        && bound.line == raw_cursor.line.0.max(0) as usize
                        && bound.left == raw_cursor.column.0
                        && bound.right == raw_cursor.column.0
                    {
                        return None;
                    }
                    (bound.line < usize::from(self.size.rows)).then_some(bound.line)
                })
                .collect(),
        };
        self.term.reset_damage();
        lines
    }

    fn render_screen(&self, include_alt_screen: bool) -> Vec<u8> {
        let mut render = Vec::new();
        let modes = self.current_modes();
        let cursor = self.current_cursor();

        if include_alt_screen && modes.alt_screen {
            render.extend_from_slice(b"\x1b[?1049h");
        }
        render.extend_from_slice(b"\x1b[2J\x1b[H");
        for row in 0..usize::from(self.size.rows) {
            self.render_line(&mut render, row);
        }
        append_cursor(&mut render, cursor);
        append_mode_changes(&mut render, Modes::default(), modes, true);
        render
    }

    fn render_line(&self, render: &mut Vec<u8>, row: usize) {
        append_cursor_position(render, row, 0);
        render.extend_from_slice(b"\x1b[0m");

        let cols = usize::from(self.size.cols);
        let mut last = None;
        for col in 0..cols {
            let cell = self.cell_at(row, col);
            if is_skipped_cell(cell) || is_default_blank(cell) {
                continue;
            }
            last = Some(col);
        }

        let Some(last) = last else {
            render.extend_from_slice(b"\x1b[0m\x1b[K");
            return;
        };

        let mut output_col = 0usize;
        let mut style = CellStyle::default();
        for col in 0..=last {
            let cell = self.cell_at(row, col);
            if is_skipped_cell(cell) {
                // shortcut: soft-wrap flags (WRAPLINE) are ignored (no reflow on the client);
                // leading-wide spacer cells are likewise not reflowed.
                output_col = col.saturating_add(1);
                continue;
            }
            if output_col != col {
                append_cursor_position(render, row, col);
            }
            let next_style = CellStyle::from_cell(cell);
            if next_style != style {
                append_cell_style(render, next_style);
                style = next_style;
            }
            append_cell_glyph(render, cell);
            output_col = if cell.flags.contains(Flags::WIDE_CHAR) {
                col + 2
            } else {
                col + 1
            };
        }
        render.extend_from_slice(b"\x1b[0m\x1b[K");
    }
}

/// The session module uses this adapter without naming alacritty's emulator types.
pub(crate) fn window_size(size: Size) -> event::WindowSize {
    event::WindowSize {
        num_lines: size.rows,
        num_cols: size.cols,
        cell_width: 0,
        cell_height: 0,
    }
}

fn public_cell(cell: &AlacrittyCell) -> Cell {
    let mut attrs = 0;
    if cell.flags.contains(Flags::BOLD) {
        attrs |= Attrs::BOLD;
    }
    if cell.flags.contains(Flags::DIM) {
        attrs |= Attrs::DIM;
    }
    if cell.flags.contains(Flags::ITALIC) {
        attrs |= Attrs::ITALIC;
    }
    if cell.flags.intersects(Flags::ALL_UNDERLINES) {
        attrs |= Attrs::UNDERLINE;
    }
    if cell.flags.contains(Flags::INVERSE) {
        attrs |= Attrs::INVERSE;
    }
    if cell.flags.contains(Flags::HIDDEN) {
        attrs |= Attrs::HIDDEN;
    }
    if cell.flags.contains(Flags::STRIKEOUT) {
        attrs |= Attrs::STRIKEOUT;
    }
    if cell.flags.contains(Flags::WIDE_CHAR) {
        attrs |= Attrs::WIDE;
    }
    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        attrs |= Attrs::WIDE_SPACER;
    }

    Cell {
        ch: cell.c,
        fg: public_color(cell.fg),
        bg: public_color(cell.bg),
        attrs: Attrs(attrs),
    }
}

fn public_color(color: AlacrittyColor) -> Color {
    match color {
        AlacrittyColor::Named(NamedColor::Foreground | NamedColor::Background) => Color::Default,
        AlacrittyColor::Named(named) if (named as usize) < 256 => Color::Indexed(named as u8),
        AlacrittyColor::Named(_) => Color::Default,
        AlacrittyColor::Indexed(index) => Color::Indexed(index),
        AlacrittyColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn is_skipped_cell(cell: &AlacrittyCell) -> bool {
    cell.flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

fn is_default_blank(cell: &AlacrittyCell) -> bool {
    if cell.c != ' ' || !is_default_color(cell.fg) || !is_default_color(cell.bg) {
        return false;
    }
    let visible_flags = cell.flags.difference(Flags::WRAPLINE);
    visible_flags.is_empty() && cell.zerowidth().is_none_or(|chars| chars.is_empty())
}

fn is_default_color(color: AlacrittyColor) -> bool {
    matches!(
        color,
        AlacrittyColor::Named(NamedColor::Foreground | NamedColor::Background)
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    attrs: Flags,
    fg: AlacrittyColor,
    bg: AlacrittyColor,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            attrs: Flags::empty(),
            fg: AlacrittyColor::Named(NamedColor::Foreground),
            bg: AlacrittyColor::Named(NamedColor::Background),
        }
    }
}

impl CellStyle {
    fn from_cell(cell: &AlacrittyCell) -> Self {
        Self {
            attrs: cell.flags.intersection(
                Flags::BOLD
                    | Flags::DIM
                    | Flags::ITALIC
                    | Flags::ALL_UNDERLINES
                    | Flags::INVERSE
                    | Flags::HIDDEN
                    | Flags::STRIKEOUT,
            ),
            fg: cell.fg,
            bg: cell.bg,
        }
    }
}

fn append_cell_glyph(render: &mut Vec<u8>, cell: &AlacrittyCell) {
    let mut encoded = [0; 4];
    render.extend_from_slice(cell.c.encode_utf8(&mut encoded).as_bytes());
    if let Some(zerowidth) = cell.zerowidth() {
        for character in zerowidth {
            render.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
    }
}

fn append_cell_style(render: &mut Vec<u8>, style: CellStyle) {
    render.extend_from_slice(b"\x1b[0m");
    let mut params = [0usize; 20];
    let mut count = 0;
    let flags = style.attrs;
    if flags.contains(Flags::BOLD) {
        push_param(&mut params, &mut count, 1);
    }
    if flags.contains(Flags::DIM) {
        push_param(&mut params, &mut count, 2);
    }
    if flags.contains(Flags::ITALIC) {
        push_param(&mut params, &mut count, 3);
    }
    if flags.intersects(Flags::ALL_UNDERLINES) {
        push_param(&mut params, &mut count, 4);
    }
    if flags.contains(Flags::INVERSE) {
        push_param(&mut params, &mut count, 7);
    }
    if flags.contains(Flags::HIDDEN) {
        push_param(&mut params, &mut count, 8);
    }
    if flags.contains(Flags::STRIKEOUT) {
        push_param(&mut params, &mut count, 9);
    }
    append_color_params(&mut params, &mut count, style.fg, true);
    append_color_params(&mut params, &mut count, style.bg, false);

    if count == 0 {
        return;
    }
    render.extend_from_slice(b"\x1b[");
    for (index, param) in params[..count].iter().enumerate() {
        if index != 0 {
            render.push(b';');
        }
        push_decimal(render, *param);
    }
    render.push(b'm');
}

fn append_color_params(
    params: &mut [usize; 20],
    count: &mut usize,
    color: AlacrittyColor,
    foreground: bool,
) {
    let base = if foreground { 38 } else { 48 };
    match color {
        AlacrittyColor::Named(NamedColor::Foreground | NamedColor::Background) => {}
        AlacrittyColor::Named(named) if (named as usize) < 16 => {
            push_param(params, count, base);
            push_param(params, count, 5);
            push_param(params, count, named as usize);
        }
        AlacrittyColor::Named(_) => {}
        AlacrittyColor::Indexed(index) => {
            push_param(params, count, base);
            push_param(params, count, 5);
            push_param(params, count, usize::from(index));
        }
        AlacrittyColor::Spec(rgb) => {
            push_param(params, count, base);
            push_param(params, count, 2);
            push_param(params, count, usize::from(rgb.r));
            push_param(params, count, usize::from(rgb.g));
            push_param(params, count, usize::from(rgb.b));
        }
    }
}

fn push_param(params: &mut [usize; 20], count: &mut usize, value: usize) {
    if *count < params.len() {
        params[*count] = value;
        *count += 1;
    }
}

fn append_cursor(render: &mut Vec<u8>, cursor: Cursor) {
    append_cursor_position(render, usize::from(cursor.row), usize::from(cursor.col));
    if cursor.visible {
        render.extend_from_slice(b"\x1b[?25h");
    } else {
        render.extend_from_slice(b"\x1b[?25l");
    }
}

fn append_cursor_position(render: &mut Vec<u8>, row: usize, col: usize) {
    render.extend_from_slice(b"\x1b[");
    push_decimal(render, row + 1);
    render.push(b';');
    push_decimal(render, col + 1);
    render.push(b'H');
}

fn append_mode_changes(render: &mut Vec<u8>, previous: Modes, current: Modes, explicit: bool) {
    append_private_mode(
        render,
        2004,
        current.bracketed_paste,
        explicit || previous.bracketed_paste != current.bracketed_paste,
    );
    append_private_mode(
        render,
        1,
        current.app_cursor,
        explicit || previous.app_cursor != current.app_cursor,
    );

    if explicit || previous.app_keypad != current.app_keypad {
        if current.app_keypad {
            render.extend_from_slice(b"\x1b=");
        } else {
            render.extend_from_slice(b"\x1b>");
        }
    }

    append_private_mode(
        render,
        1004,
        current.focus_reporting,
        explicit || previous.focus_reporting != current.focus_reporting,
    );

    if explicit || previous.mouse != current.mouse {
        if explicit {
            append_private_mode(render, 1000, false, true);
            append_private_mode(render, 1002, false, true);
            append_private_mode(render, 1003, false, true);
        } else if previous.mouse != MouseMode::Off {
            append_private_mode(render, mouse_mode_number(previous.mouse), false, true);
        }
        if current.mouse != MouseMode::Off {
            append_private_mode(render, mouse_mode_number(current.mouse), true, true);
        }
    }

    append_private_mode(
        render,
        1006,
        current.mouse_sgr,
        explicit || previous.mouse_sgr != current.mouse_sgr,
    );
}

fn mouse_mode_number(mode: MouseMode) -> usize {
    match mode {
        MouseMode::Off => 1000,
        MouseMode::Click => 1000,
        MouseMode::Drag => 1002,
        MouseMode::Motion => 1003,
    }
}

fn append_private_mode(render: &mut Vec<u8>, mode: usize, enabled: bool, emit: bool) {
    if !emit {
        return;
    }
    render.extend_from_slice(b"\x1b[?");
    push_decimal(render, mode);
    render.push(if enabled { b'h' } else { b'l' });
}

fn push_decimal(render: &mut Vec<u8>, mut value: usize) {
    let mut digits = [0u8; 20];
    let mut index = digits.len();
    if value == 0 {
        render.push(b'0');
        return;
    }
    while value != 0 {
        index -= 1;
        digits[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    render.extend_from_slice(&digits[index..]);
}
