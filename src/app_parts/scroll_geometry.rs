//! Terminal scroll geometry methods.

use super::*;

impl App {
    /// The transcript geometry this pane's scroll offset is resolved against
    ///: the FULL transcript length, the rows that fit, and the row
    /// pitch.
    ///
    /// The visible row count is the UNOBSTRUCTED height (the floating bar
    /// insets the grid's top), not the full pane — using the full height would
    /// window in more rows than fit below the bar.
    ///
    /// One function so the grid's slice, the scrollbar's range and a page-scroll
    /// distance are all derived from the same three numbers. A scrollbar built
    /// from the visible slice instead of `total_rows` always looks full.
    pub(crate) fn scroll_geometry(
        &self,
        total_rows: usize,
        geom: CellGeometry,
    ) -> scrollback::ScrollGeometry {
        let visible_rows = geom.rows_for(self.terminal_text_height()) as usize;
        scrollback::ScrollGeometry::new(total_rows, visible_rows, geom.cell_height)
    }

    /// The active pane's scroll position (a pinned default when there is no
    /// pane, so callers need no special case).
    pub(crate) fn active_pane_scroll(&self) -> scrollback::ScrollPosition {
        self.active_pane()
            .map_or_else(scrollback::ScrollPosition::pinned_to_tail, |p| p.scroll)
    }
}
