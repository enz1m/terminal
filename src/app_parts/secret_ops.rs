//! Terminal secret-op methods.

use super::*;

impl App {
    pub(crate) fn remember_secret_op(
        &mut self,
        owner: String,
        slot: CredentialSlot,
        secret: String,
    ) -> Option<(operation_origin::OperationOrigin, CredentialFuture)> {
        if self.refuse_new_credential_mutation() {
            return None;
        }
        self.remember_secret_value_op(owner, slot, SecretText::new(secret))
    }

    pub(crate) fn refuse_new_credential_mutation(&mut self) -> bool {
        if self.credential_mutation_admission_open {
            return false;
        }
        self.ssh_credential_notice = Some(CredentialNotice::info(
            "Terminal is settling credential changes for close. Keep this window open and retry afterward.",
        ));
        true
    }

    pub(crate) fn remember_secret_value_op(
        &mut self,
        owner: String,
        slot: CredentialSlot,
        secret: SecretText,
    ) -> Option<(operation_origin::OperationOrigin, CredentialFuture)> {
        let origin = self.operation_origin()?;
        if self.credential_mutation_store.is_some() {
            self.credential_mutations
                .retain(|pending| pending.owner != owner || pending.slot != slot);
            self.credential_mutations.push(PendingCredentialMutation {
                origin: origin.clone(),
                owner,
                slot,
                intent: CredentialMutationIntent::Set(secret),
                phase: CredentialMutationPhase::InFlight(CredentialMutationAttempt::Apply),
            });
            let index = self.credential_mutations.len() - 1;
            let future =
                self.dispatch_credential_mutation(index, CredentialMutationAttempt::Apply)?;
            return Some((origin, future));
        }

        let op = self.credentials.set(
            slot.key(&owner),
            Secret::new(secret.expose_secret().as_bytes().to_vec()),
        );
        self.register_credential_op(
            &owner,
            slot,
            CredentialIntent::Remember,
            origin.clone(),
            None,
            false,
            op.cancel_handle(),
        );
        let completion_origin = origin.clone();
        Some((
            origin,
            Box::pin(async move {
                let result = SshCredentialResult::from_outcome(op.await);
                Message::SshCredentialStored(completion_origin, owner, slot, result)
            }),
        ))
    }
}
