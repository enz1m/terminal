//! Terminal session resize methods.

use super::*;

impl App {
    pub(crate) fn resize_active_pty(&mut self) {
        self.resize_session_ptys(self.active);
    }

    pub(crate) fn resize_session_ptys(&mut self, index: usize) {
        let Some(session) = self.sessions.get(index) else {
            return;
        };
        if session.detached {
            let id = session.id.clone();
            if let Some(window) = self.detached_window_of(&id) {
                let size = self.detached_windows[&window].size;
                self.resize_detached_pty(&id, size);
            }
            return;
        }
        if index != self.active || !Self::usable_pty_extent(self.content_size) {
            return;
        }
        let geometries: Vec<_> = session
            .panes
            .iter()
            .filter_map(|pane| {
                self.pty_geometry_for(&pane.id, self.pane_text_size(&pane.id))
                    .map(|(rows, cols)| (pane.id.clone(), rows, cols))
            })
            .collect();
        if geometries.is_empty() {
            return;
        }
        model::perf::note_pty_reflow();
        #[cfg(test)]
        if let Some(active) = session.panes.get(session.active_pane)
            && let Some((_, rows, cols)) = geometries
                .iter()
                .find(|(id, _, _)| id.as_str() == active.id.as_str())
        {
            self.last_pty_resize = Some((*rows, *cols));
        }
        for (id, rows, cols) in geometries {
            if let Some(pty) = self.ptys.get(&id) {
                let _ = pty.resize(rows, cols);
            }
        }
    }
}
