//! Terminal reducer batch 19: window lifecycle arms.

use super::*;

impl App {
    pub(crate) fn reduce19_main_window_opened_0(&mut self, id: iced::window::Id) -> Task<Message> {
        self.main_window = Some(id);
        self.windows.opened_primary(id);
        self.opening_window = None;
        self.invalidate_sidebar();
        Task::batch([
            self.resume_endpoint_restores(),
            self.shell_activity.bind_window(id),
        ])
    }
    pub(crate) fn reduce19_detached_window_opened_1(
        &mut self,
        session: String,
        window: iced::window::Id,
    ) -> Task<Message> {
        self.windows.opened(window);
        // `window::open` resolves asynchronously. The tab may have
        // been re-docked/closed meanwhile, or a newer open may already
        // own it. Reject that stale completion and close its surface;
        // accepting it would resurrect a detached window with no live
        // owner.
        let live = self.sessions.iter().find(|item| item.id == session);
        let already_open = self.detached_window_of(&session).is_some();
        if let Some(live) = live.filter(|item| item.detached && !already_open) {
            let panes: Vec<(String, scrollback::ScrollPosition)> = live
                .panes
                .iter()
                .map(|pane| (pane.id.clone(), pane.scroll))
                .collect();
            self.detached_windows
                .insert(window, DetachedWindow::new(session, &panes));
            // The tab now lives in its own window; the main one moves on.
            self.show_a_docked_session();
            self.invalidate_sidebar();
        } else {
            self.orphaned_windows.push(window);
        }
        let bind = if self.detached_windows.contains_key(&window) {
            self.shell_activity.bind_window(window)
        } else {
            Task::none()
        };
        Task::batch([self.resume_endpoint_restores(), bind])
    }
    pub(crate) fn reduce19_window_resized_2(
        &mut self,
        id: iced::window::Id,
        size: iced::Size,
    ) -> Task<Message> {
        // Trap 4: `resize_events()` is app-wide. A detached terminal
        // dragged wider must not drive the MAIN window's responsive
        // rungs (its sidebar and titlebar fold off a width), so the
        // size is filed against the window it belongs to.
        if Some(id) == self.main_window {
            return self.update(Message::ContentResized(size));
        }
        if self.detached_windows.contains_key(&id) {
            self.clear_window_selection(id);
        }
        if let Some(detached) = self.detached_windows.get_mut(&id) {
            detached.size = size;
            let session = detached.session.clone();
            self.resize_detached_pty(&session, size);
        }

        Task::none()
    }
    pub(crate) fn reduce19_window_closed_3(&mut self, id: iced::window::Id) -> Task<Message> {
        self.window_closed(id)
    }
    pub(crate) fn reduce19_window_focus_changed_4(
        &mut self,
        id: iced::window::Id,
        focused: bool,
    ) -> Task<Message> {
        if self.windows.is_open(id) {
            let state = self.window_inputs.entry(id).or_default();
            if state.blurred == focused {
                state.end_composition();
            }
            state.blurred = !focused;
        }
        crate::window_input::trace_key(if focused {
            "focus: window focused, generation renewed"
        } else {
            "focus: window unfocused, generation renewed"
        });
        if focused {
            self.focused_window = Some(id);
            if let Some(index) = self.session_index_for_window(id)
                && self.sessions[index].status == SessionStatus::Ok
            {
                self.sessions[index].status = SessionStatus::Idle;
            }
            self.invalidate_sidebar();
        } else if self.focused_window == Some(id) {
            self.focused_window = None;
            self.invalidate_sidebar();
        }

        Task::none()
    }
}
