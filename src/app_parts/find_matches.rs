//! Terminal find-match methods.

use super::*;

impl App {
    /// Resolve the pane's [`FindState`] hits to cell-space highlights against
    /// the visible window `[win_start, win_end)`.
    ///
    /// The stored [`model::FindMatch`] is `(row, col, len)` in **byte offsets**
    /// against the row's flattened `text` string; the grid iterates
    /// `TranscriptRow::cells`, which is **cell-indexed**. Bytes and cells
    /// diverge whenever a cell holds a multi-byte grapheme, so this walks the
    /// row's cells once per hit — accumulating `cell.text.as_str().len()` — to
    /// map the byte range to `[start_cell, end_cell)`. A pure translation, no
    /// paint, no theme touch: the grid owns colour.
    ///
    /// Two clips: hits whose row falls outside the window are dropped, and hits
    /// whose row is inside are remapped from transcript index to the grid's
    /// `row_index = transcript_row - win_start`. The single active hit (per
    /// [`FindState::active_match`], keyed rather than indexed) is flagged so
    /// the grid paints it distinctly from the rest.
    pub(crate) fn find_matches_for_window(
        &self,
        pane: &model::Pane,
        win_start: usize,
        win_end: usize,
    ) -> Vec<grid_view::MatchHighlight> {
        if self.find_owner.as_deref().is_some_and(|id| id != pane.id) {
            return Vec::new();
        }
        Self::find_matches_for_state(&self.find, pane, win_start, win_end)
    }

    pub(crate) fn find_matches_for_state(
        find: &FindState,
        pane: &model::Pane,
        win_start: usize,
        win_end: usize,
    ) -> Vec<grid_view::MatchHighlight> {
        if !find.open || find.matches.is_empty() {
            return Vec::new();
        }
        // `FindMatch: PartialEq` compares on `.key` (see `model.rs`), so
        // equality against the state's active match is the stable-key check
        // without exposing the private field; it keeps the active hit anchored
        // to content when streaming output moves rows.
        let active_match = find.active_match().cloned();
        let mut out: Vec<grid_view::MatchHighlight> = Vec::new();
        for hit in &find.matches {
            if hit.row < win_start || hit.row >= win_end {
                continue;
            }
            let Some(line) = pane.transcript.get(hit.row) else {
                continue;
            };
            let hit_end_byte = hit.col.saturating_add(hit.len);
            let mut acc_bytes = 0usize;
            let mut start_cell = None;
            let mut end_cell = 0;
            for (cols, text) in selection::grapheme_columns(&line.cells) {
                if acc_bytes >= hit_end_byte {
                    break;
                }
                let end_byte = acc_bytes.saturating_add(text.len());
                if end_byte > hit.col {
                    start_cell.get_or_insert(cols.start);
                    end_cell = cols.end;
                }
                acc_bytes = end_byte;
            }
            let Some(start_cell) = start_cell else {
                continue;
            };
            if end_cell <= start_cell {
                continue;
            }
            let active = active_match.as_ref().is_some_and(|m| m == hit);
            out.push(grid_view::MatchHighlight {
                row: hit.row - win_start,
                start_cell,
                end_cell,
                active,
            });
        }
        out
    }
}
