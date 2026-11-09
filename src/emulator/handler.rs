//! The emulator's escape handler, with the app's hooks around it.
//!
//! Every [`Handler`] method passes straight through to the `alacritty_terminal`
//! `Term`, except:
//!
//! - a line feed, newline, autowrap or `CSI S` that is about to push rows off
//!   the top of a full-screen scroll region on the primary screen copies those
//!   rows first, for the app's scrollback (the emulator keeps no history);
//! - the scroll region is tracked here, the same way the emulator sets it,
//!   since that decides whether a scroll leaves the screen;
//! - clipboard (OSC 52), palette changes and queries, text-area size reports
//!   and the keyboard-protocol modes stay inert, as policy: terminal output may
//!   not read or change the desktop, and the app's key encoder speaks no
//!   kitty or modifyOtherKeys protocol.
//!
//! The pass-through methods are generated from `vte`'s trait; regenerate them
//! when `vte` changes its handler.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::cursor_icon::CursorIcon;
use alacritty_terminal::vte::ansi::{
    Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, Handler, Hyperlink, KeyboardModes,
    KeyboardModesApplyBehavior, LineClearMode, Mode, ModifyOtherKeys, PrivateMode, Rgb,
    ScpCharPath, ScpUpdateMode, StandardCharset, TabulationClearMode,
};
use unicode_width::UnicodeWidthChar;

use super::Events;
use super::project::RetainedRow;

/// The rows of the scroll region, `start..end`.
pub(super) type Region = (usize, usize);

/// One parse pass over the emulator.
pub(super) struct Hooked<'a> {
    pub term: &'a mut Term<Events>,
    pub region: &'a mut Region,
    /// Rows that left the screen in this pass, oldest first.
    pub scrolled: &'a mut Vec<RetainedRow>,
    /// The primary screen's visible rows, kept while the alternate screen
    /// hides them (nothing can change them meanwhile).
    pub primary: &'a mut Option<Vec<RetainedRow>>,
    /// Set when output edited rows in place (erased, inserted or deleted
    /// cells or lines, or scrolled a partial region): selections and find
    /// positions on those rows no longer name what they did.
    pub edited: &'a mut bool,
}

impl Hooked<'_> {
    /// Whether a scroll now moves rows off the top of the whole screen.
    fn scrolls_off_screen(&self) -> bool {
        let rows = self.term.screen_lines();
        *self.region == (0, rows) && !self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Copies the top `count` visible rows before they scroll off.
    fn keep_top(&mut self, count: usize) {
        let count = count.min(self.term.screen_lines());
        for row in 0..count {
            let line = &self.term.grid()[Line(row as i32)];
            self.scrolled.push(RetainedRow::copy_row(line));
        }
    }

    /// Whether the cursor is on the last row of the scroll region, where a
    /// line feed scrolls.
    fn at_bottom(&self) -> bool {
        self.term.grid().cursor.point.line.0 + 1 == self.region.1 as i32
    }
}

fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

impl Hooked<'_> {
    /// Joins `c` to the wide emoji just printed when it continues it: a skin
    /// tone modifier, or anything after a zero-width joiner the emoji already
    /// carries (the emulator gives each its own cells). The cluster then
    /// draws as one glyph, as in other terminals.
    fn join_emoji(&mut self, c: char, width: Option<usize>) -> bool {
        let modifier = ('\u{1F3FB}'..='\u{1F3FF}').contains(&c);
        if !modifier && width == Some(0) {
            return false;
        }
        let cursor = &self.term.grid().cursor;
        let (line, col) = (cursor.point.line, cursor.point.column.0);
        let last = if cursor.input_needs_wrap {
            col
        } else if col > 0 {
            col - 1
        } else {
            return false;
        };
        let row = &self.term.grid()[line];
        let base = if row[Column(last)].flags.contains(Flags::WIDE_CHAR_SPACER) && last > 0 {
            last - 1
        } else {
            last
        };
        let cell = &row[Column(base)];
        if !cell.flags.contains(Flags::WIDE_CHAR) {
            return false;
        }
        let after_joiner = cell.zerowidth().and_then(|z| z.last()) == Some(&'\u{200D}');
        if !(modifier || after_joiner) {
            return false;
        }
        self.term.grid_mut()[line][Column(base)].push_zerowidth(c);
        true
    }

    /// A flag is two regional indicators drawn as one two-column glyph, as
    /// other terminals draw it; the emulator prints each indicator in its own
    /// column. When `c` completes a pair with the lone indicator just
    /// printed, it is printed as usual (so the cursor and wrapping follow the
    /// emulator's rules) and then joined to that cell, which becomes a wide
    /// cell with this column as its spacer.
    fn pair_flag(&mut self, c: char) -> bool {
        let cursor = &self.term.grid().cursor;
        let (line, col) = (cursor.point.line, cursor.point.column);
        if cursor.input_needs_wrap || col.0 == 0 || col.0 >= self.term.columns() {
            return false;
        }
        let first = &self.term.grid()[line][col - 1];
        if !is_regional_indicator(first.c)
            || first.zerowidth().is_some()
            || first
                .flags
                .intersects(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER)
        {
            return false;
        }
        self.term.input(c);
        let grid = self.term.grid_mut();
        let first = &mut grid[line][col - 1];
        first.push_zerowidth(c);
        first.flags.insert(Flags::WIDE_CHAR);
        let spacer = &mut grid[line][col];
        spacer.c = ' ';
        spacer.flags.insert(Flags::WIDE_CHAR_SPACER);
        true
    }
}

impl Handler for Hooked<'_> {
    #[inline]
    fn input(&mut self, c: char) {
        // The emulator wraps before printing when the last print filled the
        // row, or when a wide glyph does not fit in the last column.
        // One width lookup serves the wrap, the emoji join and the emulator.
        let width = if c.is_ascii() { None } else { c.width() };
        let cursor = &self.term.grid().cursor;
        let wraps = cursor.input_needs_wrap
            || (width == Some(2) && cursor.point.column.0 + 1 >= self.term.columns());
        if wraps
            && self.term.mode().contains(TermMode::LINE_WRAP)
            && self.at_bottom()
            && self.scrolls_off_screen()
        {
            self.keep_top(1);
        }
        if !c.is_ascii() {
            if is_regional_indicator(c) && self.pair_flag(c) {
                return;
            }
            if self.join_emoji(c, width) {
                return;
            }
            self.term.input_width(c, width);
            return;
        }
        self.term.input(c);
    }

    /// A run of printable ASCII, a row at a time: each row's first character
    /// is where a wrap (and a row scrolling off the top) can happen, exactly
    /// as [`Self::input`] would meet it.
    #[inline]
    fn input_ascii(&mut self, text: &str) {
        #[cfg(test)]
        if SINGLE_CHARACTERS.with(std::cell::Cell::get) {
            text.chars().for_each(|c| self.input(c));
            return;
        }
        let mut rest = text;
        while !rest.is_empty() {
            let wraps = self.term.grid().cursor.input_needs_wrap
                && self.term.mode().contains(TermMode::LINE_WRAP);
            if wraps && self.at_bottom() && self.scrolls_off_screen() {
                self.keep_top(1);
            }
            let column = if wraps {
                0
            } else {
                self.term.grid().cursor.point.column.0
            };
            let count = self
                .term
                .columns()
                .saturating_sub(column)
                .clamp(1, rest.len());
            let (row, tail) = rest.split_at(count);
            self.term.input_ascii(row);
            rest = tail;
        }
    }

    #[inline]
    fn linefeed(&mut self) {
        if self.at_bottom() {
            if self.scrolls_off_screen() {
                self.keep_top(1);
            } else {
                *self.edited = true;
            }
        }
        self.term.linefeed();
    }

    #[inline]
    fn newline(&mut self) {
        if self.at_bottom() {
            if self.scrolls_off_screen() {
                self.keep_top(1);
            } else {
                *self.edited = true;
            }
        }
        self.term.newline();
    }

    #[inline]
    fn scroll_up(&mut self, lines: usize) {
        if self.scrolls_off_screen() {
            self.keep_top(lines);
        } else {
            *self.edited = true;
        }
        self.term.scroll_up(lines);
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        // As the emulator does: the bottom defaults to the last row, an empty
        // region is refused, and both ends are clamped to the screen.
        let rows = self.term.screen_lines();
        let bottom = bottom.unwrap_or(rows);
        if top < bottom {
            *self.region = ((top.saturating_sub(1)).min(rows), bottom.min(rows));
        }
        self.term.set_scrolling_region(top, bottom.into());
    }

    fn set_private_mode(&mut self, mode: PrivateMode) {
        use alacritty_terminal::vte::ansi::NamedPrivateMode;
        if mode == PrivateMode::Named(NamedPrivateMode::SwapScreenAndSetRestoreCursor)
            && !self.term.mode().contains(TermMode::ALT_SCREEN)
        {
            let rows = self.term.screen_lines();
            *self.primary = Some(
                (0..rows)
                    .map(|row| RetainedRow::copy_row(&self.term.grid()[Line(row as i32)]))
                    .collect(),
            );
        }
        self.term.set_private_mode(mode);
    }

    fn reset_state(&mut self) {
        *self.region = (0, self.term.screen_lines());
        self.term.reset_state();
    }

    fn clipboard_store(&mut self, _: u8, _: &[u8]) {}
    fn clipboard_load(&mut self, _: u8, _: &str) {}
    fn set_color(&mut self, _: usize, _: Rgb) {}
    fn reset_color(&mut self, _: usize) {}
    fn dynamic_color_sequence(&mut self, _: String, _: usize, _: &str) {}
    fn text_area_size_pixels(&mut self) {}
    fn text_area_size_chars(&mut self) {}
    fn report_keyboard_mode(&mut self) {}
    fn push_keyboard_mode(&mut self, _: KeyboardModes) {}
    fn pop_keyboard_modes(&mut self, _: u16) {}
    fn set_keyboard_mode(&mut self, _: KeyboardModes, _: KeyboardModesApplyBehavior) {}
    fn set_modify_other_keys(&mut self, _: ModifyOtherKeys) {}
    fn report_modify_other_keys(&mut self) {}

    #[inline]
    fn set_title(&mut self, a0: Option<String>) {
        self.term.set_title(a0);
    }
    #[inline]
    fn set_cursor_style(&mut self, a0: Option<CursorStyle>) {
        self.term.set_cursor_style(a0);
    }
    #[inline]
    fn set_cursor_shape(&mut self, a0: CursorShape) {
        self.term.set_cursor_shape(a0);
    }
    #[inline]
    fn goto(&mut self, a0: i32, a1: usize) {
        self.term.goto(a0, a1);
    }
    #[inline]
    fn goto_line(&mut self, a0: i32) {
        self.term.goto_line(a0);
    }
    #[inline]
    fn goto_col(&mut self, a0: usize) {
        self.term.goto_col(a0);
    }
    #[inline]
    fn insert_blank(&mut self, a0: usize) {
        *self.edited = true;
        self.term.insert_blank(a0);
    }
    #[inline]
    fn move_up(&mut self, a0: usize) {
        self.term.move_up(a0);
    }
    #[inline]
    fn move_down(&mut self, a0: usize) {
        self.term.move_down(a0);
    }
    #[inline]
    fn identify_terminal(&mut self, a0: Option<char>) {
        self.term.identify_terminal(a0);
    }
    #[inline]
    fn device_status(&mut self, a0: usize) {
        self.term.device_status(a0);
    }
    #[inline]
    fn move_forward(&mut self, a0: usize) {
        self.term.move_forward(a0);
    }
    #[inline]
    fn move_backward(&mut self, a0: usize) {
        self.term.move_backward(a0);
    }
    #[inline]
    fn move_down_and_cr(&mut self, a0: usize) {
        self.term.move_down_and_cr(a0);
    }
    #[inline]
    fn move_up_and_cr(&mut self, a0: usize) {
        self.term.move_up_and_cr(a0);
    }
    #[inline]
    fn put_tab(&mut self, a0: u16) {
        self.term.put_tab(a0);
    }
    #[inline]
    fn backspace(&mut self) {
        self.term.backspace();
    }
    #[inline]
    fn carriage_return(&mut self) {
        self.term.carriage_return();
    }
    #[inline]
    fn bell(&mut self) {
        self.term.bell();
    }
    #[inline]
    fn substitute(&mut self) {
        self.term.substitute();
    }
    #[inline]
    fn set_horizontal_tabstop(&mut self) {
        self.term.set_horizontal_tabstop();
    }
    #[inline]
    fn scroll_down(&mut self, a0: usize) {
        *self.edited = true;
        self.term.scroll_down(a0);
    }
    #[inline]
    fn insert_blank_lines(&mut self, a0: usize) {
        *self.edited = true;
        self.term.insert_blank_lines(a0);
    }
    #[inline]
    fn delete_lines(&mut self, a0: usize) {
        *self.edited = true;
        self.term.delete_lines(a0);
    }
    #[inline]
    fn erase_chars(&mut self, a0: usize) {
        *self.edited = true;
        self.term.erase_chars(a0);
    }
    #[inline]
    fn delete_chars(&mut self, a0: usize) {
        *self.edited = true;
        self.term.delete_chars(a0);
    }
    #[inline]
    fn move_backward_tabs(&mut self, a0: u16) {
        self.term.move_backward_tabs(a0);
    }
    #[inline]
    fn move_forward_tabs(&mut self, a0: u16) {
        self.term.move_forward_tabs(a0);
    }
    #[inline]
    fn save_cursor_position(&mut self) {
        self.term.save_cursor_position();
    }
    #[inline]
    fn restore_cursor_position(&mut self) {
        self.term.restore_cursor_position();
    }
    #[inline]
    fn clear_line(&mut self, a0: LineClearMode) {
        *self.edited = true;
        self.term.clear_line(a0);
    }
    #[inline]
    fn clear_screen(&mut self, a0: ClearMode) {
        *self.edited = true;
        self.term.clear_screen(a0);
    }
    #[inline]
    fn clear_tabs(&mut self, a0: TabulationClearMode) {
        self.term.clear_tabs(a0);
    }
    #[inline]
    fn set_tabs(&mut self, a0: u16) {
        self.term.set_tabs(a0);
    }
    #[inline]
    fn reverse_index(&mut self) {
        *self.edited = true;
        self.term.reverse_index();
    }
    #[inline]
    fn terminal_attribute(&mut self, a0: Attr) {
        self.term.terminal_attribute(a0);
    }
    #[inline]
    fn set_mode(&mut self, a0: Mode) {
        self.term.set_mode(a0);
    }
    #[inline]
    fn unset_mode(&mut self, a0: Mode) {
        self.term.unset_mode(a0);
    }
    #[inline]
    fn report_mode(&mut self, a0: Mode) {
        self.term.report_mode(a0);
    }
    #[inline]
    fn unset_private_mode(&mut self, a0: PrivateMode) {
        self.term.unset_private_mode(a0);
    }
    #[inline]
    fn report_private_mode(&mut self, a0: PrivateMode) {
        self.term.report_private_mode(a0);
    }
    #[inline]
    fn set_keypad_application_mode(&mut self) {
        self.term.set_keypad_application_mode();
    }
    #[inline]
    fn unset_keypad_application_mode(&mut self) {
        self.term.unset_keypad_application_mode();
    }
    #[inline]
    fn set_active_charset(&mut self, a0: CharsetIndex) {
        self.term.set_active_charset(a0);
    }
    #[inline]
    fn configure_charset(&mut self, a0: CharsetIndex, a1: StandardCharset) {
        self.term.configure_charset(a0, a1);
    }
    #[inline]
    fn decaln(&mut self) {
        self.term.decaln();
    }
    #[inline]
    fn push_title(&mut self) {
        self.term.push_title();
    }
    #[inline]
    fn pop_title(&mut self) {
        self.term.pop_title();
    }
    #[inline]
    fn set_hyperlink(&mut self, a0: Option<Hyperlink>) {
        self.term.set_hyperlink(a0);
    }
    #[inline]
    fn set_mouse_cursor_icon(&mut self, a0: CursorIcon) {
        self.term.set_mouse_cursor_icon(a0);
    }
    #[inline]
    fn set_scp(&mut self, a0: ScpCharPath, a1: ScpUpdateMode) {
        self.term.set_scp(a0, a1);
    }
}

#[cfg(test)]
thread_local! {
    /// Tests compare text taken a row at a time with the per-character path.
    pub(super) static SINGLE_CHARACTERS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
