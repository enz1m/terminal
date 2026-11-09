//! Terminal reducer batch 10: restore and close arms.

use super::*;

impl App {
    pub(crate) fn reduce10_reconnect_restored_pane_0(&mut self) -> Task<Message> {
        if let Some((session, pane)) = self.sessions.get(self.active).and_then(|session| {
            session
                .panes
                .get(session.active_pane)
                .map(|pane| (session.id.clone(), pane.id.clone()))
        }) && self.restored_outputs.contains_key(&pane)
        {
            return self.start_pane_endpoint(&session, &pane);
        }

        Task::none()
    }
    pub(crate) fn reduce10_reset_saved_sessions_1(&mut self) -> Task<Message> {
        if self.session_load_error.is_some() {
            self.session_store.authorize_replacement();
            self.session_load_error = None;
            self.secure_pins.retry();
            self.empty_save_requested = true;
            return self.queue_secure_pin_save();
        }

        Task::none()
    }
    pub(crate) fn reduce10_close_active_2(&mut self) -> Task<Message> {
        // Closing the focused tab must land focus on the
        // successor session row, not strand the ring at the top of
        // the shell.
        let detached_close = self
            .input_context
            .as_ref()
            .and_then(|origin| origin.window)
            .is_some_and(|window| self.detached_windows.contains_key(&window));
        let closed = if detached_close {
            self.remove_session_at(self.active, true)
        } else {
            self.close_active_session()
        };
        if !detached_close && let Some(target) = closed {
            return restore_session_row_focus_task(target);
        }

        Task::none()
    }
}
