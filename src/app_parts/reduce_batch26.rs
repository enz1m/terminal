//! Terminal reducer batch 26: clipboard, toast and history arms.

use super::*;

impl App {
    pub(crate) fn reduce26_request_copy_0(&mut self) -> Task<Message> {
        // Copy the selected text to the system clipboard.
        if let Some(sel) = self.selection()
            && let Some(pane) = self.active_pane()
        {
            let text = sel.selected_text(&pane.transcript);
            if !text.is_empty() {
                return iced::clipboard::write(text).discard();
            }
        }

        Task::none()
    }
    pub(crate) fn reduce26_toast_1(&mut self, event: ToastEvent) -> Task<Message> {
        let now = match &event {
            ToastEvent::Tick(now) => {
                // This recipe has delivered its one intended wake. If
                // clock granularity made it early, disarming here lets
                // reconciliation below arm only the small remainder
                // instead of waiting for a second full interval
                // emission.
                self.toast_expiry = ToastExpirySchedule::default();
                *now
            }
            _ => Instant::now(),
        };
        self.toasts.update(event);
        self.toasts.sync_expiry(&mut self.toast_expiry, now);

        Task::none()
    }
    pub(crate) fn reduce26_open_history_2(&mut self) -> Task<Message> {
        self.history_open = true;

        Task::none()
    }
    pub(crate) fn reduce26_toggle_history_3(&mut self) -> Task<Message> {
        self.history_open = !self.history_open;

        Task::none()
    }
    pub(crate) fn reduce26_close_history_4(&mut self) -> Task<Message> {
        self.history_open = false;

        Task::none()
    }
    pub(crate) fn reduce26_copy_history_command_5(&mut self, command: String) -> Task<Message> {
        self.post_history_copy_acknowledgement_at(Instant::now());
        iced::clipboard::write(command).discard()
    }
}
