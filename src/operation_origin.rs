use std::sync::Arc;

use enzui_core::cancellation::{CancelCause, CancelToken};
use iced::Task;

use crate::ssh_credentials::{CredentialIntent, CredentialSlot};
use crate::{
    App, CredentialMutationAttempt, CredentialMutationPhase, InFlightCredentialOp, Message,
    paste_bytes,
};

// Queued completions retain this allocation; wrapping the numeric SSH token cannot
// make a later request inherit an earlier request's identity.
#[derive(Clone, Debug)]
pub(crate) struct RequestGeneration(Arc<CancelToken>);

impl RequestGeneration {
    pub(crate) fn new() -> Self {
        Self(Arc::new(CancelToken::new()))
    }

    pub(crate) fn retire(&self, cause: CancelCause) {
        self.0.cancel(cause);
    }

    pub(crate) fn is_live(&self) -> bool {
        !self.0.is_terminal()
    }

    pub(crate) fn cancellation(&self) -> CancelToken {
        self.0.share()
    }
}

impl PartialEq for RequestGeneration {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RequestGeneration {}

impl std::hash::Hash for RequestGeneration {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OperationOrigin {
    pub(crate) session: String,
    pub(crate) pane: String,
    pub(crate) window: Option<iced::window::Id>,
    pub(crate) generation: RequestGeneration,
}

pub(crate) struct PendingPaste {
    origin: OperationOrigin,
}

impl Drop for PendingPaste {
    fn drop(&mut self) {
        self.origin.generation.retire(CancelCause::Disconnected);
    }
}

impl Drop for InFlightCredentialOp {
    fn drop(&mut self) {
        if !self.owner_mutation {
            self.origin.generation.retire(CancelCause::Disconnected);
        }
        self.cancel.cancel();
    }
}

impl App {
    pub(crate) fn operation_origin(&self) -> Option<OperationOrigin> {
        let session = self.sessions.get(self.active)?;
        let pane = session.panes.get(session.active_pane)?;
        self.operation_origin_for(&session.id, &pane.id)
    }

    pub(crate) fn operation_origin_for(
        &self,
        session: &str,
        pane: &str,
    ) -> Option<OperationOrigin> {
        let origin = OperationOrigin {
            session: session.to_owned(),
            pane: pane.to_owned(),
            window: self.detached_window_of(session).or(self.main_window),
            generation: RequestGeneration::new(),
        };
        self.operation_origin_is_live(&origin).then_some(origin)
    }

    pub(crate) fn operation_origin_is_live(&self, origin: &OperationOrigin) -> bool {
        origin.generation.is_live()
            && origin.window
                == self
                    .detached_window_of(&origin.session)
                    .or(self.main_window)
            && origin.window.is_none_or(|window| {
                !self
                    .detached_windows
                    .get(&window)
                    .is_some_and(|window| window.closing)
            })
            && origin
                .window
                .is_none_or(|window| self.windows.is_open(window))
            && self.sessions.iter().any(|session| {
                session.id == origin.session
                    && session.panes.iter().any(|pane| pane.id == origin.pane)
            })
    }

    fn credential_completion_is_current(
        &self,
        origin: &OperationOrigin,
        owner: &str,
        slot: CredentialSlot,
        intent: CredentialIntent,
    ) -> bool {
        let operation = self.credential_ops.iter().find(|op| {
            op.origin == *origin && op.owner == owner && op.slot == slot && op.intent == intent
        });
        if operation.is_some_and(|op| op.owner_mutation) {
            return origin.generation.is_live()
                && self.ssh_hosts.iter().any(|host| host.id == owner)
                && self.credential_mutations.iter().any(|pending| {
                    pending.origin.generation == origin.generation
                        && pending.owner == owner
                        && pending.slot == slot
                });
        }
        self.operation_origin_is_live(origin)
            && self.pane_connection_is_current(origin)
            && self.ssh_hosts.iter().any(|host| host.id == owner)
            && operation.is_some()
    }

    fn owner_mutation_attempt_is_current(
        &self,
        origin: &OperationOrigin,
        owner: &str,
        slot: CredentialSlot,
        attempt: CredentialMutationAttempt,
    ) -> bool {
        self.credential_ops.iter().any(|op| {
            op.owner_mutation && op.origin == *origin && op.owner == owner && op.slot == slot
        }) && self.credential_mutations.iter().any(|pending| {
            pending.origin.generation == origin.generation
                && pending.owner == owner
                && pending.slot == slot
                && pending.phase == CredentialMutationPhase::InFlight(attempt)
        })
    }

    fn paste_completion_is_current(&self, origin: &OperationOrigin) -> bool {
        self.operation_origin_is_live(origin)
            && self.ptys.contains_key(&origin.pane)
            && self
                .paste_requests
                .iter()
                .any(|pending| pending.origin == *origin)
    }

    pub(crate) fn accepts_operation_completion(&self, message: &Message) -> bool {
        match message {
            Message::WindowChromeKey(window, epoch, ..) => self.window_chrome_live(*window, epoch),
            Message::PaneInput(origin, message) => self.accepts_pane_input(origin, message),
            Message::PanePointer(origin, epoch, _, message) => {
                self.pointer_epoch_is_current(origin, epoch)
                    && self.accepts_pane_input(origin, message)
            }
            Message::InputMenuEvent(origin, _) => self.input_menu_live(origin),
            Message::OwnedUriActivation(origin, ..) | Message::OwnedLinkFinished(origin, ..) => {
                self.owned_link_live(origin)
            }
            Message::SshCredentialStored(origin, owner, slot, _)
                if self.credential_mutation_store.is_some() =>
            {
                self.owner_mutation_attempt_is_current(
                    origin,
                    owner,
                    *slot,
                    CredentialMutationAttempt::Apply,
                )
            }
            Message::SshCredentialStored(origin, owner, slot, _) => self
                .credential_completion_is_current(origin, owner, *slot, CredentialIntent::Remember),
            Message::SshCredentialLoaded(origin, owner, slot, token, _) => {
                self.credential_completion_is_current(origin, owner, *slot, CredentialIntent::Load)
                    && self
                        .credential_ops
                        .iter()
                        .any(|op| op.origin == *origin && op.connection_token == Some(*token))
            }
            Message::SshCredentialDeleted(origin, owner, slot, _)
                if self.credential_mutation_store.is_some() =>
            {
                self.owner_mutation_attempt_is_current(
                    origin,
                    owner,
                    *slot,
                    CredentialMutationAttempt::Apply,
                )
            }
            Message::SshCredentialDeleted(origin, owner, slot, _) => self
                .credential_completion_is_current(origin, owner, *slot, CredentialIntent::Forget),
            Message::SshCredentialReconciled(origin, owner, slot, retry_mismatch, _) => self
                .owner_mutation_attempt_is_current(
                    origin,
                    owner,
                    *slot,
                    CredentialMutationAttempt::Reconcile {
                        retry_mismatch: *retry_mismatch,
                    },
                ),
            Message::ClipboardPasteReady(origin, _) => self.paste_completion_is_current(origin),
            _ => true,
        }
    }

    pub(crate) fn finish_credential_op(
        &mut self,
        origin: &OperationOrigin,
        owner: &str,
        slot: CredentialSlot,
        intent: CredentialIntent,
    ) -> bool {
        if !self.credential_completion_is_current(origin, owner, slot, intent) {
            return false;
        }
        self.credential_ops.retain(|op| op.origin != *origin);
        true
    }

    pub(crate) fn cancel_credential_operations(&mut self) {
        for op in &self.credential_ops {
            if !op.owner_mutation {
                op.origin.generation.retire(CancelCause::Cancelled);
            }
            op.cancel.cancel();
        }
        self.credential_ops.retain(|op| op.owner_mutation);
        self.reconcile_endpoint_connections();
        for id in std::mem::take(&mut self.pending_endpoint_restores) {
            self.pane_endpoint_error(
                &id,
                "SSH restoration cancelled. Open Remote to reconnect explicitly.",
            );
        }
        self.pending_ssh_connections.clear();
        if self.credential_mutation_store.is_none() {
            self.pending_secret_deletion = None;
        }
    }

    pub(crate) fn retire_connection_requests(&mut self, owner: &str) {
        let retired: Vec<_> = self
            .pending_ssh_connections
            .iter()
            .filter(|pending| pending.owner == owner)
            .map(|pending| pending.origin.generation.clone())
            .collect();
        self.credential_ops
            .retain(|op| !retired.contains(&op.origin.generation));
        self.pending_ssh_connections
            .retain(|pending| pending.owner != owner);
    }

    pub(crate) fn retire_pending_deletion(&mut self) {
        if let Some(pending) = self.pending_secret_deletion.take() {
            self.credential_ops
                .retain(|op| !pending.operations.contains(&op.origin.generation));
        }
    }

    pub(crate) fn reconcile_operation_origins(&mut self) {
        let invalid: Vec<_> = self
            .credential_ops
            .iter()
            .filter(|op| {
                !op.owner_mutation
                    && (!self.operation_origin_is_live(&op.origin)
                        || !self.ssh_hosts.iter().any(|host| host.id == op.owner))
            })
            .map(|op| op.origin.generation.clone())
            .collect();
        self.credential_ops
            .retain(|op| !invalid.contains(&op.origin.generation));
        self.pending_ssh_connections
            .retain(|pending| pending.origin.generation.is_live());
        if self
            .pending_secret_deletion
            .as_ref()
            .is_some_and(|pending| {
                pending
                    .operations
                    .iter()
                    .any(|generation| !generation.is_live())
            })
        {
            self.retire_pending_deletion();
        }
        let invalid: Vec<_> = self
            .paste_requests
            .iter()
            .filter(|pending| {
                !self.operation_origin_is_live(&pending.origin)
                    || !self.ptys.contains_key(&pending.origin.pane)
            })
            .map(|pending| pending.origin.generation.clone())
            .collect();
        self.paste_requests
            .retain(|pending| !invalid.contains(&pending.origin.generation));
        self.reconcile_endpoint_connections();
        self.reconcile_host_trust();
        self.reconcile_window_inputs();
    }

    pub(crate) fn request_paste(&mut self) -> Task<Message> {
        let Some(origin) = self.operation_origin() else {
            return Task::none();
        };
        self.request_paste_for(origin)
    }

    pub(crate) fn request_paste_for(&mut self, origin: OperationOrigin) -> Task<Message> {
        if !self.operation_origin_is_live(&origin) {
            return Task::none();
        }
        if !self.ptys.contains_key(&origin.pane) {
            return Task::none();
        }
        self.paste_requests.retain(|pending| {
            pending.origin.session != origin.session || pending.origin.pane != origin.pane
        });
        self.paste_requests.push(PendingPaste {
            origin: origin.clone(),
        });
        iced::clipboard::read_text().map(move |result| {
            Message::ClipboardPasteReady(origin.clone(), result.ok().map(|text| text.to_string()))
        })
    }

    pub(crate) fn finish_paste(&mut self, origin: OperationOrigin, text: Option<String>) {
        if !self.paste_completion_is_current(&origin) {
            return;
        }
        self.paste_requests
            .retain(|pending| pending.origin != origin);
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return;
        };
        let Some(pane) = self
            .sessions
            .iter()
            .find(|session| session.id == origin.session)
            .and_then(|session| session.panes.iter().find(|pane| pane.id == origin.pane))
        else {
            return;
        };
        let bracketed = pane.bracketed_paste;
        match crate::escape_policy::normalized_paste(&text) {
            Ok(payload) if payload.is_empty() => {}
            Ok(payload) => {
                let bytes = paste_bytes(&payload, bracketed);
                self.exclude_pinned_input(&origin.pane);
                self.write_pane_pty(&origin.session, &origin.pane, &bytes);
            }
            Err(refusal) => {
                self.pane_feedback = Some(refusal.user_summary().to_owned());
            }
        }
    }
}

#[cfg(test)]
#[path = "operation_origin_tests.rs"]
pub(crate) mod tests;
