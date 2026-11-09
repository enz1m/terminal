//! Terminal pane and session helpers.

use super::*;

impl App {
    pub(crate) fn fresh_pane_id(&mut self) -> String {
        self.next_pane_seq += 1;
        format!("pane-{}", self.next_pane_seq)
    }

    /// Apply (or not) the capture-only demo seed to the active pane.
    ///
    /// When `enabled`, replaces the active pane's transcript with
    /// [`capture_demo_transcript`] and DROPS that pane's live PTY so the shell
    /// cannot overwrite the demo. When `!enabled`, does nothing — the live PTY
    /// and its prompt are left exactly as seeded. The `bool` is a parameter, not
    /// an env read, so `capture_seed::{seeds_when_enabled,is_inert_when_disabled}`
    /// can assert BOTH arms deterministically.
    pub(crate) fn apply_capture_seed(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        let target = self.sessions.get(self.active).and_then(|s| {
            let pane = s.panes.get(s.active_pane)?;
            Some((s.id.clone(), pane.id.clone()))
        });
        if let Some((session_id, pane_id)) = target {
            self.retire_pty(&session_id, &pane_id);
            if let Some(s) = self.sessions.get_mut(self.active)
                && let Some(p) = s.panes.get_mut(s.active_pane)
            {
                p.transcript = capture_demo_transcript(&pane_id);
            }
        }
    }

    pub(crate) fn active_pane_id(&self) -> Option<String> {
        let s = self.sessions.get(self.active)?;
        s.panes.get(s.active_pane).map(|p| p.id.clone())
    }

    pub(crate) fn session_index_for_window(&self, window: iced::window::Id) -> Option<usize> {
        if self.main_window == Some(window) {
            return self
                .sessions
                .get(self.active)
                .filter(|session| !session.detached)
                .map(|_| self.active);
        }
        let session_id = &self.detached_windows.get(&window)?.session;
        self.sessions
            .iter()
            .position(|session| &session.id == session_id)
    }

    pub(crate) fn session_is_focused(&self, index: usize) -> bool {
        self.focused_window
            .and_then(|window| self.session_index_for_window(window))
            == Some(index)
    }

    /// Select by position while recording stable-id MRU and acknowledging a
    /// completed command. Reordering changes positions, never this history.
    pub(crate) fn activate_session(&mut self, index: usize) {
        let Some(id) = self.sessions.get(index).map(|s| s.id.clone()) else {
            return;
        };
        self.active = index;
        if self.sessions[index].status == SessionStatus::Ok {
            self.sessions[index].status = SessionStatus::Idle;
        }
        self.session_mru.retain(|seen| seen != &id);
        self.session_mru.insert(0, id);
        self.invalidate_sidebar();
    }
}
