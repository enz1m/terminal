//! Terminal PTY resize methods.

use super::*;

impl App {
    /// Whether a main-window pane follows its live output (at the end of its
    /// scrollback, nothing selected or found), and if so hands its PTY reader
    /// `screen` to write directly, below `rows_above` rows of scrollback (the
    /// ones shown under the titlebar); otherwise takes it back. Returns
    /// whether the view must write the screen itself.
    pub(crate) fn live(
        &self,
        pane: &model::Pane,
        screen: &enzui_terminal::TerminalScreen,
        rows_above: u16,
        at_end: bool,
        quiet: bool,
        focused: bool,
    ) -> bool {
        let Some(pty) = self.ptys.get(&pane.id) else {
            return true;
        };
        let live = at_end && quiet;
        pty.set_live(
            live.then(|| (screen, focused, self.settings.resolved_cursor_style())),
            rows_above,
        );
        !live
    }

    pub(crate) fn usable_pty_extent(size: iced::Size) -> bool {
        size.width.is_finite() && size.width > 0.0 && size.height.is_finite() && size.height > 0.0
    }

    // Rendering may estimate a default grid; a live PTY must wait for a usable allocation.
    pub(crate) fn measured_pty_geometry(&self, text: iced::Size) -> Option<(u16, u16)> {
        Self::usable_pty_extent(text).then(|| {
            let geometry = self.geometry();
            (
                geometry.rows_for(text.height),
                geometry.cols_for(text.width),
            )
        })
    }

    /// The app's own estimate of a pane's grid, `(rows, cols)`, from its
    /// text rect in whichever window paints it.
    fn pane_estimate(&self, pane_id: &str) -> Option<(u16, u16)> {
        let session = self
            .sessions
            .iter()
            .find(|session| session.panes.iter().any(|pane| pane.id == pane_id))?;
        let text = if session.detached {
            let window = self.detached_window_of(&session.id)?;
            let pane = session.panes.iter().find(|pane| pane.id == pane_id)?;
            self.input_text_size(window, session, pane)
        } else {
            self.pane_text_size(pane_id)
        };
        self.measured_pty_geometry(text)
    }

    /// The grid a pane's PTY gets, `(rows, cols)`: what its view reported
    /// it fits while that report is current, else the estimate for `text`.
    pub(crate) fn pty_geometry_for(&self, pane_id: &str, text: iced::Size) -> Option<(u16, u16)> {
        let estimate = self.measured_pty_geometry(text)?;
        Some(match self.terminal_fit.get(pane_id) {
            Some(&((cols, rows), (est_cols, est_rows))) if (est_rows, est_cols) == estimate => {
                (rows, cols)
            }
            _ => estimate,
        })
    }

    /// A pane's view reported the grid it fits. When that differs from what
    /// the PTY was given, resize the PTY to the view's grid.
    pub(crate) fn terminal_fit_reported(&mut self, pane: String, cols: u16, rows: u16) {
        let Some((est_rows, est_cols)) = self.pane_estimate(&pane) else {
            return;
        };
        let previous = self
            .terminal_fit
            .insert(pane.clone(), ((cols, rows), (est_cols, est_rows)));
        if previous.map(|(fit, _)| fit) == Some((cols, rows)) {
            return;
        }
        if let Some(pty) = self.ptys.get(&pane) {
            let _ = pty.resize(rows, cols);
        }
    }

    pub(crate) fn resize_detached_pty(&mut self, session_id: &str, _size: iced::Size) {
        let Some(window) = self.detached_window_of(session_id) else {
            return;
        };
        if !self
            .detached_windows
            .get(&window)
            .is_some_and(|window| !window.closing && Self::usable_pty_extent(window.size))
        {
            return;
        }
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
        else {
            return;
        };
        let geometries: Vec<_> = session
            .panes
            .iter()
            .filter_map(|pane| {
                self.pty_geometry_for(&pane.id, self.input_text_size(window, session, pane))
                    .map(|(rows, cols)| (pane.id.clone(), rows, cols))
            })
            .collect();
        for (id, rows, cols) in geometries {
            #[cfg(test)]
            {
                self.last_pty_resize = Some((rows, cols));
            }
            if let Some(pty) = self.ptys.get(&id) {
                let _ = pty.resize(rows, cols);
            }
        }
    }
}
