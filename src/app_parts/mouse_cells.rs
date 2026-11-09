//! Terminal mouse and cell methods.

use super::*;

impl App {
    /// The active pane's live mouse-tracking `(mode, encoding)` pair, or
    /// the default (no tracking) when there is no pane.
    pub(crate) fn active_mouse_protocol(&self) -> mouse::MouseProtocol {
        self.active_pane()
            .map(|p| p.mouse_protocol)
            .unwrap_or_default()
    }

    /// The 1-based screen cell a grid pointer target maps to for mouse
    /// reporting. The grid reports a 0-based column and a viewport-
    /// relative (0-based) row; a mouse report is 1-based, clamped to `>= 1`.
    pub(crate) fn report_cell(target: grid_view::GridPointerTarget) -> (usize, usize) {
        let col = target.column + 1;
        let row = usize::try_from(target.window_row).unwrap_or(0) + 1;
        (col, row)
    }

    /// Whether the active pane's program has asked for application cursor keys
    /// (DECCKM). The published per-pane flag `emulator` maintains; a
    /// missing pane defaults to normal mode, the shell's resting state.
    pub(crate) fn active_application_cursor_keys(&self) -> bool {
        self.active_pane()
            .is_some_and(|p| p.application_cursor_keys)
    }

    /// Whether the active pane's program has asked for bracketed paste (DECSET
    /// 2004). The published per-pane flag `emulator` maintains; a missing
    /// pane defaults to off, so a paste is sent raw — the conservative default.
    pub(crate) fn active_bracketed_paste(&self) -> bool {
        self.active_pane().is_some_and(|p| p.bracketed_paste)
    }

    /// The `[start, end)` transcript slice the grid should render for a buffer
    /// of `total_rows` at the current pane height, honouring an active find
    /// scroll target. This is the single decision `content()` and its tests
    /// share, so a test asserting the window moved is asserting the *same*
    /// computation the real view performs — not a parallel copy.
    /// Translate a grid pointer report into a **transcript** cell.
    ///
    /// The grid reports its position inside the *rendered window*; only the app
    /// knows which transcript row that window starts at, which is exactly why
    /// the framework widget does not own a selection. Computed in `i64` and
    /// deliberately **not** clamped to the transcript: a drag above the first
    /// visible row is a negative row, and losing that sign is how "drag up to
    /// select" ends up selecting the top line forever.
    pub(crate) fn transcript_cell(&self, target: grid_view::GridPointerTarget) -> (i64, usize) {
        if let Some(start) = self.input_frame_start {
            return (
                i64::try_from(start)
                    .unwrap_or(i64::MAX)
                    .saturating_add(i64::from(target.window_row)),
                target.column,
            );
        }
        if let Some(origin) = self.input_context.as_ref()
            && let Some((start, _)) = self.scoped_grid_window(origin)
        {
            return (
                i64::try_from(start)
                    .unwrap_or(i64::MAX)
                    .saturating_add(i64::from(target.window_row)),
                target.column,
            );
        }
        let total = self
            .sessions
            .get(self.active)
            .and_then(|s| s.panes.get(s.active_pane))
            .map_or(0, |p| p.transcript.len());
        let (win_start, _) = self.grid_window(total, self.geometry());
        let start = i64::try_from(win_start).unwrap_or(i64::MAX);
        (start + i64::from(target.window_row), target.column)
    }
}
