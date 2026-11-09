//! Terminal credential-queue methods.

use super::*;

impl App {
    pub(crate) fn queue_session_save(&mut self) -> Task<Message> {
        self.queue_secure_pin_save()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Admission keeps operation identity and cancellation ownership explicit"
    )]
    pub(crate) fn register_credential_op(
        &mut self,
        owner: &str,
        slot: CredentialSlot,
        intent: CredentialIntent,
        origin: operation_origin::OperationOrigin,
        connection_token: Option<u64>,
        owner_mutation: bool,
        cancel: CancelHandle,
    ) {
        if intent != CredentialIntent::Load {
            self.credential_ops
                .retain(|op| !(op.owner == owner && op.slot == slot && op.intent == intent));
        }
        self.credential_ops.push(InFlightCredentialOp {
            origin,
            connection_token,
            owner: owner.to_owned(),
            slot,
            intent,
            owner_mutation,
            cancel,
        });
    }

    /// Allocate the next SSH credential-read token.
    ///
    /// Every issued token is nonzero: 0 is reserved for reads that never park
    /// a pending connection, so it must never be handed to one. The counter
    /// rolls `u64::MAX` back to 1, never to 0, so no release path (which would
    /// wrap silently) or debug path (which would panic) can emit the reserved
    /// value.
    pub(crate) fn next_ssh_credential_token(&mut self) -> u64 {
        let token = self.next_credential_token;
        self.next_credential_token = token.checked_add(1).unwrap_or(1);
        token
    }

    /// Whether any credential operation is still waiting on the keyring — the
    /// state the manager's "Waiting for the keyring… / Cancel" row renders.
    pub(crate) fn credential_op_in_flight(&self) -> bool {
        !self.credential_ops.is_empty()
            || self
                .credential_mutations
                .iter()
                .any(|pending| !matches!(pending.phase, CredentialMutationPhase::OwnerStopped))
    }

    pub(crate) fn credential_mutation_waiting(&self) -> bool {
        self.credential_mutations
            .iter()
            .any(|pending| matches!(pending.phase, CredentialMutationPhase::Waiting { .. }))
    }

    /// The single place a credential future becomes runtime work.
    ///
    /// Under `cfg(test)` the future is parked instead of handed to iced, and
    /// `drive_credentials` runs it and feeds its completion message back
    /// through `update`. That is what lets a headless test drive the REAL
    /// reducer path — message in, store call, completion message, state out —
    /// rather than a hand-built future no production arm creates.
    pub(crate) fn spawn_credential(&mut self, fut: CredentialFuture) -> Task<Message> {
        #[cfg(test)]
        {
            self.credential_futures.push(fut);
            Task::none()
        }
        #[cfg(not(test))]
        {
            Task::future(fut)
        }
    }
}
