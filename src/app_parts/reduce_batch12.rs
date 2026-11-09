//! Terminal reducer batch 12: pane focus and rename arms.

use super::*;

impl App {
    pub(crate) fn reduce12_focus_pane_0(&mut self, id: String) -> Task<Message> {
        let index = self
            .sessions
            .get(self.active)
            .and_then(|session| session.panes.iter().position(|pane| pane.id == id));
        if let Some(index) = index {
            self.focus_pane_index(index);
            // Clear only the pressed pane's range; a sibling keeps its
            // own selection.
            let _ = self.selections.remove(&id);
            self.grid_pointer = GridPointerProbe::default();
        }

        Task::none()
    }
    pub(crate) fn reduce12_pane_grid_event_1(
        &mut self,
        event: enzui_widgets::EnzPaneGridEvent,
    ) -> Task<Message> {
        // Resizing a PTY reflows and reprojects its whole terminal
        // surface; doing that once per mouse pixel caused stacked
        // panes to flicker, so the framework publishes one explicit
        // release boundary and keyboard steps stay discrete commits.
        //
        // The persisted `layout` ratios — the source the
        // painted terminal geometry (columns, rows, scroll window)
        // reads through `pane_surface_size` — are committed on the same
        // boundary. While the pointer drag is live the widget shows the
        // seam move, but the text stays wrapped and windowed for the
        // geometry the PTY actually holds. Following the transient
        // ratio per pixel reprojected un-reflowed content: a narrowing
        // pane sprouted `…` continuation cues, and a stacked pane's
        // bottom-anchored window jumped rows.
        let commit_pty = matches!(
            event,
            enzui_widgets::EnzPaneGridEvent::PointerResizeFinished
                | enzui_widgets::EnzPaneGridEvent::Key(_)
        );
        if let Some(session) = self.sessions.get_mut(self.active) {
            session.pane_grid.update(event);
            if commit_pty {
                let ratios = session.pane_grid.ratios();
                session.layout.apply_pane_grid_ratios(&ratios);
                // Committing the seam re-wraps both adjacent panes, so
                // a column-anchored selection range is stale after the
                // reflow.
                self.clear_selection_on_reflow();
                self.resize_active_pty();
            }
        }

        Task::none()
    }
    pub(crate) fn reduce12_begin_pane_rename_2(&mut self) -> Task<Message> {
        // Seed the draft with the current pane title so the field opens
        // pre-filled (the user edits rather than retypes).
        let current = self
            .sessions
            .get(self.active)
            .and_then(|s| s.panes.get(s.active_pane))
            .map(|p| p.title.clone())
            .unwrap_or_default();
        self.pane_rename_draft = Some(current);

        Task::none()
    }
    pub(crate) fn reduce12_pane_rename_edited_3(&mut self, v: String) -> Task<Message> {
        if let Some(d) = self.pane_rename_draft.as_mut() {
            *d = v;
        }

        Task::none()
    }
}
