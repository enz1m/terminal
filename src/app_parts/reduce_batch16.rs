//! Terminal reducer batch 16: sidebar and focus arms.

use super::*;

impl App {
    pub(crate) fn reduce16_dismiss_sidebar_0(&mut self) -> Task<Message> {
        self.nav_split.dismiss_overlay();
        self.resize_active_pty();

        Task::none()
    }
    pub(crate) fn reduce16_sidebar_state_1(&mut self, next: NavSplitState) -> Task<Message> {
        self.nav_split = next;
        self.resize_active_pty();

        Task::none()
    }
    pub(crate) fn reduce16_toggle_sidebar_2(&mut self) -> Task<Message> {
        self.toggle_sidebar();
        self.resize_active_pty();

        Task::none()
    }
    pub(crate) fn reduce16_focus_pane_dir_3(
        &mut self,
        dir: pane_layout::FocusDir,
    ) -> Task<Message> {
        // Geometric, not tree-order: the pane the user sees to the
        // left is the one that takes focus.
        if let Some(session) = self.sessions.get_mut(self.active)
            && let Some(current) = session.panes.get(session.active_pane)
            && let Some(next_id) = pane_layout::focus_direction(&session.layout, &current.id, dir)
            && let Some(idx) = session.panes.iter().position(|p| p.id == next_id)
        {
            session.active_pane = idx;
            self.resize_active_pty();
        }

        Task::none()
    }
}
