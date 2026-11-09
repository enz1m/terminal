//! Terminal shell-event apply method.

use super::*;

impl App {
    pub(crate) fn apply_shell_events(
        &mut self,
        session_index: usize,
        pane_index: usize,
        events: std::collections::VecDeque<shell_history::ShellEvent>,
    ) -> bool {
        use shell_history::ShellEvent;
        let focused = self.session_is_focused(session_index);
        let Some(session) = self.sessions.get_mut(session_index) else {
            return false;
        };
        let Some(pane) = session.panes.get(pane_index) else {
            return false;
        };
        let pane_id = pane.id.clone();
        let mut dirty = false;
        for event in events {
            self.shell_activity.event(&pane_id, &event);
            session.in_app_history.apply_shell_event(&event);
            let old_status = session.status;
            let unavailable = matches!(&event, ShellEvent::Unavailable);
            match &event {
                ShellEvent::Started { .. } => {
                    session.panes[pane_index].status = SessionStatus::Running;
                    session.status = SessionStatus::Running;
                }
                ShellEvent::Finished { key: Some(_), .. } | ShellEvent::Unavailable => {
                    session.panes[pane_index].status = SessionStatus::Idle;
                    if session.connection_error.is_none() {
                        session.status = if session
                            .panes
                            .iter()
                            .any(|pane| pane.status == SessionStatus::Running)
                        {
                            SessionStatus::Running
                        } else if focused || unavailable {
                            SessionStatus::Idle
                        } else {
                            SessionStatus::Ok
                        };
                    }
                }
                ShellEvent::Finished { key: None, .. } => {}
            }
            for member in &mut session.group_members {
                member.apply_shell_event(&pane_id, &event, &session.panes);
            }
            dirty |= old_status != session.status;
        }
        dirty
    }
}
