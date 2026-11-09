//! Terminal reducer batch 7: detached scroll arms.

use super::*;

impl App {
    pub(crate) fn reduce7_detached_transcript_wheel_0(
        &mut self,
        window: iced::window::Id,
        pane: String,
        rows: f32,
    ) -> Task<Message> {
        let Some((session_id, target, fallback)) = self.detached_wheel_target(window, &pane) else {
            return Task::none();
        };
        let whole = {
            let Some(detached) = self.detached_windows.get_mut(&window) else {
                return Task::none();
            };
            let viewport = detached
                .viewports
                .entry(pane.clone())
                .or_insert_with(|| DetachedViewport::new(fallback));
            viewport.wheel_row_residue += rows;
            let whole = viewport.wheel_row_residue.trunc();
            viewport.wheel_row_residue -= whole;
            whole as i64
        };
        match scrollback::WheelRoute::resolve(target, whole) {
            scrollback::WheelRoute::Transcript(delta) => {
                self.scroll_detached_pane_rows(window, &pane, delta);
                return self.detached_viewport_scope(window);
            }
            scrollback::WheelRoute::Pty(bytes) => {
                // PTY writes update session status rendered by the main sidebar.
                self.type_pane_pty(&session_id, &pane, &bytes);
            }
            scrollback::WheelRoute::Nothing => {
                return self.detached_viewport_scope(window);
            }
        }

        Task::none()
    }
    pub(crate) fn reduce7_detached_scroll_transcript_rows_1(
        &mut self,
        window: iced::window::Id,
        pane: String,
        delta: i64,
    ) -> Task<Message> {
        self.scroll_detached_pane_rows(window, &pane, delta);
        self.detached_viewport_scope(window)
    }
    pub(crate) fn reduce7_detached_scroll_transcript_to_row_2(
        &mut self,
        window: iced::window::Id,
        pane: String,
        row: usize,
    ) -> Task<Message> {
        self.scroll_detached_pane_to_row(window, &pane, row);
        self.detached_viewport_scope(window)
    }
    pub(crate) fn reduce7_detached_jump_to_latest_3(
        &mut self,
        window: iced::window::Id,
        pane: String,
    ) -> Task<Message> {
        self.jump_detached_pane_to_latest(window, &pane);
        self.detached_viewport_scope(window)
    }
}
