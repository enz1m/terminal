//! Terminal close methods.

use super::*;

impl App {
    pub(crate) fn close_active_pane(&mut self) {
        let owner = self
            .sessions
            .get(self.active)
            .filter(|session| session.panes.len() > 1)
            .and_then(|session| {
                let pane = session.panes.get(session.active_pane)?;
                let belongs_to_member = session
                    .group_members
                    .iter()
                    .any(|member| member.pane_ids.iter().any(|id| id == &pane.id));
                let original_panes = session
                    .panes
                    .iter()
                    .filter(|candidate| {
                        !session
                            .group_members
                            .iter()
                            .any(|member| member.pane_ids.iter().any(|id| id == &candidate.id))
                    })
                    .count();
                // Keep one pane owned by the target session. Otherwise a later
                // ungroup could move every surviving pane away and leave the
                // target's layout pointing at a pane it no longer owns.
                if !belongs_to_member && original_panes <= 1 {
                    return None;
                }
                Some(link_open::UriOwner {
                    session: session.id.clone(),
                    pane: pane.id.clone(),
                })
            });
        let Some(owner) = owner else {
            return; // Cannot close the last pane; close the session instead.
        };
        self.uri_portal
            .cancel_owner(&owner, enzui_core::portal::CancelReason::ParentClosed);
        let leaf_removed = self
            .sessions
            .get_mut(self.active)
            .is_some_and(|s| s.layout.remove_leaf(&owner.pane));
        if leaf_removed {
            self.retire_pty(&owner.session, &owner.pane);
            if let Some(s) = self.sessions.get_mut(self.active) {
                s.panes.retain(|p| p.id != owner.pane);
                s.group_members
                    .retain_mut(|member| member.forget_closed_pane(&owner.pane));
                if s.active_pane >= s.panes.len() {
                    s.active_pane = s.panes.len().saturating_sub(1);
                }
                s.pane_grid.replace_tree(s.layout.pane_grid_tree());
            }
            self.pane_feedback = None;
            // The survivors reclaim the closed pane's width and re-wrap.
            self.clear_selection_on_reflow();
            self.resize_active_pty();
        }
        self.reconcile_operation_origins();
    }

    /// Close the selected session, then select and focus the most-recently
    /// used surviving stable id.
    pub(crate) fn close_active_session(&mut self) -> Option<SessionRowFocusTarget> {
        self.close_session_at(self.active)
    }
}
