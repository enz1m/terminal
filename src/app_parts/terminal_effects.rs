//! Terminal effect-apply method.

use super::*;

impl App {
    pub(crate) fn apply_terminal_effects(
        &mut self,
        session_index: usize,
        pane_index: usize,
        mut effects: emulator::TerminalEffects,
    ) {
        let mut sidebar_dirty = false;
        if !effects.shell_events.is_empty() {
            sidebar_dirty = self.apply_shell_events(
                session_index,
                pane_index,
                std::mem::take(&mut effects.shell_events),
            );
            #[cfg(test)]
            {
                effects.commands_started.clear();
                effects.command_finished = None;
            }
        }
        #[cfg(test)]
        let session_is_focused = self.session_is_focused(session_index);
        let Some(session) = self.sessions.get_mut(session_index) else {
            return;
        };
        #[cfg(test)]
        if !effects.commands_started.is_empty() {
            for command in effects.commands_started {
                session.in_app_history.record(command);
            }
            if let Some(pane) = session.panes.get_mut(pane_index) {
                pane.status = SessionStatus::Running;
            }
            sidebar_dirty = session.status != SessionStatus::Running;
            session.status = SessionStatus::Running;
        }
        if let Some(title) = effects.title {
            let title = title.trim();
            if !title.is_empty()
                && !self.title_overrides.contains(&session.id)
                && session.title != title
            {
                session.title = title.to_owned();
                sidebar_dirty = true;
            }
        }
        // Every prompt reports the directory; the branch may have moved under
        // an unchanged one (a checkout), so each report looks it up again.
        let reported_cwd = effects.cwd.is_some() || effects.generated_cwd.is_some();
        if let Some(pane) = session.panes.get_mut(pane_index)
            && let Some(new_cwd) = effects
                .generated_cwd
                .map(|cwd| cwd.location(pane.endpoint.kind(), pane.endpoint.remote_hint()))
                .or_else(|| {
                    effects.cwd.map(|cwd| {
                        display_reported_cwd(
                            &cwd,
                            pane.endpoint.kind(),
                            pane.endpoint.remote_hint(),
                        )
                    })
                })
            && pane.cwd != new_cwd
        {
            pane.cwd = new_cwd;
            if pane_index == session.active_pane {
                sidebar_dirty = true;
            }
        }
        // A prompt means the shell has read its startup files.
        #[cfg(unix)]
        if reported_cwd
            && let Some(pane) = session.panes.get(pane_index)
            && let Some(pty) = self.ptys.get_mut(&pane.id)
        {
            pty.shell_started();
        }
        if reported_cwd
            && let Some(pane) = session.panes.get(pane_index)
            && pane.endpoint == crate::pane_endpoint::PaneEndpoint::Local
            && let Some(dir) = crate::escape_policy::resolvable_local_base(&pane.cwd)
        {
            self.git_lookups_due.push((pane.id.clone(), dir));
        }
        #[cfg(test)]
        if let Some(exit_code) = effects.command_finished {
            let tracked_command = session
                .panes
                .get(pane_index)
                .is_some_and(|pane| pane.status == SessionStatus::Running);
            if tracked_command {
                session.in_app_history.finish_newest_pending(exit_code);
                if let Some(pane) = session.panes.get_mut(pane_index) {
                    pane.status = SessionStatus::Idle;
                }
                let old_status = session.status;
                if session.connection_error.is_none() {
                    session.status = if session
                        .panes
                        .iter()
                        .any(|pane| pane.status == SessionStatus::Running)
                    {
                        SessionStatus::Running
                    } else if session_is_focused {
                        SessionStatus::Idle
                    } else {
                        SessionStatus::Ok
                    };
                }
                if session.status != old_status {
                    sidebar_dirty = true;
                }
            }
        }
        // BEL remains a terminal bell, not a second persistent notification
        // model. `bell_count` is deliberately drained and otherwise ignored.
        let _ = effects.bell_count;
        if sidebar_dirty {
            self.invalidate_sidebar();
        }
    }
}
