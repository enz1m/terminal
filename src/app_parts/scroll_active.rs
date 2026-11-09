//! Terminal scroll-active methods.

use super::*;

impl App {
    /// A user scroll gesture retires any find scroll target, so the pane's own
    /// offset is the one thing deciding which rows are visible.
    pub(crate) fn clear_find_scroll_override(&mut self) {
        self.find.scroll_to_row = None;
    }

    /// Move the active pane's offset by `delta` rows (positive = toward the
    /// newest output). The single mutation the wheel, PageUp/PageDown and a
    /// scrollbar drag all reach.
    pub(crate) fn scroll_active_pane_rows(&mut self, delta: i64) {
        let sg = self.active_scroll_geometry();
        self.clear_find_scroll_override();
        if let Some(pane) = self.active_pane_mut() {
            pane.scroll.scroll_by_rows(delta, sg);
        }
    }

    /// Set the active pane's first visible row absolutely (a thumb drag).
    pub(crate) fn scroll_active_pane_to_row(&mut self, row: usize) {
        let sg = self.active_scroll_geometry();
        self.clear_find_scroll_override();
        if let Some(pane) = self.active_pane_mut() {
            pane.scroll.set_top_row(row, sg);
        }
    }
}
