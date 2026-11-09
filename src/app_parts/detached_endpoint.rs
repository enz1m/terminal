//! Terminal detached-endpoint methods.

use super::*;

impl App {
    pub(crate) fn set_session_detached(
        &mut self,
        session_id: &str,
        detached: bool,
    ) -> Task<Message> {
        let Some(index) = self.sessions.iter().position(|s| s.id == session_id) else {
            return Task::none();
        };
        if detached == self.sessions[index].detached {
            return Task::none();
        }
        if !detached {
            // Re-dock: close the window; `window_closed` clears the flag. Mark
            // it closing first so input already queued behind this click is
            // rejected rather than reaching a PTY the user just left.
            if let Some(window) = self.detached_window_of(session_id) {
                if let Some(detached) = self.detached_windows.get_mut(&window) {
                    detached.closing = true;
                }
                self.intentional_closes
                    .insert(window, CloseIntent::Dependent);
                self.retire_window_input(window);
                return iced::window::close(window);
            }
            // No window (a restored session that was persisted detached):
            // the flag alone is the whole state, so clear it directly.
            self.sessions[index].detached = false;
            self.invalidate_sidebar();
            return Task::none();
        }
        self.sessions[index].detached = true;
        self.invalidate_sidebar();
        self.open_detached_endpoint_window(self.sessions[index].id.clone())
    }

    pub(crate) fn open_detached_endpoint_window(&self, session: String) -> Task<Message> {
        let (_id, open) = iced::window::open(Self::detached_window_settings());
        open.map(move |window| Message::DetachedWindowOpened {
            session: session.clone(),
            window,
        })
    }
}
