//! One terminal's older output, continuing under the floating titlebar.
//!
//! The PTY's grid starts at the bar's bottom edge. The rows that scrolled off
//! just before the grid's window are drawn above it, under the bar, fading
//! into the canvas towards the window's top: output scrolls on under the bar
//! instead of ending at a strip of empty canvas. They are the top rows of the
//! pane's own screen, drawn and faded above the view's bounds by the same
//! primitive (`rows_above`, `fade_rows_above`), so they take no input, no PTY
//! rows and no second draw or layer. Tiled panes and a full-screen program
//! (the alternate screen) show none.

use super::*;

impl App {
    /// The height above the grid's first row, from the content pane's top:
    /// the bar's band plus the grid's own top inset.
    fn under_titlebar_height(&self) -> f32 {
        self.grid_top_inset() + self.terminal_inset()
    }

    /// How many whole older rows fit above the active pane's first row:
    /// none unless it is the session's one pane, on the primary screen.
    pub(crate) fn under_titlebar_rows(&self) -> u16 {
        let Some(session) = self.sessions.get(self.active) else {
            return 0;
        };
        if session.panes.len() != 1
            || self.in_own_window(self.active)
            || session.panes.first().is_none_or(|pane| pane.alt_screen)
        {
            return 0;
        }
        let cell = self.geometry().cell_height;
        if cell <= 0.0 {
            return 0;
        }
        (self.under_titlebar_height() / cell)
            .floor()
            .clamp(0.0, f32::from(u16::MAX)) as u16
    }
}

#[cfg(test)]
mod tests {
    use crate::grid_view::GridPointerTarget;
    use crate::model::{LineTone, TranscriptLine, TranscriptRow};

    #[derive(Clone, Debug, PartialEq)]
    enum Probe {
        Pressed(GridPointerTarget),
        Other,
        Fit(u16),
    }

    fn row(text: &str) -> TranscriptRow {
        TranscriptRow::new(TranscriptLine::monochrome(
            text,
            text,
            LineTone::Default,
            false,
        ))
    }
}
