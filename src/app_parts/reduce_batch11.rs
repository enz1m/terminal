//! Terminal reducer batch 11: pane-split arms.

use super::*;

impl App {
    pub(crate) fn reduce11_new_pane_0(&mut self) -> Task<Message> {
        self.split_active_pane(pane_layout::SplitDirection::Horizontal)
    }
    pub(crate) fn reduce11_new_pane_down_1(&mut self) -> Task<Message> {
        self.split_active_pane(pane_layout::SplitDirection::Vertical)
    }
    pub(crate) fn reduce11_detached_split_pane_2(
        &mut self,
        window: iced::window::Id,
        pane: String,
        direction: pane_layout::SplitDirection,
    ) -> Task<Message> {
        let Some((index, _)) = self.detached_target(window, &pane) else {
            return Task::none();
        };
        let active = self.active;
        self.active = index;
        let task = self.split_active_pane(direction);
        self.active = active;
        if let Some(detached) = self.detached_windows.get(&window) {
            let (session, size) = (detached.session.clone(), detached.size);
            self.resize_detached_pty(&session, size);
        }
        task
    }
    pub(crate) fn reduce11_close_pane_3(&mut self) -> Task<Message> {
        self.close_active_pane();

        Task::none()
    }
}
