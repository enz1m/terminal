//! Terminal orphan and detached-window query methods.

use super::*;

impl App {
    /// The window a detached session owns, if it has one.
    pub(crate) fn detached_window_of(&self, session_id: &str) -> Option<iced::window::Id> {
        self.detached_windows
            .iter()
            .find(|(_, detached)| detached.session == session_id)
            .map(|(id, _)| *id)
    }

    /// Whether the tab at `index` is detached into a window that is open. A
    /// tab persisted detached whose window has not opened (or failed to)
    /// stays in the main window, so it cannot become unreachable.
    pub(crate) fn in_own_window(&self, index: usize) -> bool {
        self.sessions.get(index).is_some_and(|session| {
            session.detached && self.detached_window_of(&session.id).is_some()
        })
    }
}
