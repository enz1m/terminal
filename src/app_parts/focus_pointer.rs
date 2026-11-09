//! Terminal focus and pointer methods.

use super::*;

impl App {
    /// Make the pane at `index` the active one, resizing its PTY if the focus
    /// actually moved.
    ///
    /// Shared by the accessible `FocusPane` route and by a press that landed on
    /// an unfocused leaf's text. A no-op when the pane is
    /// already focused, so a press inside it does not reflow a PTY.
    pub(crate) fn focus_pane_index(&mut self, index: usize) {
        let Some(session) = self.sessions.get_mut(self.active) else {
            return;
        };
        if index >= session.panes.len() || session.active_pane == index {
            return;
        }
        session.active_pane = index;
        self.grid_pointer = GridPointerProbe::default();
        self.resize_active_pty();
    }

    /// Whether a drag/release report belongs to the pane that owns the live
    /// gesture.
    ///
    /// A pointer owns one gesture; the pane it pressed in owns the range. A
    /// report from any other leaf (a drag across the seam) is dropped rather
    /// than allowed to rewrite the neighbour's coordinates.
    pub(crate) fn pointer_report_owns_gesture(&self, target: grid_view::GridPointerTarget) -> bool {
        let Some(owner) = self.grid_pointer.pane.as_deref() else {
            // No gesture recorded (a stray move, or state cleared by a
            // resize): the existing `holding` checks decide.
            return true;
        };
        self.pointer_pane(target).is_some_and(|(_, id)| id == owner)
    }

    /// Resolve which pane a grid pointer report belongs to, as `(index, id)`.
    ///
    /// An unscoped report (the unsplit surface) resolves to the active pane; a
    /// stamped one names its leaf. A stale index (a pane closed between the
    /// press and the report) resolves to `None` rather than to whoever now
    /// occupies that slot.
    pub(crate) fn pointer_pane(
        &self,
        target: grid_view::GridPointerTarget,
    ) -> Option<(usize, String)> {
        let session = self.sessions.get(self.active)?;
        let index = target.pane.unwrap_or(session.active_pane);
        session
            .panes
            .get(index)
            .map(|pane| (index, pane.id.clone()))
    }
}
