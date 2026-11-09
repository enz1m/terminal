//! Owned focus for Settings and the SSH manager.
//!
//! Terminal renders one `EnzModalHost` whose surface is Settings, the SSH
//! host list, or the SSH add/edit form (the manager takes priority). Settings
//! captures the titlebar gear as its invoker; the manager captures none,
//! because it is reached from two interchangeable `server` icons whose
//! presence depends on the responsive rung.
//!
//! The manager's scope is also handed to the nested `FormPresenter`: the form
//! remains the state owner, but its focus operations must resolve against the
//! same presentation scope the host binds, so a covered form declines
//! structurally instead of cycling behind the surface above it.

use enzui_core::modal::DismissCause;
use enzui_widgets::modal_host::{ModalFocusLifecycle, ModalHostEvent};
use iced::Task;

use crate::{App, Message, SETTINGS_ID_PREFIX, SSH_MANAGER_SCOPE_KEY, settings_button_id};

impl App {
    /// Reconcile both standard hosts with their owners after every turn.
    pub(crate) fn sync_standard_modal_focus(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        if self.settings_open && self.settings_focus.is_none() {
            let (lifecycle, op) = ModalFocusLifecycle::open::<Message>(
                SETTINGS_ID_PREFIX,
                Some(settings_button_id()),
            );
            self.settings_focus = Some(lifecycle);
            tasks.push(iced::advanced::widget::operate(op));
        } else if !self.settings_open
            && let Some(lifecycle) = self.settings_focus.take()
        {
            tasks.push(lifecycle.close_for_host::<Message>().into_task());
        }
        if self.ssh_manager_open && self.ssh_manager_focus.is_none() {
            let (lifecycle, op) = ModalFocusLifecycle::open::<Message>(SSH_MANAGER_SCOPE_KEY, None);
            self.ssh_manager_focus = Some(lifecycle);
            tasks.push(iced::advanced::widget::operate(op));
        } else if !self.ssh_manager_open
            && let Some(lifecycle) = self.ssh_manager_focus.take()
        {
            tasks.push(lifecycle.close_for_host::<Message>().into_task());
        }
        Task::batch(tasks)
    }

    /// The presentation scope the nested SSH form must aim its focus work at.
    pub(crate) fn ssh_manager_scope(&self) -> Option<iced::advanced::widget::Id> {
        self.ssh_manager_focus
            .as_ref()
            .map(|lifecycle| lifecycle.scope().clone())
    }

    /// A logical Settings dismissal (the surface X / Done).
    pub(crate) fn dismiss_settings_focus(&mut self, cause: DismissCause) -> Task<Message> {
        self.settings_open = false;
        self.settings_focus
            .take()
            .map_or_else(Task::none, |lifecycle| {
                lifecycle
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    /// A logical manager dismissal. Any open form goes with it: the manager is
    /// the presentation, the form is a surface inside it.
    pub(crate) fn dismiss_ssh_manager_focus(&mut self, cause: DismissCause) -> Task<Message> {
        self.ssh_manager_open = false;
        self.ssh_form = None;
        self.ssh_manager_focus
            .take()
            .map_or_else(Task::none, |lifecycle| {
                lifecycle
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    /// Reduce a Settings host event through its owner.
    pub(crate) fn reduce_settings_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(lifecycle) = self.settings_focus.as_ref() else {
            return Task::none();
        };
        let outcome = lifecycle.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.settings_open = false;
            self.settings_focus = None;
        }
        outcome.into_task()
    }

    /// Reduce an SSH-manager host event through its owner. A dismissal also
    /// abandons any keyring wait the manager started.
    pub(crate) fn reduce_ssh_manager_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(lifecycle) = self.ssh_manager_focus.as_ref() else {
            return Task::none();
        };
        let outcome = lifecycle.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.ssh_manager_open = false;
            self.ssh_form = None;
            self.ssh_manager_focus = None;
            let cancel = self.update(Message::CancelSshCredentialOps);
            return Task::batch([outcome.into_task(), cancel]);
        }
        outcome.into_task()
    }
}
