//! Terminal reducer batch 15: session rename arms.

use super::*;

impl App {
    pub(crate) fn reduce15_begin_session_rename_0(&mut self) -> Task<Message> {
        // F2 owns the catalogued `tab.rename` action. Seed the active
        // session's title and expose the sidebar-hosted editor instead
        // of opening the pane editor, which is absent in one-pane tabs.
        if self.session_rename.is_none()
            && let Some(session) = self.sessions.get(self.active)
        {
            self.session_rename = Some((session.id.clone(), session.title.clone()));
            self.ensure_sidebar_visible_for_rename();
            self.invalidate_sidebar();
            // The editor takes the keyboard at once; it used to wait for a
            // click while keys went on to the terminal.
            return crate::focus_ids::focus_session_rename_task();
        }

        Task::none()
    }
    pub(crate) fn reduce15_session_rename_edited_1(&mut self, text: String) -> Task<Message> {
        if let Some((_, draft)) = self.session_rename.as_mut() {
            *draft = text;
            self.invalidate_sidebar();
        }

        Task::none()
    }
    pub(crate) fn reduce15_commit_session_rename_2(&mut self) -> Task<Message> {
        if let Some((id, draft)) = self.session_rename.take() {
            let trimmed = draft.trim();
            if !trimmed.is_empty()
                && let Some(s) = self.sessions.iter_mut().find(|s| s.id == id)
            {
                s.title = trimmed.to_owned();
                self.title_overrides.insert(id);
            }
            self.invalidate_sidebar();
        }

        Task::none()
    }
    pub(crate) fn reduce15_cancel_session_rename_3(&mut self) -> Task<Message> {
        self.session_rename = None;
        self.invalidate_sidebar();

        Task::none()
    }
}
