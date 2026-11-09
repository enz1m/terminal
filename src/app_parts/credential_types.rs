//! Terminal credential operation types.

use super::*;

pub(crate) struct InFlightCredentialOp {
    pub(crate) origin: operation_origin::OperationOrigin,
    pub(crate) connection_token: Option<u64>,
    pub(crate) owner: String,
    pub(crate) slot: CredentialSlot,
    pub(crate) intent: CredentialIntent,
    pub(crate) owner_mutation: bool,
    pub(crate) cancel: CancelHandle,
}

#[derive(Clone)]
pub(crate) enum CredentialMutationIntent {
    Set(SecretText),
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CredentialMutationAttempt {
    Apply,
    Reconcile { retry_mismatch: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CredentialMutationPhase {
    InFlight(CredentialMutationAttempt),
    Waiting {
        attempt: CredentialMutationAttempt,
        observed: CredentialMutationSnapshot,
        automatic: bool,
    },
    OwnerStopped,
}

pub(crate) struct PendingCredentialMutation {
    pub(crate) origin: operation_origin::OperationOrigin,
    pub(crate) owner: String,
    pub(crate) slot: CredentialSlot,
    pub(crate) intent: CredentialMutationIntent,
    pub(crate) phase: CredentialMutationPhase,
}

impl Drop for PendingCredentialMutation {
    fn drop(&mut self) {
        self.origin
            .generation
            .retire(enzui_core::cancellation::CancelCause::Disconnected);
    }
}

// Editing a saved profile must not retarget a connection already waiting on its secret.
pub(crate) struct PendingSshConnection {
    pub(crate) origin: operation_origin::OperationOrigin,
    pub(crate) token: u64,
    pub(crate) owner: String,
    pub(crate) slot: CredentialSlot,
    pub(crate) host: SshHostRecord,
}

/// Test-only SSH spawn seam (see [`App::ssh_spawn_override`]).
#[cfg(test)]
pub(crate) type SshSpawnOverride =
    Box<dyn FnMut(&str, &SshHostRecord, Option<&str>) -> Result<pty::PtySession, pty::PtyError>>;

/// A profile deletion waiting on its secrets.
///
/// The record is NOT removed when the user confirms; it is removed only once
/// every slot has answered with an outcome that permits removal. A refused
/// deletion keeps the profile, so no restart can find an item whose profile
/// is gone.
pub(crate) struct PendingSecretDeletion {
    pub(crate) profile_id: String,
    pub(crate) profile_name: String,
    pub(crate) operations: Vec<operation_origin::RequestGeneration>,
    pub(crate) refused: bool,
}

/// The future shape every credential operation hands to `Task::future`.
/// Spelled out (rather than `impl Future`) so the reducer can build one in a
/// `&mut self` method — registering its cancel handle — and so tests can
/// drive the exact same future the runtime drives.
pub(crate) type CredentialFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Message> + Send>>;
