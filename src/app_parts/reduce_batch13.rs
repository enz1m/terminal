//! Terminal reducer batch 13: pane rename commit and detach arms.

use super::*;

impl App {
    pub(crate) fn reduce13_commit_pane_rename_0(&mut self) -> Task<Message> {
        if let Some(draft) = self.pane_rename_draft.take() {
            let trimmed = draft.trim();
            // A blank name is a cancel — a pane must always have a title.
            if !trimmed.is_empty()
                && let Some(s) = self.sessions.get_mut(self.active)
                && let Some(p) = s.panes.get_mut(s.active_pane)
            {
                p.title = trimmed.to_owned();
            }
        }

        Task::none()
    }
    pub(crate) fn reduce13_cancel_pane_rename_1(&mut self) -> Task<Message> {
        self.pane_rename_draft = None;

        Task::none()
    }
    #[cfg(test)]
    pub(crate) fn reduce13_toggle_detach_2(&mut self) -> Task<Message> {
        if let Some(id) = self.sessions.get(self.active).map(|s| s.id.clone()) {
            return self.toggle_session_detached(&id);
        }

        Task::none()
    }
}
