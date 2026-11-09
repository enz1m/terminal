//! Terminal secret-load methods.

use super::*;

impl App {
    pub(crate) fn load_secret_op(
        &mut self,
        owner: String,
        slot: CredentialSlot,
        token: u64,
    ) -> Option<(operation_origin::OperationOrigin, CredentialFuture)> {
        let origin = self.operation_origin()?;
        Some(self.load_secret_op_for(origin, owner, slot, token))
    }

    pub(crate) fn load_secret_op_for(
        &mut self,
        origin: operation_origin::OperationOrigin,
        owner: String,
        slot: CredentialSlot,
        token: u64,
    ) -> (operation_origin::OperationOrigin, CredentialFuture) {
        let op = self.credentials.get(slot.key(&owner));
        self.register_credential_op(
            &owner,
            slot,
            CredentialIntent::Load,
            origin.clone(),
            Some(token),
            false,
            op.cancel_handle(),
        );
        let completion_origin = origin.clone();
        (
            origin,
            Box::pin(async move {
                let result = SshCredentialResult::from_outcome(op.await);
                Message::SshCredentialLoaded(completion_origin, owner, slot, token, result)
            }),
        )
    }

    pub(crate) fn forget_secret_op(
        &mut self,
        owner: String,
        slot: CredentialSlot,
    ) -> Option<(operation_origin::OperationOrigin, CredentialFuture)> {
        let origin = self.operation_origin()?;
        if self.credential_mutation_store.is_some() {
            self.credential_mutations
                .retain(|pending| pending.owner != owner || pending.slot != slot);
            self.credential_mutations.push(PendingCredentialMutation {
                origin: origin.clone(),
                owner,
                slot,
                intent: CredentialMutationIntent::Delete,
                phase: CredentialMutationPhase::InFlight(CredentialMutationAttempt::Apply),
            });
            let index = self.credential_mutations.len() - 1;
            let future =
                self.dispatch_credential_mutation(index, CredentialMutationAttempt::Apply)?;
            return Some((origin, future));
        }

        let op = self.credentials.delete(slot.key(&owner));
        self.register_credential_op(
            &owner,
            slot,
            CredentialIntent::Forget,
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
                Message::SshCredentialDeleted(completion_origin, owner, slot, result)
            }),
        ))
    }
}
