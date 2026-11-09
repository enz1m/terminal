//! Terminal reducer batch 18: window arms.

use super::*;

impl App {
    pub(crate) fn reduce18_window_0(&mut self, command: WindowCommand) -> Task<Message> {
        // The app's own close control is a guarded request, not a
        // destruction: it takes the same route as the compositor's.
        if matches!(command, WindowCommand::Close) {
            return match self.main_window {
                Some(id) => self.request_window_close(id),
                None => Task::none(),
            };
        }
        self.main_window
            .and_then(|id| self.window_chrome.get(&id))
            .map_or_else(Task::none, |chrome| chrome.run_command(command))
    }

    pub(crate) fn observe_window_chrome(&mut self, id: iced::window::Id) -> Task<Message> {
        let chrome = self
            .window_chrome
            .entry(id)
            .or_insert_with(|| enzui_widgets::app_shell::ShellWindowState::for_window(id));
        chrome
            .update(enzui_widgets::app_shell::ShellWindowEvent::Observed(id))
            .map(Message::WindowState)
    }

    pub(crate) fn reduce18_window_for_1(
        &mut self,
        id: iced::window::Id,
        command: WindowCommand,
    ) -> Task<Message> {
        // A gesture that already knows its window. The
        // detached terminal's bar publishes this, so dragging it can
        // never move the main window (and vice versa).
        if matches!(command, WindowCommand::Close) {
            return self.request_window_close(id);
        }
        self.window_chrome
            .get(&id)
            .map_or_else(Task::none, |chrome| chrome.run_command(command))
    }
}
