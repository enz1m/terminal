//! Terminal scroll-detached methods.

use super::*;

impl App {
    pub(crate) fn scroll_detached_pane_rows(
        &mut self,
        window: iced::window::Id,
        pane_id: &str,
        delta: i64,
    ) {
        let Some((session_index, pane_index)) = self.detached_target(window, pane_id) else {
            return;
        };
        if self.sessions[session_index].panes[pane_index].alt_screen {
            return;
        }
        let fallback = self.sessions[session_index].panes[pane_index].scroll;
        let Some(geometry) = self.detached_scroll_geometry(window, pane_id) else {
            return;
        };
        let Some(detached) = self.detached_windows.get_mut(&window) else {
            return;
        };
        detached
            .viewports
            .entry(pane_id.to_owned())
            .or_insert_with(|| DetachedViewport::new(fallback))
            .scroll
            .scroll_by_rows(delta, geometry);
    }

    pub(crate) fn scroll_detached_pane_to_row(
        &mut self,
        window: iced::window::Id,
        pane_id: &str,
        row: usize,
    ) {
        let Some((session_index, pane_index)) = self.detached_target(window, pane_id) else {
            return;
        };
        if self.sessions[session_index].panes[pane_index].alt_screen {
            return;
        }
        let fallback = self.sessions[session_index].panes[pane_index].scroll;
        let Some(geometry) = self.detached_scroll_geometry(window, pane_id) else {
            return;
        };
        let Some(detached) = self.detached_windows.get_mut(&window) else {
            return;
        };
        detached
            .viewports
            .entry(pane_id.to_owned())
            .or_insert_with(|| DetachedViewport::new(fallback))
            .scroll
            .set_top_row(row, geometry);
    }

    pub(crate) fn jump_detached_pane_to_latest(&mut self, window: iced::window::Id, pane_id: &str) {
        let Some((session_index, pane_index)) = self.detached_target(window, pane_id) else {
            return;
        };
        if self.sessions[session_index].panes[pane_index].alt_screen {
            return;
        }
        let fallback = self.sessions[session_index].panes[pane_index].scroll;
        let Some(detached) = self.detached_windows.get_mut(&window) else {
            return;
        };
        let viewport = detached
            .viewports
            .entry(pane_id.to_owned())
            .or_insert_with(|| DetachedViewport::new(fallback));
        viewport.scroll.jump_to_tail();
        viewport.wheel_row_residue = 0.0;
    }
}
