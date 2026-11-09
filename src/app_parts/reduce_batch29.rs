//! Terminal reducer batch 29: SSH save and delete arms.

use super::*;

impl App {
    pub(crate) fn reduce29_confirm_delete_ssh_host_event_3(
        &mut self,
        event: ConfirmDialogEvent,
    ) -> Task<Message> {
        match event {
            // One exit for all of them. The Delete button, the Cancel
            // button, Escape and a scrim press all arrive as
            // `Resolved(cause)`; `ConfirmDecision::from_cause` is the
            // single place that decides which of those means consent, so
            // the app cannot wire the button and the scrim to two
            // different answers.
            ConfirmDialogEvent::Resolved(cause) => {
                let confirmed =
                    enzui_core::confirm::ConfirmDecision::from_cause(cause).is_confirmed();
                return self.update(Message::ConfirmDeleteSshHost(confirmed));
            }
            ConfirmDialogEvent::FocusNext | ConfirmDialogEvent::FocusPrevious => {
                if self.pending_delete_ssh_host.is_some()
                    && let Some(op) = event.focus_operation::<Message>(true)
                {
                    return iced::advanced::widget::operate(op);
                }
            }
            // Neither confirmation carries a checkbox.
            ConfirmDialogEvent::CheckboxToggled(_) => {}
        }

        Task::none()
    }
    #[cfg(test)]
    pub(crate) fn reduce29_save_ssh_connection_0(&mut self) -> Task<Message> {
        self.update(Message::SshFormEvent(FormDialogEvent::SubmitRequested))
    }
    pub(crate) fn reduce29_delete_ssh_host_1(&mut self, idx: usize) -> Task<Message> {
        // Destructive — guard with confirmation.
        self.pending_delete_ssh_host = Some(idx);

        Task::none()
    }
    pub(crate) fn reduce29_confirm_delete_ssh_host_2(&mut self, accepted: bool) -> Task<Message> {
        if accepted
            && let Some(idx) = self.pending_delete_ssh_host
            && idx < self.ssh_hosts.len()
        {
            if self.refuse_new_credential_mutation() {
                self.pending_delete_ssh_host = None;
                return Task::none();
            }
            // The record is NOT removed here. Its secrets are deleted
            // first, and only outcomes that permit removal (the item
            // is gone, was never there, or there is no store at all)
            // let the profile go. A refusal keeps the profile so the
            // secret never outlives it.
            let target = self.ssh_hosts[idx].clone();
            self.retire_pending_deletion();
            self.credential_ops.retain(|op| op.owner != target.id);
            self.pending_ssh_connections
                .retain(|pending| pending.owner != target.id);
            self.pending_delete_ssh_host = None;
            self.ssh_credential_notice = None;
            let mut operations = Vec::new();
            let mut tasks = Vec::new();
            for slot in CredentialSlot::ALL {
                let Some((origin, fut)) = self.forget_secret_op(target.id.clone(), slot) else {
                    self.credential_ops
                        .retain(|op| !operations.contains(&op.origin.generation));
                    return Task::none();
                };
                operations.push(origin.generation.clone());
                tasks.push(self.spawn_credential(fut));
            }
            self.pending_secret_deletion = Some(PendingSecretDeletion {
                profile_id: target.id,
                profile_name: target.name,
                operations,
                refused: false,
            });
            return Task::batch(tasks);
        }
        self.pending_delete_ssh_host = None;

        Task::none()
    }
}
