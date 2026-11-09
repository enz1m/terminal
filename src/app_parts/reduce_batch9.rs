//! Terminal reducer batch 9: session arms.

use super::*;

impl App {
    pub(crate) fn reduce9_select_session_0(&mut self, i: usize) -> Task<Message> {
        // A detached tab is shown by its own window, not the main one.
        if i < self.sessions.len() && !self.in_own_window(i) {
            self.activate_session(i);
            self.resize_active_pty();
        }

        Task::none()
    }
    pub(crate) fn reduce9_select_next_session_1(&mut self) -> Task<Message> {
        // Wrap past the last tab back to the first. Route through
        // Route through SelectSession so MRU and acknowledgement stay
        // in one place.
        if let Some(next) = self.docked_session_from(self.active, 1) {
            return self.update(Message::SelectSession(next));
        }

        Task::none()
    }
    pub(crate) fn reduce9_select_prev_session_2(&mut self) -> Task<Message> {
        if let Some(prev) = self.docked_session_from(self.active, -1) {
            return self.update(Message::SelectSession(prev));
        }

        Task::none()
    }
    pub(crate) fn reduce9_new_local_3(&mut self) -> Task<Message> {
        self.new_local_session()
    }

    /// The next tab shown in the main window from `from`, stepping by `step`
    /// (1 or -1) and wrapping, past detached tabs.
    pub(crate) fn docked_session_from(&self, from: usize, step: isize) -> Option<usize> {
        let len = self.sessions.len();
        (1..=len)
            .map(|offset| {
                (from as isize + step * offset as isize).rem_euclid(len as isize) as usize
            })
            .find(|&index| !self.in_own_window(index))
    }

    /// Moves the main window off a tab shown in its own window: to the most
    /// recently used docked tab, else to any docked tab. With every tab
    /// detached it stays, and the main window shows no terminal.
    pub(crate) fn show_a_docked_session(&mut self) {
        if !self.in_own_window(self.active) {
            return;
        }
        let recent = self.session_mru.iter().find_map(|id| {
            let index = self.sessions.iter().position(|session| &session.id == id)?;
            (!self.in_own_window(index)).then_some(index)
        });
        if let Some(index) = recent.or_else(|| self.docked_session_from(self.active, 1)) {
            self.activate_session(index);
            self.resize_active_pty();
        }
    }
}
