//! Terminal detached-pane methods.

use super::*;

impl App {
    /// Resolve a detached callback's stable coordinates. Requiring the pane to
    /// still be the mapped session's active pane makes messages from an old
    /// view inert after a pane switch, while requiring the window mapping makes
    /// callbacks racing close/re-dock inert.
    pub(crate) fn detached_target(
        &self,
        window: iced::window::Id,
        pane_id: &str,
    ) -> Option<(usize, usize)> {
        let detached = self.detached_windows.get(&window)?;
        if detached.closing {
            return None;
        }
        let session_index = self
            .sessions
            .iter()
            .position(|session| session.id == detached.session)?;
        let session = &self.sessions[session_index];
        let pane_index = session.panes.iter().position(|pane| pane.id == pane_id)?;
        (session.active_pane == pane_index).then_some((session_index, pane_index))
    }

    /// Scroll geometry derived from the detached window's own viewport, never
    /// from the main window. This is shared by paint, wheel, page and drag.
    pub(crate) fn detached_scroll_geometry(
        &self,
        window: iced::window::Id,
        pane_id: &str,
    ) -> Option<scrollback::ScrollGeometry> {
        let (session_index, pane_index) = self.detached_target(window, pane_id)?;
        let session = &self.sessions[session_index];
        let pane = &session.panes[pane_index];
        let text_height = self.input_text_size(window, session, pane).height;
        Some(scrollback::ScrollGeometry::new(
            pane.transcript.len(),
            self.geometry().rows_for(text_height) as usize,
            self.geometry().cell_height,
        ))
    }

    pub(crate) fn detached_wheel_target(
        &self,
        window: iced::window::Id,
        pane_id: &str,
    ) -> Option<(String, scrollback::WheelTarget, scrollback::ScrollPosition)> {
        let (session_index, pane_index) = self.detached_target(window, pane_id)?;
        let session = &self.sessions[session_index];
        let pane = &session.panes[pane_index];
        Some((
            session.id.clone(),
            scrollback::WheelTarget {
                alt_screen: pane.alt_screen,
                application_cursor_keys: pane.application_cursor_keys,
                mouse_protocol: pane.mouse_protocol,
                mouse_shift: self.mods.shift(),
                mouse_cell: self.mouse_last_cell,
            },
            pane.scroll,
        ))
    }

    pub(crate) fn detached_viewport_scope(&self, window: iced::window::Id) -> Task<Message> {
        if self
            .detached_windows
            .get(&window)
            .is_some_and(|detached| !detached.closing)
        {
            iced::window::invalidate_only(window)
        } else {
            Task::none()
        }
    }
}
