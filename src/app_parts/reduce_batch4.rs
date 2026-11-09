//! Terminal reducer batch 4: resize, poll and scroll arms.

use super::*;

impl App {
    pub(crate) fn reduce4_content_resized_0(&mut self, size: iced::Size) -> Task<Message> {
        self.has_initial_resize = true;
        if (size.width - self.content_size.width).abs() > 0.5
            || (size.height - self.content_size.height).abs() > 0.5
        {
            self.content_size = size;
            // Resizing may fold away the menu trigger or change its presentation.
            self.titlebar_overflow_open = false;
            // A reflow invalidates column coordinates.
            self.clear_selection_on_reflow();
            self.resize_active_pty();
        }

        Task::none()
    }
    pub(crate) fn reduce4_poll_1(&mut self) -> Task<Message> {
        self.drain_ptys();

        Task::none()
    }
    pub(crate) fn reduce4_transcript_wheel_2(&mut self, rows: f32) -> Task<Message> {
        // A trackpad reports far less than a row per event; the residue
        // is banked so a slow gesture still moves eventually, and a
        // whole row is spent as soon as one is available.
        self.wheel_row_residue += rows;
        let whole = self.wheel_row_residue.trunc();
        self.wheel_row_residue -= whole;
        match scrollback::WheelRoute::resolve(self.active_wheel_target(), whole as i64) {
            scrollback::WheelRoute::Transcript(delta) => {
                return self.update(Message::ScrollTranscriptRows(delta));
            }
            // The alternate screen's wheel belongs to the application
            // running in it (xterm's alternateScroll). The transcript
            // offset is deliberately untouched here.
            scrollback::WheelRoute::Pty(bytes) => self.type_active_pty(&bytes),
            scrollback::WheelRoute::Nothing => {}
        }

        Task::none()
    }
    pub(crate) fn reduce4_scroll_transcript_rows_3(&mut self, delta: i64) -> Task<Message> {
        self.scroll_active_pane_rows(delta);

        Task::none()
    }
    pub(crate) fn reduce4_scroll_transcript_to_row_4(&mut self, row: usize) -> Task<Message> {
        self.scroll_active_pane_to_row(row);

        Task::none()
    }
}
