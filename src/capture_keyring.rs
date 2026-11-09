//! Capture-only credential fixtures.
//!
//! The three states this app's keyring integration can be in — a remembered
//! secret, a locked keyring whose unlock prompt was dismissed, and no keyring
//! service at all — are all *backend* states. A review screenshot of them
//! cannot be produced by clicking: the reviewer's own keyring decides, and on
//! a headless capture host it is always "no service".
//!
//! So the capture harness scripts the backend, exactly the way
//! `TERMINAL_EMULATOR_CAPTURE_SEED` already scripts the transcript. The store
//! installed here is the framework's public `FakeSecretService`
//! (`enzui_services::credential_store`) behind the *same*
//! `TransportCredentialStore` the real D-Bus transport is wrapped in, so a
//! capture exercises the production reducer path — install store, remember a
//! secret, render whatever outcome came back — and not a painted mock.
//!
//! Inert unless `TERMINAL_EMULATOR_CAPTURE_KEYRING` names a fixture:
//! [`from_env_value`] returns `None` for absent, empty and unknown values, so
//! a normal run never gets a fake keyring.

use std::sync::Arc;

use enzui_services::credential_store::{
    CredentialStore, FakeMode, FakeSecretService, TransportCredentialStore,
    UnavailableCredentialStore,
};

/// Which backend state a capture wants to photograph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureKeyring {
    /// A working keyring holding a remembered password.
    Saved,
    /// A locked keyring whose unlock prompt the user dismissed.
    Locked,
    /// No Secret Service on the bus at all.
    Unavailable,
}

/// The profile id and name the fixture seeds. Fixed so a capture is
/// reproducible.
pub(crate) const FIXTURE_PROFILE_ID: &str = "ssh-host-1";
pub(crate) const FIXTURE_PROFILE_NAME: &str = "Production";
pub(crate) const FIXTURE_DESTINATION: &str = "deploy@prod.example";
/// Deliberately not a real-looking secret: it is typed into a fake daemon that
/// exists for the duration of one screenshot.
pub(crate) const FIXTURE_SECRET: &str = "capture-fixture-secret";

/// Parse the capture env var. Absent, empty or unrecognised ⇒ no fixture.
pub(crate) fn from_env_value(value: Option<&std::ffi::OsStr>) -> Option<CaptureKeyring> {
    match value?.to_str()? {
        "saved" => Some(CaptureKeyring::Saved),
        "locked" => Some(CaptureKeyring::Locked),
        "unavailable" => Some(CaptureKeyring::Unavailable),
        _ => None,
    }
}

impl CaptureKeyring {
    /// The store this fixture installs. `Unavailable` uses the framework's
    /// real `UnavailableCredentialStore` rather than a fake in a scripted
    /// mode: that is literally what a box with no Secret Service gets.
    pub(crate) fn store(self) -> Arc<dyn CredentialStore> {
        match self {
            CaptureKeyring::Saved => Self::fake(FakeMode::Available),
            CaptureKeyring::Locked => Self::fake(FakeMode::LockedDismissed),
            CaptureKeyring::Unavailable => Arc::new(UnavailableCredentialStore),
        }
    }

    fn fake(mode: FakeMode) -> Arc<dyn CredentialStore> {
        let daemon = FakeSecretService::with_mode(mode);
        // The transport holds the daemon alive; the `FakeSecretService` handle
        // itself is not needed once the client exists.
        Arc::new(TransportCredentialStore::new(daemon.transport()))
    }
}
