//! Terminal credential mutation methods.

use super::*;

impl App {
    pub(crate) fn credential_mutation_subscription(&self) -> Subscription<Message> {
        if self.credential_mutation_owner.is_some()
            && self.credential_mutations.iter().any(|pending| {
                matches!(
                    pending.phase,
                    CredentialMutationPhase::Waiting {
                        automatic: true,
                        ..
                    }
                )
            })
        {
            iced::time::every(Duration::from_millis(50)).map(|_| Message::CredentialMutationWake)
        } else {
            Subscription::none()
        }
    }

    // Retrieved secrets stay in wipe-on-drop custody until the SSH spawn seals its memfd;
    // they must never be typed into an already-running terminal.
    pub(crate) fn resume_pending_ssh_connection(
        &mut self,
        origin: &operation_origin::OperationOrigin,
        owner: &str,
        slot: CredentialSlot,
        token: u64,
        result: SshCredentialResult,
    ) -> Task<Message> {
        let Some(pos) = self.pending_ssh_connections.iter().position(|pending| {
            pending.origin == *origin
                && pending.token == token
                && pending.owner == owner
                && pending.slot == slot
        }) else {
            // A superseded, already-resolved, or duplicated completion has no
            // pending entry; the secret (if any) is dropped without a route.
            return Task::none();
        };
        let pending = self.pending_ssh_connections.remove(pos);
        // A missing secret may prompt in SSH; cancellation cannot launch a child.
        if matches!(result, SshCredentialResult::Cancelled) {
            return Task::none();
        }

        // Take-once custody: a cloned copy of this message finds no pending
        // entry above and cannot deliver again.
        let secret = match result {
            SshCredentialResult::Retrieved(handle) => handle.take(),
            _ => None,
        };

        // The stored secret round-trips from the `String` the form captured,
        // so it is UTF-8; a non-UTF-8 read degrades to the prompt rather than
        // spawning with a lossy password. Without one, plain `ssh` prompts
        // in-terminal.
        let secret = secret.filter(|secret| std::str::from_utf8(secret.expose_secret()).is_ok());
        self.connect_ssh_record(pending.host, secret)
    }
}
