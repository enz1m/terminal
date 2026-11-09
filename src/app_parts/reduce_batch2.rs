//! Terminal reducer batch 2: key-press arms.

use super::*;

impl App {
    #[cfg(test)]
    pub(crate) fn reduce2_window_key_pressed_0(
        &mut self,
        window: iced::window::Id,
        key: keyboard::Key,
        mods: Modifiers,
        captured: bool,
    ) -> Task<Message> {
        if let Some(index) = self.session_index_for_window(window)
            && let Some(session) = self.sessions.get(index)
            && let Some(pane) = session.panes.get(session.active_pane)
            && let Some(origin) = self.input_origin(&session.id, &pane.id)
        {
            return self.reduce_pane_input(
                origin,
                Message::KeyPressed {
                    key,
                    mods,
                    captured,
                },
            );
        }
        self.mods = mods;
        if Some(window) == self.main_window {
            return self.update(Message::KeyPressed {
                key,
                mods,
                captured,
            });
        }
        let Some(detached) = self
            .detached_windows
            .get(&window)
            .filter(|detached| !detached.closing)
        else {
            // A key queued by a window that has closed/re-docked is
            // stale. Never reinterpret it as main-window input.
            return Task::none();
        };
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.id == detached.session)
        else {
            return Task::none();
        };
        let Some(pane) = session.panes.get(session.active_pane) else {
            return Task::none();
        };
        self.update(Message::DetachedKeyPressed {
            window,
            pane: pane.id.clone(),
            key,
            mods,
            captured,
        })
    }
    pub(crate) fn reduce2_detached_key_pressed_1(
        &mut self,
        window: iced::window::Id,
        pane: String,
        key: keyboard::Key,
        mods: Modifiers,
        captured: bool,
    ) -> Task<Message> {
        self.mods = mods;
        let Some((session_index, pane_index)) = self.detached_target(window, &pane) else {
            return Task::none();
        };
        if !captured && let Some(action) = self.shortcut_action_for_key(&key, mods) {
            let direction = match action.as_str() {
                "pane.split-horizontal" => Some(pane_layout::SplitDirection::Horizontal),
                "pane.split-vertical" => Some(pane_layout::SplitDirection::Vertical),
                _ => None,
            };
            if let Some(direction) = direction {
                return self.update(Message::DetachedSplitPane {
                    window,
                    pane,
                    direction,
                });
            }
        }
        let session_id = self.sessions[session_index].id.clone();
        // History overlay: Escape closes it before the key can reach the
        // PTY, so dismissing history never also sends ESC to the shell.
        if self
            .detached_windows
            .get(&window)
            .is_some_and(|detached| detached.history_open)
            && let keyboard::Key::Named(keyboard::key::Named::Escape) = key
        {
            return self.update(Message::DetachedCloseHistory(window));
        }
        let target = &self.sessions[session_index].panes[pane_index];
        let alt_screen = target.alt_screen;
        let application_cursor_keys = target.application_cursor_keys;
        if !captured
            && !mods.control()
            && !mods.alt()
            && !mods.logo()
            && !alt_screen
            && let keyboard::Key::Named(named) = &key
            && matches!(
                named,
                keyboard::key::Named::PageUp | keyboard::key::Named::PageDown
            )
        {
            let page = self
                .detached_scroll_geometry(window, &pane)
                .map_or(1, |geometry| geometry.page_rows()) as i64;
            let delta = if matches!(named, keyboard::key::Named::PageUp) {
                -page
            } else {
                page
            };
            return self.update(Message::DetachedScrollTranscriptRows {
                window,
                pane,
                delta,
            });
        }
        if let Some(bytes) =
            self.pty_bytes_for_application_mode(captured, &key, mods, application_cursor_keys)
        {
            self.type_pane_pty(&session_id, &pane, &bytes);
        }

        Task::none()
    }
}
