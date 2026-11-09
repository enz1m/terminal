//! Terminal selection methods.

use super::*;

impl App {
    /// The active pane's text selection, or `None`.
    ///
    /// Copy and the unsplit surface read this; a split leaf reads
    /// [`Self::selection_for`] with its own id so it is never handed a
    /// sibling's coordinates.
    pub(crate) fn selection(&self) -> Option<selection::SelectionRange> {
        self.active_pane_id()
            .and_then(|id| self.selections.get(&id).copied())
    }

    /// One named pane's selection.
    pub(crate) fn selection_for(&self, pane_id: &str) -> Option<selection::SelectionRange> {
        self.selections.get(pane_id).copied()
    }

    /// Replace (or clear) the selection of the pane the live gesture owns,
    /// falling back to the active pane when no gesture is in flight.
    pub(crate) fn set_selection(&mut self, range: Option<selection::SelectionRange>) {
        let Some(id) = self
            .grid_pointer
            .pane
            .clone()
            .or_else(|| self.active_pane_id())
        else {
            return;
        };
        match range {
            Some(range) => {
                let _ = self.selections.insert(id, range);
            }
            None => {
                let _ = self.selections.remove(&id);
            }
        }
    }

    /// Recompute the selection from the grid pointer's anchor and focus.
    pub(crate) fn rebuild_selection(&mut self) {
        let (Some(anchor), Some(focus)) = (self.grid_pointer.anchor, self.grid_pointer.focus)
        else {
            self.set_selection(None);
            return;
        };
        let range = selection::SelectionRange::from_anchor_focus(
            anchor,
            focus,
            self.grid_pointer.focus_past_line_end,
        );
        self.set_selection(if range.is_empty() { None } else { Some(range) });
    }

    /// Clear the selection when the terminal is resized (reflow).
    ///
    /// A reflow re-wraps the PTY, so column coordinates lose their meaning.
    /// Every pane's range is cleared: a resize re-wraps all of them, not just
    /// the focused leaf.
    pub(crate) fn clear_selection_on_reflow(&mut self) {
        if let Some(session) = self.sessions.get(self.active) {
            for pane in &session.panes {
                self.selections.remove(&pane.id);
            }
        }
        self.grid_pointer = GridPointerProbe::default();
    }
}
