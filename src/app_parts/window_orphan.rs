//! Terminal window-settings and orphan methods.

use super::*;

impl App {
    pub(crate) fn detached_window_settings() -> iced::window::Settings {
        let mut settings = app_identity().apply_to_window_settings(
            enzui_widgets::app_shell::enz_window_settings(ShellLayout::FloatingTitlebar),
        );
        // Every window this app owns routes its close through the same review.
        settings.exit_on_close_request = enzui_core::close_guard::EXIT_ON_CLOSE_REQUEST;
        #[cfg(feature = "native-invalidation-acceptance")]
        if std::env::var_os(enzui_core::app_runtime::native_invalidation::CONTROL_PATH_ENV)
            .is_some()
        {
            settings.size = iced::Size::new(580.0, 660.0);
        }
        settings
    }

    /// Windows whose session is gone, waiting to be closed by `update`'s drain
    ///.
    pub(crate) fn take_orphaned_windows(&mut self) -> Option<Task<Message>> {
        if self.orphaned_windows.is_empty() {
            return None;
        }
        let orphans: Vec<iced::window::Id> = std::mem::take(&mut self.orphaned_windows);
        // The drain happens inside the same `update` that parked the window, so
        // by the time a test can look `orphaned_windows` is already empty and
        // the `Task` itself is opaque. Recording what the PRODUCTION path closed
        // is the only observable a disposal gate can stand on.
        #[cfg(test)]
        self.disposed_windows.extend(orphans.iter().copied());
        let closes: Vec<Task<Message>> = orphans
            .into_iter()
            .map(|id| {
                self.intentional_closes.insert(id, CloseIntent::Dependent);
                iced::window::close(id)
            })
            .collect();
        Some(Task::batch(closes))
    }
}
