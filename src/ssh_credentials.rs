//! SSH credential persistence through the framework Secret Service seam.
//!
//! The framework ships the real credential transport
//! (`enzui_services::credential_store::secret_service`), so a saved password
//! survives a restart. The app's job is to name its credentials and to project
//! outcomes truthfully, not to invent a keyring or a plaintext profile field.
//!
//! Three decisions live here rather than in `main.rs`:
//!
//! * **Namespacing.** Every credential is `(app id, stable profile id,
//!   label)`. The label is [`CredentialSlot::label`] — `password` for
//!   password auth, `passphrase` for a key file's passphrase — so the two
//!   secrets of ONE profile occupy two distinct Secret Service items and
//!   cannot overwrite or read each other. The owner is `SshHostRecord::id`
//!   (`ssh-host-N`), which is stable across renames, so renaming a profile
//!   never orphans its secret.
//! * **The outcome ladder is not collapsed.** [`SshCredentialResult`] maps
//!   1:1 from [`CredentialOutcome`]. A dismissed unlock prompt and a policy
//!   refusal are *ordinary states* with their own text, never a generic
//!   "connection failed" — the user who clicked *Cancel* on the keyring
//!   prompt did that on purpose, and the SSH session itself is unaffected.
//! * **Deletion consent.** [`SshCredentialResult::permits_profile_removal`]
//!   is the single place that decides whether removing a profile record is
//!   allowed. A refused secret deletion keeps the profile, so a restart
//!   cannot resurrect a dangling item with no matching profile.

use std::fmt;
use std::sync::{Arc, Mutex};

use enzui_core::form::SecretText;
use enzui_services::credential_store::{
    CredentialKey, CredentialOutcome, CredentialPresence, CredentialStore, Secret,
    mutation_owner::{CredentialMutationOwner, CredentialMutationStore},
};

/// Reverse-DNS application id, the first attribute of every key. Shared with
/// nothing: another enzUI app storing `password` for owner `ssh-host-1` gets a
/// different item because this attribute differs.
pub(crate) const APP_ID: &str = "com.enzui.terminal-emulator";

/// Which of a profile's two possible secrets a key names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CredentialSlot {
    /// Password authentication (`SshAuthMode::Password`).
    Password,
    /// The passphrase protecting a key file (`SshAuthMode::KeyFile`).
    Passphrase,
}

impl CredentialSlot {
    /// Every slot, for the paths that must act on all of a profile's secrets
    /// (deleting the profile, clearing after an auth-mode change).
    pub(crate) const ALL: [CredentialSlot; 2] =
        [CredentialSlot::Password, CredentialSlot::Passphrase];

    /// The Secret Service item label. These two strings are the collision
    /// boundary between a profile's two secrets; `ssh_credential_persistence_tests`
    /// asserts they differ and that each round-trips independently.
    pub(crate) fn label(self) -> &'static str {
        match self {
            CredentialSlot::Password => "password",
            CredentialSlot::Passphrase => "passphrase",
        }
    }

    /// The full three-attribute key for `profile_id`.
    pub(crate) fn key(self, profile_id: &str) -> CredentialKey {
        CredentialKey::new(APP_ID, profile_id, self.label())
    }

    /// How the UI names this secret in a sentence.
    pub(crate) fn noun(self) -> &'static str {
        match self {
            CredentialSlot::Password => "password",
            CredentialSlot::Passphrase => "key passphrase",
        }
    }
}

/// What the app was trying to do, so an outcome can be phrased about the
/// intent rather than about the transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CredentialIntent {
    /// Remember a secret the user just typed.
    Remember,
    /// Read a remembered secret to answer ssh's own prompt.
    Load,
    /// Forget a secret (profile deleted, or auth mode changed).
    Forget,
}

/// A message-safe startup result that keeps the mutation owner and store together.
///
/// Its `Debug` reports lifecycle shape only and never provider state.
#[derive(Clone)]
pub(crate) struct CredentialBackend {
    store: Arc<dyn CredentialStore>,
    mutation_owner: Option<Arc<CredentialMutationOwner>>,
    mutation_store: Option<CredentialMutationStore>,
    unavailable: bool,
}

impl CredentialBackend {
    #[cfg(test)]
    pub(crate) fn new(store: Arc<dyn CredentialStore>) -> Self {
        Self {
            store,
            mutation_owner: None,
            mutation_store: None,
            unavailable: false,
        }
    }

    pub(crate) fn with_mutation_owner(
        owner: CredentialMutationOwner,
        store: CredentialMutationStore,
    ) -> Self {
        Self {
            store: Arc::new(store.clone()),
            mutation_owner: Some(Arc::new(owner)),
            mutation_store: Some(store),
            unavailable: false,
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            store: Arc::new(enzui_services::credential_store::UnavailableCredentialStore),
            mutation_owner: None,
            mutation_store: None,
            unavailable: true,
        }
    }

    pub(crate) fn store(&self) -> Arc<dyn CredentialStore> {
        Arc::clone(&self.store)
    }

    pub(crate) fn mutation_owner(&self) -> Option<Arc<CredentialMutationOwner>> {
        self.mutation_owner.clone()
    }

    pub(crate) fn mutation_store(&self) -> Option<CredentialMutationStore> {
        self.mutation_store.clone()
    }

    pub(crate) fn is_unavailable(&self) -> bool {
        self.unavailable
    }
}

impl fmt::Debug for CredentialBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialBackend")
            .field("owner_backed", &self.mutation_owner.is_some())
            .field("unavailable", &self.unavailable)
            .finish()
    }
}

/// The app-facing projection of one credential operation.
///
/// One arm per [`CredentialOutcome`] arm: folding `UnlockCancelled` into
/// `Cancelled`, or `PermissionDenied` into `Failed`, is exactly the mistake
/// the framework ladder exists to prevent, and this enum is where an app
/// would make it.
///
/// [`Self::Retrieved`] carries the framework [`Secret`] itself, in
/// [`RetrievedSecret`] custody: no plaintext `String`/`Vec<u8>` copy is minted
/// on the way through the message, so there is no un-wiped heap buffer to
/// leak. Cloning the message clones a handle, never the bytes, and the bytes
/// can be taken exactly once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SshCredentialResult {
    Stored,
    Retrieved(RetrievedSecret),
    Deleted,
    NotFound,
    /// No Secret Service at all. Nothing was written, and nothing may be
    /// written to disk instead.
    Unavailable,
    /// The keyring was locked and the unlock prompt was dismissed. A normal
    /// user decision, not an error.
    UnlockDismissed,
    /// The service answered and refused (policy, sandbox).
    PermissionDenied,
    /// The app cancelled the operation (dialog dismissed, explicit Cancel).
    Cancelled,
    /// Bounded mutation admission refused this attempt.
    Saturated,
    /// The retained mutation owner stopped before provider entry.
    OwnerShutdown,
    /// Provider entry occurred but its final state is not yet known.
    MutationUncertain,
    Failed(String),
}

impl SshCredentialResult {
    /// 1:1 projection. Every arm is named; nothing is folded.
    pub(crate) fn from_outcome(outcome: CredentialOutcome) -> Self {
        match outcome {
            CredentialOutcome::Stored => Self::Stored,
            // The Secret is MOVED into the handle. Nothing here copies the
            // plaintext into a `String`: such a copy would outlive the
            // Secret's wipe-on-drop and is exactly what this projection used
            // to do.
            CredentialOutcome::Retrieved(secret) => Self::Retrieved(RetrievedSecret::new(secret)),
            CredentialOutcome::Deleted => Self::Deleted,
            CredentialOutcome::NotFound => Self::NotFound,
            CredentialOutcome::Unavailable => Self::Unavailable,
            CredentialOutcome::UnlockCancelled => Self::UnlockDismissed,
            CredentialOutcome::PermissionDenied => Self::PermissionDenied,
            CredentialOutcome::Cancelled => Self::Cancelled,
            CredentialOutcome::Saturated => Self::Saturated,
            CredentialOutcome::OwnerShutdown => Self::OwnerShutdown,
            CredentialOutcome::MutationUncertain => Self::MutationUncertain,
            CredentialOutcome::Failed(reason) => Self::Failed(reason),
        }
    }

    /// May the profile record be removed after this deletion outcome?
    ///
    /// `Deleted`/`NotFound` mean the item is gone; `Unavailable` means there
    /// is no store that could be holding one. Everything else — refused,
    /// locked, cancelled, broken — leaves a secret that would outlive its
    /// profile, so the profile stays and the user is told why.
    pub(crate) fn permits_profile_removal(&self) -> bool {
        matches!(self, Self::Deleted | Self::NotFound | Self::Unavailable)
    }

    /// The user-visible line for this outcome, or `None` when the outcome
    /// needs no words (a store that worked, a read that found nothing and
    /// hands over to ssh's own prompt, an operation the app itself
    /// cancelled).
    pub(crate) fn notice(
        &self,
        intent: CredentialIntent,
        slot: CredentialSlot,
    ) -> Option<CredentialNotice> {
        let noun = slot.noun();
        match (self, intent) {
            (Self::Stored | Self::Retrieved(_) | Self::Deleted | Self::Cancelled, _) => None,
            (Self::NotFound, CredentialIntent::Load | CredentialIntent::Forget) => None,
            (Self::NotFound, CredentialIntent::Remember) => None,
            (Self::Unavailable, CredentialIntent::Remember) => {
                Some(CredentialNotice::info(format!(
                    "No keyring service is available, so the {noun} was not remembered. \
                     Nothing was written to disk."
                )))
            }
            (Self::Unavailable, CredentialIntent::Load) => Some(CredentialNotice::info(format!(
                "No keyring service is available, so no remembered {noun} could be read. \
                 Type it at the prompt below."
            ))),
            (Self::Unavailable, CredentialIntent::Forget) => None,
            (Self::UnlockDismissed, CredentialIntent::Remember) => {
                Some(CredentialNotice::info(format!(
                    "The keyring stayed locked, so the {noun} was not remembered. \
                     Unlock it and save again to remember it."
                )))
            }
            (Self::UnlockDismissed, CredentialIntent::Load) => {
                Some(CredentialNotice::info(format!(
                    "The keyring stayed locked, so no remembered {noun} was read. \
                     Type it at the prompt below."
                )))
            }
            (Self::UnlockDismissed, CredentialIntent::Forget) => Some(CredentialNotice::info(
                format!("The keyring stayed locked, so the remembered {noun} was kept."),
            )),
            (Self::PermissionDenied, CredentialIntent::Remember) => {
                Some(CredentialNotice::info(format!(
                    "The keyring refused to store the {noun} (permission denied). \
                     The connection itself is unaffected."
                )))
            }
            (Self::PermissionDenied, CredentialIntent::Load) => {
                Some(CredentialNotice::info(format!(
                    "The keyring refused to release the {noun} (permission denied). \
                     Type it at the prompt below."
                )))
            }
            (Self::PermissionDenied, CredentialIntent::Forget) => Some(CredentialNotice::info(
                format!("The keyring refused to remove the remembered {noun} (permission denied)."),
            )),
            (Self::Saturated, CredentialIntent::Remember) => Some(CredentialNotice::info(format!(
                "The keyring is busy. The {noun} is still pending and will retry after an earlier change settles."
            ))),
            (Self::Saturated, CredentialIntent::Forget) => Some(CredentialNotice::info(format!(
                "The keyring is busy. Removing the remembered {noun} is still pending and will retry after an earlier change settles."
            ))),
            (Self::Saturated, CredentialIntent::Load) => Some(CredentialNotice::info(format!(
                "The keyring is busy, so the remembered {noun} could not be read. Retry the connection."
            ))),
            (Self::OwnerShutdown, CredentialIntent::Remember) => {
                Some(CredentialNotice::danger(format!(
                    "The keyring worker stopped before storing the {noun}. Restart Terminal before retrying."
                )))
            }
            (Self::OwnerShutdown, CredentialIntent::Forget) => {
                Some(CredentialNotice::danger(format!(
                    "The keyring worker stopped before removing the remembered {noun}. Restart Terminal before retrying."
                )))
            }
            (Self::OwnerShutdown, CredentialIntent::Load) => {
                Some(CredentialNotice::danger(format!(
                    "The keyring worker stopped before reading the {noun}. Retry the connection after restarting Terminal."
                )))
            }
            (Self::MutationUncertain, CredentialIntent::Remember) => {
                Some(CredentialNotice::danger(format!(
                    "The keyring may have stored the {noun}. Terminal is checking the provider before reporting success."
                )))
            }
            (Self::MutationUncertain, CredentialIntent::Forget) => {
                Some(CredentialNotice::danger(format!(
                    "The keyring may have removed the remembered {noun}. Terminal is checking the provider before continuing."
                )))
            }
            (Self::MutationUncertain, CredentialIntent::Load) => Some(CredentialNotice::danger(
                format!("The keyring could not confirm the {noun} read. Retry the connection."),
            )),
            (Self::Failed(reason), CredentialIntent::Remember) => Some(CredentialNotice::danger(
                format!("The keyring could not store the {noun}: {reason}"),
            )),
            (Self::Failed(reason), CredentialIntent::Load) => Some(CredentialNotice::danger(
                format!("The keyring could not read the {noun}: {reason}"),
            )),
            (Self::Failed(reason), CredentialIntent::Forget) => Some(CredentialNotice::danger(
                format!("The keyring could not remove the {noun}: {reason}"),
            )),
        }
    }
}

/// One retrieved secret in transit through one completion message.
///
/// The app's `Message` is `Clone` (iced clones messages freely), and
/// [`Secret`] deliberately is not: a `Clone` would mint a second copy of the
/// credential with its own drop site. So the secret travels behind a shared,
/// **take-once** cell:
///
/// * cloning the message clones the `Arc`, never the bytes;
/// * [`take`](Self::take) removes the `Secret` from the cell and hands over
///   its wipe-on-drop custody, so a cloned copy of the same message cannot
///   deliver the secret a second time;
/// * `Debug` is redacted and reveals not even the length, matching
///   [`Secret`]'s own rule, so a `{:?}` on a `Message` cannot leak it.
#[derive(Clone)]
pub(crate) struct RetrievedSecret(Arc<Mutex<Option<Secret>>>);

impl RetrievedSecret {
    pub(crate) fn new(secret: Secret) -> Self {
        Self(Arc::new(Mutex::new(Some(secret))))
    }

    /// Take custody of the secret. `None` on the second call: a secret is
    /// delivered once.
    pub(crate) fn take(&self) -> Option<Secret> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }

    /// Compares the retained bytes without creating another plaintext copy.
    pub(crate) fn matches_text(&self, expected: &SecretText) -> bool {
        self.0.lock().is_ok_and(|slot| {
            slot.as_ref()
                .is_some_and(|secret| secret.expose_secret() == expected.expose_secret().as_bytes())
        })
    }

    /// Borrow the bytes without copying them out, for assertions that must
    /// compare a value they never own. Test-only: production code takes the
    /// `Secret` and reads it through `expose_secret`, so no other caller can
    /// grow a habit of peeking.
    #[cfg(test)]
    pub(crate) fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
        let slot = self.0.lock().ok()?;
        slot.as_ref().map(|secret| f(secret.expose_secret()))
    }
}

// Identity, not value: comparing two secrets byte by byte in a derived
// `PartialEq` is a timing oracle and a reason to keep plaintext reachable.
// Two handles are equal when they are the same handle.
impl PartialEq for RetrievedSecret {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RetrievedSecret {}

impl fmt::Debug for RetrievedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RetrievedSecret([redacted])")
    }
}

/// Severity of a credential notice.
///
/// `Info` is load-bearing: a dismissed unlock prompt, a policy refusal and a
/// missing service are *states*, and painting them in the danger ink is how
/// an app tells the user it broke when it did not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoticeTone {
    Info,
    Danger,
}

/// One line of credential feedback in the SSH manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CredentialNotice {
    pub text: String,
    pub tone: NoticeTone,
}

impl CredentialNotice {
    pub(crate) fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: NoticeTone::Info,
        }
    }

    pub(crate) fn danger(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: NoticeTone::Danger,
        }
    }
}

/// The badge a saved-host row shows for its secret, from the *cheap*
/// existence query.
///
/// `contains` never unlocks and never prompts, which is why the manager can
/// call it once per row while painting instead of firing a boot-time `get`
/// storm. [`CredentialPresence::Unknown`] — what a real locked Secret Service
/// returns — is rendered as its own honest state, never as "no password
/// saved".
pub(crate) fn presence_badge(presence: CredentialPresence, slot: CredentialSlot) -> String {
    let noun = slot.noun();
    match presence {
        CredentialPresence::Present => format!("Saved {noun}"),
        CredentialPresence::Absent => format!("No saved {noun}"),
        CredentialPresence::Unknown => format!("Saved {noun} unknown"),
    }
}
