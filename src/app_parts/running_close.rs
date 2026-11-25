//! Confirmation before a close ends a running command.

use super::*;

const RUNNING_CLOSE_TITLE: &str = "Close running command?";
const RUNNING_CLOSE_BODY: &str = "Closing ends the command running in this terminal. Programs started with nohup, setsid or disown keep running.";

/// A close request held until the user confirms ending its running work.
#[derive(Debug, Clone)]
pub(crate) struct PendingRunningClose {
    /// The window that shows the confirmation.
    pub(crate) window: Option<iced::window::Id>,
    /// The active session and pane the request was made for, when it targets
    /// whichever is active.
    active: Option<(String, Option<String>)>,
    retry: Message,
}

impl App {
    /// Holds a pane, tab or window close that would end a running command
    /// and asks the user first. Returns whether `message` was held.
    pub(crate) fn hold_running_close(&mut self, message: &Message) -> bool {
        if self.running_close_confirmed {
            return false;
        }
        let origin = self
            .input_context
            .as_ref()
            .and_then(|origin| origin.window)
            .or(self.main_window);
        let active = self.sessions.get(self.active);
        let (window, running, target) = match message {
            Message::ClosePane => {
                let pane = active
                    .filter(|session| session.panes.len() > 1)
                    .and_then(|session| session.panes.get(session.active_pane));
                (
                    origin,
                    pane.is_some_and(|pane| self.pane_runs_command(pane)),
                    active
                        .zip(pane)
                        .map(|(session, pane)| (session.id.clone(), Some(pane.id.clone()))),
                )
            }
            Message::CloseActive => {
                let closes = self.sessions.len() > 1
                    || origin.is_some_and(|window| self.detached_windows.contains_key(&window));
                (
                    origin,
                    closes && active.is_some_and(|session| self.session_runs_command(session)),
                    active.map(|session| (session.id.clone(), None)),
                )
            }
            Message::ContextRowActivated(id, context_menu::RowKind::Close) => (
                self.main_window,
                self.sessions.len() > 1
                    && self
                        .sessions
                        .iter()
                        .find(|session| &session.id == id)
                        .is_some_and(|session| self.session_runs_command(session)),
                None,
            ),
            Message::CloseRequested(window) => (
                Some(*window),
                self.window_close_ends_running_work(*window),
                None,
            ),
            _ => return false,
        };
        if running {
            self.pending_running_close = Some(PendingRunningClose {
                window,
                active: target,
                retry: message.clone(),
            });
        }
        running
    }

    /// Whether closing `window` now would end a running command.
    fn window_close_ends_running_work(&self, window: iced::window::Id) -> bool {
        if !self.windows.is_open(window) {
            return false;
        }
        if self.close_terminates(window) {
            return self
                .sessions
                .iter()
                .any(|session| self.session_runs_command(session));
        }
        if let Some(detached) = self.detached_windows.get(&window) {
            return !detached.closing
                && self
                    .sessions
                    .iter()
                    .find(|session| session.id == detached.session)
                    .is_some_and(|session| self.session_runs_command(session));
        }
        self.main_window == Some(window)
            && self.sessions.iter().any(|session| {
                !session.detached && !session.pinned && self.session_runs_command(session)
            })
    }

    fn session_runs_command(&self, session: &Session) -> bool {
        session
            .panes
            .iter()
            .any(|pane| self.pane_runs_command(pane))
    }

    fn pane_runs_command(&self, pane: &Pane) -> bool {
        self.ptys
            .get(&pane.id)
            .is_some_and(|pty| pane.status == SessionStatus::Running || pty.has_foreground_job())
    }

    pub(crate) fn running_close_event(&mut self, event: ConfirmDialogEvent) -> Task<Message> {
        match event {
            ConfirmDialogEvent::Resolved(cause) => {
                let pending = self.pending_running_close.take();
                if !enzui_core::confirm::ConfirmDecision::from_cause(cause).is_confirmed() {
                    return Task::none();
                }
                let Some(pending) = pending else {
                    return Task::none();
                };
                let still_active = pending.active.as_ref().is_none_or(|(session, pane)| {
                    self.sessions.get(self.active).is_some_and(|active| {
                        &active.id == session
                            && pane.as_ref().is_none_or(|pane| {
                                active
                                    .panes
                                    .get(active.active_pane)
                                    .is_some_and(|active| &active.id == pane)
                            })
                    })
                });
                if !still_active {
                    return Task::none();
                }
                self.running_close_confirmed = true;
                let task = self.update_message(pending.retry);
                self.running_close_confirmed = false;
                task
            }
            ConfirmDialogEvent::FocusNext | ConfirmDialogEvent::FocusPrevious => {
                match (
                    &self.pending_running_close,
                    event.focus_operation::<Message>(true),
                ) {
                    (Some(_), Some(operation)) => iced::advanced::widget::operate(operation),
                    _ => Task::none(),
                }
            }
            ConfirmDialogEvent::CheckboxToggled(_) => Task::none(),
        }
    }

    /// The confirmation for `window`, when a held close belongs to it.
    pub(crate) fn running_close_overlay(
        &self,
        window: Option<iced::window::Id>,
    ) -> Option<Element<'_, Message>> {
        let pending = self.pending_running_close.as_ref()?;
        (pending.window == window).then(|| {
            self.confirm_overlay(
                &self.running_close_confirm_state,
                crate::i18n::label(RUNNING_CLOSE_TITLE),
                crate::i18n::label(RUNNING_CLOSE_BODY),
                crate::i18n::label("Close"),
                Message::RunningCloseEvent,
            )
        })
    }
}
