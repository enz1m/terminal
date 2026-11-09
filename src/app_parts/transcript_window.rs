//! Terminal transcript window methods.

use super::*;

impl App {
    pub(crate) fn grid_window(&self, total_rows: usize, geom: CellGeometry) -> (usize, usize) {
        let sg = self.scroll_geometry(total_rows, geom);
        let focus = if self.find.open {
            self.find.scroll_to_row
        } else {
            None
        };
        Self::transcript_window(
            total_rows,
            sg.visible_rows,
            geom.cell_height,
            self.active_pane_scroll(),
            self.active_pane_is_alt_screen(),
            focus,
        )
    }

    /// The `[start, end)` transcript slice a surface paints, for any pane.
    ///
    /// The one implementation behind [`Self::grid_window`] (the active pane,
    /// which is what the reducer's scroll arithmetic resolves against) and the
    /// per-leaf window each split surface paints. Two copies is how a leaf ends
    /// up painting a different slice than the reducer scrolls.
    pub(crate) fn transcript_window(
        total_rows: usize,
        visible_rows: usize,
        cell_height: f32,
        scroll: scrollback::ScrollPosition,
        alt_screen: bool,
        find_row: Option<usize>,
    ) -> (usize, usize) {
        // A find target is a TRANSIENT override of the same window: it is
        // cleared the moment the user scrolls (`clear_find_scroll_override`), so
        // the pane's own offset is the resting answer rather than a second one.
        if let Some(row) = find_row {
            return model::visible_window(total_rows, visible_rows, Some(row));
        }
        // A degenerate viewport (zero rows fit, or everything fits) shows
        // everything rather than nothing.
        if visible_rows == 0 || visible_rows >= total_rows {
            return model::visible_window(total_rows, visible_rows, None);
        }
        // The alternate screen has no scrollback: `capture_scrollback_before_scroll`
        // refuses to record while it is engaged, and a full-screen program owns the
        // whole viewport. So it is always bottom-pinned, whatever offset the pane
        // was left at on the primary screen; otherwise `btop` starting while the
        // reader is scrolled up paints half a transcript above half a TUI.
        if alt_screen {
            return model::visible_window(total_rows, visible_rows, None);
        }
        let sg = scrollback::ScrollGeometry::new(total_rows, visible_rows, cell_height);
        let top = scroll.top_row(sg);
        (top, (top + visible_rows).min(total_rows))
    }

    /// The slice one pane paints, resolved from that pane's own state.
    pub(crate) fn pane_transcript_window(
        &self,
        pane: &model::Pane,
        visible_rows: usize,
        focused: bool,
    ) -> (usize, usize) {
        // Find state belongs to the active session's search, so only the
        // focused leaf follows a find target.
        let find_row = (focused && self.find.open)
            .then_some(self.find.scroll_to_row)
            .flatten();
        Self::transcript_window(
            pane.transcript.len(),
            visible_rows,
            self.geometry().cell_height,
            pane.scroll,
            pane.alt_screen,
            find_row,
        )
    }

    /// Keep full per-cell projections only for scrollback rows near a viewport.
    ///
    /// A retained row expands into one `TranscriptCell` per column (about
    /// 8 KB for a 200-column row) the first time it is painted, and used to keep
    /// that forever, so wheeling through a 20 000-row history held ~160 MB.
    /// Once more than [`EXPANDED_ROW_BUDGET`] rows are expanded, every row
    /// outside a margin around each viewport that can paint it (the docked
    /// pane, a find target, every detached window) drops its expansion; the
    /// next read re-projects the same frozen source. Below the budget this is a
    /// single counter read.
    pub(crate) fn bound_scrollback_expansion(&mut self) {
        if model::expanded_deferred_rows() <= EXPANDED_ROW_BUDGET {
            return;
        }
        let geom = self.geometry();
        let mut keep: Vec<(usize, usize, Vec<std::ops::Range<usize>>)> = Vec::new();
        for (si, session) in self.sessions.iter().enumerate() {
            for (pi, pane) in session.panes.iter().enumerate() {
                let total = pane.transcript.len();
                let mut zones = Vec::new();
                let visible = if si == self.active {
                    geom.rows_for(self.pane_text_size(&pane.id).height) as usize
                } else {
                    0
                };
                let mut around = |start: usize, end: usize, margin: usize| {
                    zones.push(start.saturating_sub(margin)..end.saturating_add(margin));
                };
                if si == self.active {
                    let focused = pi == session.active_pane;
                    let (start, end) = self.pane_transcript_window(pane, visible, focused);
                    around(start, end, EXPANSION_MARGIN_ROWS);
                    let (start, end) = Self::transcript_window(
                        total,
                        visible,
                        geom.cell_height,
                        pane.scroll,
                        pane.alt_screen,
                        None,
                    );
                    around(start, end, EXPANSION_MARGIN_ROWS);
                }
                // A detached window's own row count is not tracked here; its
                // window lies within the largest possible viewport of its top.
                for detached in self
                    .detached_windows
                    .values()
                    .filter(|detached| detached.session == session.id)
                {
                    if let Some(viewport) = detached.viewports.get(&pane.id) {
                        let (start, end) = Self::transcript_window(
                            total,
                            visible.max(1),
                            geom.cell_height,
                            viewport.scroll,
                            pane.alt_screen,
                            None,
                        );
                        around(start, end, DETACHED_EXPANSION_MARGIN_ROWS);
                    }
                }
                keep.push((si, pi, zones));
            }
        }
        for (si, pi, zones) in keep {
            let Some(pane) = self
                .sessions
                .get_mut(si)
                .and_then(|session| session.panes.get_mut(pi))
            else {
                continue;
            };
            for (index, row) in pane.transcript.iter_mut().enumerate() {
                if !zones.iter().any(|zone| zone.contains(&index)) {
                    row.release_expansion();
                }
            }
        }
    }
}

/// Expanded scrollback rows tolerated before far rows are released (~8 MB at
/// 200 columns).
pub(crate) const EXPANDED_ROW_BUDGET: usize = 1024;
/// Rows kept expanded on each side of a painted window, so ordinary wheel and
/// page scrolling re-uses expansions instead of re-projecting.
pub(crate) const EXPANSION_MARGIN_ROWS: usize = 256;
/// Margin for a detached window, covering any viewport height (rows are
/// clamped to 1000).
pub(crate) const DETACHED_EXPANSION_MARGIN_ROWS: usize = 1000;
