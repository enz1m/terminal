//! Terminal session-remove method.

use super::*;

impl App {
    pub(crate) fn remove_session_at(
        &mut self,
        index: usize,
        allow_last: bool,
    ) -> Option<SessionRowFocusTarget> {
        if (!allow_last && self.sessions.len() <= 1) || index >= self.sessions.len() {
            return None;
        }
        let active_id = self.sessions.get(self.active).map(|s| s.id.clone());
        let closing_active = self.active == index;
        let removed = self.sessions.remove(index);
        self.note_pinned_removal(&removed.id);
        self.empty_save_requested |= self.sessions.is_empty();
        self.uri_portal
            .cancel_session(&removed.id, enzui_core::portal::CancelReason::ParentClosed);
        // Closing the TAB owns closing its window. The mapping is
        // dropped here so the resulting close event cannot re-dock a session
        // that no longer exists, and the id is parked for `update`'s single
        // drain because this function can only return a focus id.
        if let Some(window) = self.detached_window_of(&removed.id) {
            self.detached_windows.remove(&window);
            self.orphaned_windows.push(window);
        }
        for pane in &removed.panes {
            self.retire_pty(&removed.id, &pane.id);
        }
        self.reconcile_operation_origins();
        self.title_overrides.remove(&removed.id);
        self.session_mru.retain(|id| id != &removed.id);
        if self
            .session_rename
            .as_ref()
            .is_some_and(|(id, _)| id == &removed.id)
        {
            self.session_rename = None;
        }

        if closing_active {
            let next_id = self
                .session_mru
                .iter()
                .find(|id| self.sessions.iter().any(|s| &s.id == *id))
                .cloned()
                .or_else(|| {
                    self.sessions
                        .get(index.min(self.sessions.len().saturating_sub(1)))
                        .map(|s| s.id.clone())
                });
            if let Some(next) = next_id
                && let Some(pos) = self.sessions.iter().position(|s| s.id == next)
            {
                self.activate_session(pos);
            }
        } else if let Some(active_id) = active_id
            && let Some(pos) = self.sessions.iter().position(|s| s.id == active_id)
        {
            self.active = pos;
        }

        self.resize_active_pty();
        self.invalidate_sidebar();
        self.sessions
            .get(self.active)
            .map(|session| SessionRowFocusTarget {
                tree: sidebar_session_tree::session_tree_id(session.kind),
                row: context_menu::session_row_id(&session.id),
            })
    }
}
