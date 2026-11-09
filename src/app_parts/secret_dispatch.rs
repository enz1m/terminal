//! Terminal credential-dispatch methods.

use super::*;

impl App {
    pub(crate) fn dispatch_credential_mutation(
        &mut self,
        index: usize,
        attempt: CredentialMutationAttempt,
    ) -> Option<CredentialFuture> {
        let store = self.credential_mutation_store.clone()?;
        let pending = self.credential_mutations.get(index)?;
        let origin = pending.origin.clone();
        let owner = pending.owner.clone();
        let slot = pending.slot;
        let intent = match &pending.intent {
            CredentialMutationIntent::Set(_) => CredentialIntent::Remember,
            CredentialMutationIntent::Delete => CredentialIntent::Forget,
        };
        let op = match (attempt, &pending.intent) {
            (CredentialMutationAttempt::Apply, CredentialMutationIntent::Set(secret)) => store.set(
                slot.key(&owner),
                Secret::new(secret.expose_secret().as_bytes().to_vec()),
            ),
            (CredentialMutationAttempt::Apply, CredentialMutationIntent::Delete) => {
                store.delete(slot.key(&owner))
            }
            (CredentialMutationAttempt::Reconcile { .. }, _) => store.reconcile(slot.key(&owner)),
        };
        self.credential_mutations[index].phase = CredentialMutationPhase::InFlight(attempt);
        self.register_credential_op(
            &owner,
            slot,
            intent,
            origin.clone(),
            None,
            true,
            op.cancel_handle(),
        );
        Some(Box::pin(async move {
            let result = SshCredentialResult::from_outcome(op.await);
            match (attempt, intent) {
                (CredentialMutationAttempt::Apply, CredentialIntent::Remember) => {
                    Message::SshCredentialStored(origin, owner, slot, result)
                }
                (CredentialMutationAttempt::Apply, CredentialIntent::Forget) => {
                    Message::SshCredentialDeleted(origin, owner, slot, result)
                }
                (
                    CredentialMutationAttempt::Reconcile { retry_mismatch },
                    CredentialIntent::Remember | CredentialIntent::Forget,
                ) => Message::SshCredentialReconciled(origin, owner, slot, retry_mismatch, result),
                (_, CredentialIntent::Load) => unreachable!("loads never use the mutation owner"),
            }
        }))
    }

    pub(crate) fn retry_credential_mutations(&mut self, force: bool) -> Task<Message> {
        let Some(owner) = self.credential_mutation_owner.as_ref() else {
            return Task::none();
        };
        let snapshot = owner.snapshot();
        if !snapshot.is_accepting() {
            return Task::none();
        }
        let available = CREDENTIAL_MUTATION_CAPACITY.saturating_sub(snapshot.admitted());
        let indices: Vec<_> = self
            .credential_mutations
            .iter()
            .enumerate()
            .filter_map(|(index, pending)| match pending.phase {
                CredentialMutationPhase::Waiting {
                    attempt,
                    observed,
                    automatic,
                } if force
                    || (automatic
                        && (snapshot.admitted() < observed.admitted()
                            || snapshot.uncertain_keys() < observed.uncertain_keys())) =>
                {
                    Some((index, attempt))
                }
                _ => None,
            })
            .take(available)
            .collect();
        let mut tasks = Vec::with_capacity(indices.len());
        for (index, attempt) in indices {
            if let Some(future) = self.dispatch_credential_mutation(index, attempt) {
                tasks.push(self.spawn_credential(future));
            }
        }
        Task::batch(tasks)
    }
}
