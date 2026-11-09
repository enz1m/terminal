//! Terminal geometry and detach methods.

use super::*;

impl App {
    /// One place that flips a session's detached state, for the sidebar context
    /// row, the pane-bar button and the detached window's own Re-dock.
    ///
    /// Detaching OPENS a window; re-docking CLOSES it and lets the close event
    /// do the re-docking. That asymmetry is deliberate and it is the honest
    /// ownership rule: the window's destruction is the one fact both routes
    /// agree on, so a user who re-docks from the button and a user who clicks
    /// the detached window's X take the same path and cannot leave a session
    /// marked detached with no window (or docked with a window still up).
    /// Toggle a session's window ownership. A multi-pane workspace first
    /// extracts only its selected pane; the remaining panes stay grouped and
    /// visible in the main window.
    pub(crate) fn toggle_session_detached(&mut self, session_id: &str) -> Task<Message> {
        let Some(index) = self.sessions.iter().position(|s| s.id == session_id) else {
            return Task::none();
        };
        if self.sessions[index].detached {
            return self.set_session_detached(session_id, false);
        }
        let detached_index = if self.sessions[index].panes.len() > 1 {
            match pane_group::extract_pane_for_detach(&mut self.sessions, index) {
                Some(index) => index,
                None => return Task::none(),
            }
        } else {
            index
        };
        self.invalidate_sidebar();
        let id = self.sessions[detached_index].id.clone();
        self.set_session_detached(&id, true)
    }
}
