//! Terminal reducer batch 17: titlebar and settings arms.

use super::*;

impl App {
    pub(crate) fn reduce17_toggle_titlebar_overflow_0(&mut self) -> Task<Message> {
        self.titlebar_overflow_open = !self.titlebar_overflow_open;

        Task::none()
    }
    pub(crate) fn reduce17_close_titlebar_overflow_1(&mut self) -> Task<Message> {
        self.titlebar_overflow_open = false;

        Task::none()
    }
    pub(crate) fn reduce17_open_settings_3(&mut self) -> Task<Message> {
        self.settings_open = true;
        settings_surface::sync(self);

        Task::none()
    }
    pub(crate) fn reduce17_close_settings_4(&mut self) -> Task<Message> {
        let task = self.dismiss_settings_focus(enzui_core::modal::DismissCause::CloseButton);
        settings_surface::sync(self);
        task
    }
    pub(crate) fn reduce17_settings_exit_settled_5(
        &mut self,
        key: ModalPresentationKey,
    ) -> Task<Message> {
        settings_surface::finish_exit(self, key);

        Task::none()
    }
}
