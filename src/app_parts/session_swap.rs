//! Terminal session swap and close methods.

use super::*;

impl App {
    /// Swap two sessions, keeping `active` pointed at the SAME session rather
    /// than the same index.
    ///
    /// Without the fixup, moving the selected tab up would leave the selection
    /// behind on whatever slid into its slot — the tab the user is looking at
    /// would change under them as a side effect of reordering.
    pub(crate) fn swap_sessions(&mut self, a: usize, b: usize) {
        self.sessions.swap(a, b);
        if self.active == a {
            self.active = b;
        } else if self.active == b {
            self.active = a;
        }
        self.invalidate_sidebar();
    }

    /// Close any session by index. Closing an inactive context-menu target
    /// preserves the current stable-id selection; closing the active target
    /// resolves the first surviving id in the MRU history. The returned id is
    /// always the row that is selected after the removal.
    pub(crate) fn close_session_at(&mut self, index: usize) -> Option<SessionRowFocusTarget> {
        self.remove_session_at(index, false)
    }
}
