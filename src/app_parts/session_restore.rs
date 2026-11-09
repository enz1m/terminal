//! Terminal session-restore methods.

use super::*;

impl App {
    // Loading a workspace never authorizes a process or credential request.
    pub(crate) fn install_restored_sessions(
        &mut self,
        state: &session_store::SessionState,
        restore_windows: bool,
    ) -> bool {
        if (state.sessions.is_empty() && state.active_session_id.is_some())
            || (!state.sessions.is_empty() && !state.is_sound())
        {
            return false;
        }
        // A live PTY must leave through its exact owner route before the pane
        // that addresses it disappears. Adoption then waits: replacing the
        // metadata while a child is still being torn down would leave custody
        // the new workspace can no longer name.
        self.retire_every_live_pty();
        self.poll_teardown_debts();
        if !self.teardown_debts.is_empty() {
            self.restore_held_by_teardown = true;
            return false;
        }
        self.restore_held_by_teardown = false;
        self.cancel_credential_operations();
        self.secure_pins.adopt(state);
        self.paste_requests.clear();
        let old_panes: Vec<_> = self
            .sessions
            .iter()
            .flat_map(|session| &session.panes)
            .map(|pane| pane.id.clone())
            .collect();
        for pane in old_panes {
            self.invalidate_pane_coordinates(&pane);
        }
        for origin in self.input_origins.values() {
            origin
                .generation
                .retire(enzui_core::cancellation::CancelCause::Disconnected);
        }
        self.input_origins.clear();
        // No `ptys.clear()` here: retirement above is the only route a live PTY
        // may leave by, and clearing the map would reach a field drop instead.
        self.restored_outputs.clear();
        self.finished_outputs.clear();
        self.restored_grid_sizes.clear();
        self.durable_titles.clear();
        self.pending_endpoint_restores.clear();
        self.pending_detached_restores.clear();
        // A child still starting belongs to a pane this restore replaces.
        self.pending_pane_spawns.clear();
        self.sessions = state
            .sessions
            .iter()
            .map(|saved| {
                let panes: Vec<Pane> = saved
                    .panes
                    .iter()
                    .map(|pane| {
                        let output: Arc<pinned_output::PinnedOutput> = Arc::default();
                        let mut restored = scripted_pane(&pane.id, "", &pane.cwd);
                        restored.transcript = output.render(&pane.id);
                        restored.status = SessionStatus::Warning;
                        self.restored_outputs.insert(pane.id.clone(), output);
                        restored.title = pane.title.clone();
                        restored.endpoint = pane.endpoint.clone();
                        restored
                    })
                    .collect();
                let active_pane = panes
                    .iter()
                    .position(|pane| pane.id == saved.active_pane_id)
                    .unwrap_or(0);
                Session {
                    id: saved.id.clone(),
                    title: saved.title.clone(),
                    subtitle: saved.subtitle.clone(),
                    icon: match saved.kind {
                        SessionKind::Local => "terminal",
                        SessionKind::Ssh => "server",
                    }
                    .into(),
                    status: SessionStatus::Warning,
                    kind: saved.kind,
                    ssh_host_id: saved.ssh_host_id.clone(),
                    connection_error: None,
                    pinned: saved.pinned,
                    detached: saved.detached,
                    panes,
                    active_pane,
                    pane_grid: saved.layout.pane_grid_state(),
                    layout: saved.layout.clone(),
                    in_app_history: model::CommandHistory::new(),
                    group_members: saved
                        .group_members
                        .iter()
                        .map(session_store::SavedGroupMember::restore)
                        .collect(),
                }
            })
            .collect();
        self.title_overrides = state
            .sessions
            .iter()
            .filter(|session| session.title_overridden)
            .map(|session| session.id.clone())
            .collect();
        self.active = state
            .active_session_id
            .as_ref()
            .and_then(|id| self.sessions.iter().position(|session| &session.id == id))
            .unwrap_or(0);
        self.next_pane_seq = self
            .sessions
            .iter()
            .flat_map(|session| &session.panes)
            .filter_map(|pane| pane.id.strip_prefix("pane-")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);

        if restore_windows {
            self.pending_detached_restores = self
                .sessions
                .iter()
                .filter(|session| session.detached)
                .map(|session| session.id.clone())
                .collect();
        }
        for index in 0..self.sessions.len() {
            let history = self.restore_output_history(&self.sessions[index].panes);
            self.sessions[index].in_app_history = history;
            pinned_sessions::restore_member_histories(
                &mut self.sessions[index].group_members,
                &self.restored_outputs,
            );
        }
        true
    }
}
