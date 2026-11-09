//! Terminal reducer batch 8: detached history arms.

use super::*;

impl App {
    pub(crate) fn reduce8_detached_toggle_history_0(
        &mut self,
        window: iced::window::Id,
    ) -> Task<Message> {
        if let Some(detached) = self.detached_windows.get_mut(&window) {
            detached.history_open = !detached.history_open;
        }

        Task::none()
    }
    pub(crate) fn reduce8_detached_close_history_1(
        &mut self,
        window: iced::window::Id,
    ) -> Task<Message> {
        if let Some(detached) = self.detached_windows.get_mut(&window) {
            detached.history_open = false;
        }

        Task::none()
    }
    pub(crate) fn reduce8_detached_paste_history_command_2(
        &mut self,
        window: iced::window::Id,
        command: String,
    ) -> Task<Message> {
        let target = self.detached_windows.get(&window).and_then(|detached| {
            let session_id = detached.session.clone();
            self.sessions
                .iter()
                .find(|s| s.id == session_id)
                .and_then(|s| s.panes.get(s.active_pane))
                .map(|pane| (session_id, pane.id.clone(), pane.bracketed_paste))
        });
        if let Some(detached) = self.detached_windows.get_mut(&window) {
            detached.history_open = false;
        }
        if let Some((session_id, pane_id, bracketed)) = target
            && !command.is_empty()
        {
            match escape_policy::normalized_paste(&command) {
                Ok(payload) if payload.is_empty() => {}
                Ok(payload) => {
                    let bytes = paste_bytes(&payload, bracketed);
                    self.exclude_pinned_input(&pane_id);
                    self.write_pane_pty(&session_id, &pane_id, &bytes);
                }
                Err(refusal) => {
                    self.pane_feedback = Some(refusal.user_summary().to_owned());
                }
            }
        }

        Task::none()
    }
}
