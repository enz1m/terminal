//! Terminal credential-finish methods.

use super::*;

impl App {
    pub(crate) fn wait_credential_mutation(
        &mut self,
        index: usize,
        attempt: CredentialMutationAttempt,
        outcome: SshCredentialResult,
        intent: CredentialIntent,
        slot: CredentialSlot,
        automatic: bool,
    ) -> Task<Message> {
        let Some(owner) = self.credential_mutation_owner.as_ref() else {
            return Task::none();
        };
        self.credential_mutations[index].phase = CredentialMutationPhase::Waiting {
            attempt,
            observed: owner.snapshot(),
            automatic,
        };
        self.ssh_credential_notice = outcome.notice(intent, slot);
        Task::none()
    }

    pub(crate) fn finish_owner_mutation(
        &mut self,
        origin: &operation_origin::OperationOrigin,
        owner: &str,
        slot: CredentialSlot,
        result: SshCredentialResult,
    ) -> Task<Message> {
        let intent = if matches!(&result, SshCredentialResult::Deleted) {
            CredentialIntent::Forget
        } else {
            self.credential_mutations
                .iter()
                .find(|pending| {
                    pending.origin.generation == origin.generation
                        && pending.owner == owner
                        && pending.slot == slot
                })
                .map_or(CredentialIntent::Remember, |pending| {
                    match &pending.intent {
                        CredentialMutationIntent::Set(_) => CredentialIntent::Remember,
                        CredentialMutationIntent::Delete => CredentialIntent::Forget,
                    }
                })
        };
        if !self.finish_credential_op(origin, owner, slot, intent) {
            return Task::none();
        }
        let Some(index) = self.credential_mutations.iter().position(|pending| {
            pending.origin.generation == origin.generation
                && pending.owner == owner
                && pending.slot == slot
        }) else {
            return Task::none();
        };
        match result {
            SshCredentialResult::Saturated => self.wait_credential_mutation(
                index,
                CredentialMutationAttempt::Apply,
                SshCredentialResult::Saturated,
                intent,
                slot,
                true,
            ),
            SshCredentialResult::MutationUncertain => {
                self.ssh_credential_notice =
                    SshCredentialResult::MutationUncertain.notice(intent, slot);
                self.dispatch_credential_mutation(
                    index,
                    CredentialMutationAttempt::Reconcile {
                        retry_mismatch: true,
                    },
                )
                .map_or_else(Task::none, |future| self.spawn_credential(future))
            }
            SshCredentialResult::Cancelled => self
                .dispatch_credential_mutation(
                    index,
                    CredentialMutationAttempt::Reconcile {
                        retry_mismatch: false,
                    },
                )
                .map_or_else(Task::none, |future| self.spawn_credential(future)),
            settled
                if matches!(
                    (&settled, intent),
                    (SshCredentialResult::Stored, CredentialIntent::Remember)
                        | (SshCredentialResult::Deleted, CredentialIntent::Forget)
                ) =>
            {
                self.credential_mutations.remove(index);
                self.ssh_credential_notice = settled.notice(intent, slot);
                if intent == CredentialIntent::Forget {
                    self.resolve_pending_deletion(origin, owner, &settled)
                } else {
                    Task::none()
                }
            }
            SshCredentialResult::OwnerShutdown => {
                self.credential_mutations[index].phase = CredentialMutationPhase::OwnerStopped;
                self.ssh_credential_notice =
                    SshCredentialResult::OwnerShutdown.notice(intent, slot);
                Task::none()
            }
            terminal => self.wait_credential_mutation(
                index,
                CredentialMutationAttempt::Apply,
                terminal,
                intent,
                slot,
                false,
            ),
        }
    }

    pub(crate) fn finish_credential_reconciliation(
        &mut self,
        origin: &operation_origin::OperationOrigin,
        owner: &str,
        slot: CredentialSlot,
        retry_mismatch: bool,
        result: SshCredentialResult,
    ) -> Task<Message> {
        let Some(index) = self.credential_mutations.iter().position(|pending| {
            pending.origin.generation == origin.generation
                && pending.owner == owner
                && pending.slot == slot
        }) else {
            return Task::none();
        };
        let intent = match &self.credential_mutations[index].intent {
            CredentialMutationIntent::Set(_) => CredentialIntent::Remember,
            CredentialMutationIntent::Delete => CredentialIntent::Forget,
        };
        if !self.finish_credential_op(origin, owner, slot, intent) {
            return Task::none();
        }
        let matches_intent = match (&self.credential_mutations[index].intent, &result) {
            (CredentialMutationIntent::Set(expected), SshCredentialResult::Retrieved(actual)) => {
                actual.matches_text(expected)
            }
            (CredentialMutationIntent::Delete, SshCredentialResult::NotFound) => true,
            _ => false,
        };
        if matches_intent {
            self.credential_mutations.remove(index);
            self.ssh_credential_notice = None;
            let settled = if intent == CredentialIntent::Remember {
                SshCredentialResult::Stored
            } else {
                SshCredentialResult::Deleted
            };
            return if intent == CredentialIntent::Forget {
                self.resolve_pending_deletion(origin, owner, &settled)
            } else {
                Task::none()
            };
        }
        match result {
            SshCredentialResult::Retrieved(_) | SshCredentialResult::NotFound if retry_mismatch => {
                self.dispatch_credential_mutation(index, CredentialMutationAttempt::Apply)
                    .map_or_else(Task::none, |future| self.spawn_credential(future))
            }
            SshCredentialResult::Saturated => self.wait_credential_mutation(
                index,
                CredentialMutationAttempt::Reconcile { retry_mismatch },
                SshCredentialResult::Saturated,
                intent,
                slot,
                true,
            ),
            SshCredentialResult::OwnerShutdown => {
                self.credential_mutations[index].phase = CredentialMutationPhase::OwnerStopped;
                self.ssh_credential_notice =
                    SshCredentialResult::OwnerShutdown.notice(intent, slot);
                Task::none()
            }
            settled @ (SshCredentialResult::Retrieved(_) | SshCredentialResult::NotFound) => {
                self.credential_mutations.remove(index);
                self.ssh_credential_notice = settled.notice(intent, slot);
                if intent == CredentialIntent::Forget {
                    self.resolve_pending_deletion(origin, owner, &settled)
                } else {
                    Task::none()
                }
            }
            unsettled => {
                let automatic = matches!(&unsettled, SshCredentialResult::MutationUncertain);
                self.wait_credential_mutation(
                    index,
                    CredentialMutationAttempt::Reconcile { retry_mismatch },
                    unsettled,
                    intent,
                    slot,
                    automatic,
                )
            }
        }
    }
}
