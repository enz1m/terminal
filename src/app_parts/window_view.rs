//! Terminal window view methods.

use super::*;

impl App {
    /// The daemon's per-window `view`.
    ///
    /// A daemon is asked what to draw BY ID, which is exactly why a detached
    /// tab can be a real window: the id selects the surface. An unknown id
    /// falls back to the shell so a frame that races a close never panics.
    pub(crate) fn view_window(&self, window: iced::window::Id) -> Element<'_, Message> {
        crate::i18n::localizer().view(|| self.localized_window(window))
    }

    fn localized_window(&self, window: iced::window::Id) -> Element<'_, Message> {
        if let Some(detached) = self.detached_windows.get(&window) {
            return self.detached_view(window, &detached.session);
        }
        if self.main_window == Some(window) {
            self.view()
        } else {
            grid_view::empty_grid()
        }
    }

    /// The MAIN window's surface. Kept as its own no-argument function because
    /// it is the thing every existing render/semantics gate builds, and because
    /// "the main window's view" is a claim independent of which id the runtime
    /// minted for it.
    pub(crate) fn view(&self) -> Element<'_, Message> {
        match self.screen {
            Screen::Shell => self.input_menu_view(self.main_window, self.shell_view()),
        }
    }

    /// The daemon's per-window title. A detached terminal is named after the
    /// session it carries, so the two windows are distinguishable in a
    /// taskbar/alt-tab list.
    pub(crate) fn window_title(&self, window: iced::window::Id) -> String {
        #[cfg(feature = "native-invalidation-acceptance")]
        if let Some(title) = self.native_window_title(window) {
            return title;
        }
        match self.detached_windows.get(&window) {
            Some(detached) => self
                .sessions
                .iter()
                .find(|s| s.id == detached.session)
                .map_or_else(
                    || crate::i18n::label("Terminal").to_owned(),
                    |s| s.title.clone(),
                ),
            None => crate::i18n::label("Terminal").to_owned(),
        }
    }

    /// The daemon's per-window theme. One theme for every window: a detached
    /// terminal is the same app.
    pub(crate) fn window_theme(&self, _window: iced::window::Id) -> iced::Theme {
        self.theme()
    }
}
