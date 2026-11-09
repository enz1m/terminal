//! Notices about the app's own condition (a refused write, a failed save, a
//! pane restored without its process) as toasts in the window's
//! bottom-trailing corner, over the terminal rather than above it, so the
//! grid keeps its rows whether one shows or not.
//!
//! Each notice can be dismissed. A dismissed notice stays hidden while its
//! text is unchanged; one with new text, or one whose cause clears and comes
//! back, shows again.

use crate::ssh_credentials::NoticeTone;
use crate::{App, Message, pane_endpoint::PaneEndpoint};
use enzui_widgets::notice_stack::{EnzNotice, enz_notice_stack};
use enzui_widgets::toast::ToastTone;
use enzui_widgets::toast_host::{ToastOverlayPlacement, enz_toast_overlay};
use iced::{Element, Length};

/// One notice before it becomes a toast: its identity is its kind and text.
struct Notice {
    id: String,
    title: String,
    description: Option<String>,
    tone: ToastTone,
    action: Option<(String, Message)>,
}

impl Notice {
    fn new(kind: &str, title: impl Into<String>, tone: ToastTone) -> Self {
        let title = title.into();
        Self {
            id: format!("{kind}\u{1f}{title}"),
            title,
            description: None,
            tone,
            action: None,
        }
    }

    fn description(mut self, description: impl Into<String>) -> Self {
        let description = description.into();
        self.id.push('\u{1f}');
        self.id.push_str(&description);
        self.description = Some(description);
        self
    }

    fn action(mut self, label: &'static str, message: Message) -> Self {
        self.action = Some((crate::i18n::label(label).to_owned(), message));
        self
    }
}

impl App {
    /// The main window's notices, in the order they stack.
    fn main_notices(&self) -> Vec<Notice> {
        let mut notices = Vec::new();
        if let Some(feedback) = &self.pane_feedback {
            notices.push(Notice::new("pane", feedback.clone(), ToastTone::Error));
        }
        if let Some(error) = self
            .sessions
            .get(self.active)
            .and_then(|session| session.connection_error.as_deref())
        {
            notices.push(Notice::new(
                "connection",
                crate::i18n::text(error),
                ToastTone::Error,
            ));
        }
        if let Some(error) = self.session_load_error.as_deref() {
            let notice = Notice::new("load", crate::i18n::text(error), ToastTone::Error);
            notices.push(if self.sessions.is_empty() {
                notice
                    .description(crate::i18n::label(
                        "Saved sessions are protected; no process was started.",
                    ))
                    .action("Reset saved sessions", Message::ResetSavedSessions)
            } else {
                notice.description(crate::i18n::label(
                    "A safe workspace was opened. Saved session data is protected and will not be overwritten.",
                ))
            });
        }
        if let Some(error) = &self.session_save_error {
            notices.push(
                Notice::new(
                    "save",
                    crate::i18n::label("Session state could not be saved"),
                    ToastTone::Error,
                )
                .description(error.clone()),
            );
        }
        notices.extend(self.preference_notices());
        notices.extend(self.pinned_storage_notice());
        notices.extend(self.restored_pane_notice());
        notices.extend(self.close_veto_notice());
        notices
    }

    /// Settings load/save and connection-profile save failures, and keyring
    /// outcomes from the SSH manager: app conditions that used to be banners
    /// inside Settings and the SSH host list. OpenSSH discovery problems stay
    /// beside the list's Reload button.
    fn preference_notices(&self) -> Vec<Notice> {
        let mut notices = Vec::new();
        if let Some(summary) = self.settings_load_error.as_deref() {
            notices.push(
                Notice::new(
                    "settings-load",
                    crate::i18n::text(summary),
                    ToastTone::Error,
                )
                .description(crate::i18n::label(
                    "Default settings are in use. Saved settings are protected and will not be overwritten.",
                )),
            );
        }
        if let Some(error) = self.shortcuts.save_error() {
            notices.push(Notice::new(
                "shortcuts-save",
                crate::i18n::format(
                    crate::i18n::localizer(),
                    "Could not save settings: {error}",
                    &[("error", &(error))],
                ),
                ToastTone::Error,
            ));
        }
        if let Some(error) = self.ssh_save_error.as_ref() {
            notices.push(Notice::new(
                "ssh-save",
                crate::i18n::text(error),
                ToastTone::Error,
            ));
        }
        if let Some(notice) = self.ssh_credential_notice.as_ref() {
            notices.push(Notice::new(
                "credential",
                notice.text.clone(),
                match notice.tone {
                    NoticeTone::Info => ToastTone::Info,
                    NoticeTone::Danger => ToastTone::Error,
                },
            ));
        }
        notices
    }

    /// A detached window's notices: its own refused input, and a close veto
    /// that has no main window left to show it.
    fn detached_notices(&self, window: iced::window::Id) -> Vec<Notice> {
        let mut notices = Vec::new();
        if let Some(feedback) = self
            .window_inputs
            .get(&window)
            .and_then(|state| state.feedback.as_ref())
        {
            notices.push(Notice::new("pane", feedback.clone(), ToastTone::Error));
        }
        notices.extend(self.close_veto_notice());
        notices
    }

    /// Pinned output that could not be stored securely. A pin whose output
    /// was simply never stored is no failure and has no notice.
    fn pinned_storage_notice(&self) -> Option<Notice> {
        let reason = crate::i18n::text(self.secure_pins.failure_notice()?);
        let description = if self.secure_pins.pending_deletes.is_empty() {
            reason
        } else {
            crate::i18n::format(
                crate::i18n::localizer(),
                "Stored output removal is pending. {reason}",
                &[("reason", &(reason))],
            )
        };
        Some(
            Notice::new(
                "pinned",
                crate::i18n::label(crate::pinned_secret::NOT_SAVED),
                ToastTone::Warning,
            )
            .description(description)
            .action("Retry secure storage", Message::RetryPinnedStorage),
        )
    }

    /// The active pane shows output restored from a pin, with no process.
    fn restored_pane_notice(&self) -> Option<Notice> {
        let pane = self.active_pane()?;
        if !self.restored_outputs.contains_key(&pane.id) {
            return None;
        }
        let label = if pane.endpoint == PaneEndpoint::Local {
            "New shell"
        } else {
            "Reconnect"
        };
        Some(
            Notice::new(
                "restored",
                crate::i18n::label(crate::pinned_output::RESTORED_LABEL),
                ToastTone::Info,
            )
            .description(crate::i18n::label(
                "Only eligible settled output was retained; no process or credentials were resumed.",
            ))
            .action(label, Message::ReconnectRestoredPane),
        )
    }

    /// A held close: one bounded error plus the single action that is safe
    /// to offer at this point.
    fn close_veto_notice(&self) -> Option<Notice> {
        let debt = self.close_veto()?;
        let (label, message) = self.close_veto_action();
        Some(
            Notice::new(
                "close",
                crate::i18n::label("This window cannot close yet"),
                ToastTone::Error,
            )
            .description(crate::i18n::text(debt.summary()))
            .action(label, message),
        )
    }

    /// `presentation` with the notices for `window` and the transient toasts
    /// floating bottom-right. The presentation is always
    /// the stack's first child, so a toast or notice appearing does not
    /// re-parent the app.
    pub(crate) fn notice_layer<'a>(
        &'a self,
        window: Option<iced::window::Id>,
        presentation: Element<'a, Message>,
        transient: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let notices: Vec<EnzNotice<Message>> = self
            .notices_for(window)
            .into_iter()
            .filter(|notice| !self.dismissed_notices.contains(&notice.id))
            .map(|notice| {
                let mut toast = EnzNotice::new(notice.id.clone(), notice.title, notice.tone)
                    .on_dismiss(Message::DismissNotice(notice.id));
                if let Some(description) = notice.description {
                    toast = toast.description(description);
                }
                if let Some((label, message)) = notice.action {
                    toast = toast.action(label, message);
                }
                toast
            })
            .collect();
        // One bottom-right column: the window's notices above the transient
        // toasts, so the two never overlap in the corner.
        let notices = (!notices.is_empty()).then(|| enz_notice_stack(notices).into_element());
        let overlay = (notices.is_some() || transient.is_some()).then(|| {
            enz_toast_overlay(
                iced::widget::column(notices.into_iter().chain(transient))
                    .spacing(enzui_theme::SpacingScale::default().xs())
                    .align_x(iced::Alignment::End),
                ToastOverlayPlacement::default(),
            )
        });
        iced::widget::Stack::with_children(std::iter::once(presentation).chain(overlay))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn notices_for(&self, window: Option<iced::window::Id>) -> Vec<Notice> {
        match window {
            Some(window) if self.detached_windows.contains_key(&window) => {
                self.detached_notices(window)
            }
            _ => self.main_notices(),
        }
    }

    pub(crate) fn dismiss_notice(&mut self, id: String) {
        self.dismissed_notices.insert(id);
    }

    /// Forgets dismissals whose notice is gone, so a cause that clears and
    /// comes back shows again.
    pub(crate) fn prune_dismissed_notices(&mut self) {
        if self.dismissed_notices.is_empty() {
            return;
        }
        let mut live: Vec<String> = self.main_notices().into_iter().map(|n| n.id).collect();
        for window in self.detached_windows.keys() {
            live.extend(self.detached_notices(*window).into_iter().map(|n| n.id));
        }
        self.dismissed_notices.retain(|id| live.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::probe::nodes_of;

    const SIZE: iced::Size = iced::Size::new(1100.0, 800.0);

    fn labels(app: &App) -> Vec<String> {
        nodes_of(app.view(), SIZE)
            .into_iter()
            .filter_map(|node| node.semantics.label)
            .collect()
    }
}
