//! Pinned storage machinery: close debt, secure pins, and the
//! session-state write pipeline.

use crate::blocking_dispatch::{Dispatch, Latest};
use crate::operation_origin::RequestGeneration;
use crate::pinned_secret::{self, Marker, Reference};
use crate::session_store::{SavedSession, SessionState};
use enzui_core::blocking_task::BlockingTaskError;
use enzui_core::cancellation::CancelCause;
use enzui_core::close_guard::CloseGuard;
use enzui_core::persist::Store;
use enzui_services::credential_store::{
    CancelHandle, CredentialOutcome, CredentialStore, PendingCredentialOp,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// How long a close waits for owed durable state before it is vetoed.
pub(super) const CLOSE_SETTLE_BUDGET: Duration = Duration::from_secs(2);

// How often a waiting close re-reads its debt.
pub(super) const CLOSE_TICK: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Unavailable,
    Denied,
    Locked,
    Cancelled,
    Invalid,
    Transport,
    Metadata,
    Busy,
    Interrupted,
    Worker,
}
impl Failure {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Secret Service is unavailable.",
            Self::Denied => "Secret Service denied access.",
            Self::Locked => "The keyring remained locked.",
            Self::Cancelled => "The secure operation was cancelled.",
            Self::Invalid => "The secure snapshot is missing, stale, corrupt or oversized.",
            Self::Transport => "Secret Service did not complete the operation.",
            Self::Metadata => "The metadata checkpoint could not be published.",
            Self::Busy => "Saving is busy; this change has not been written yet.",
            Self::Interrupted => "Saving stopped before this change was written.",
            Self::Worker => "The background saver failed before this change was written.",
        }
    }
    /// Whether the change is still owed and a retry can publish it.
    pub(crate) fn owes_durable_state(self) -> bool {
        matches!(
            self,
            Self::Metadata | Self::Busy | Self::Interrupted | Self::Worker
        )
    }
}
pub(super) fn failure(outcome: &CredentialOutcome) -> Failure {
    match outcome {
        CredentialOutcome::Unavailable => Failure::Unavailable,
        CredentialOutcome::PermissionDenied => Failure::Denied,
        CredentialOutcome::UnlockCancelled => Failure::Locked,
        CredentialOutcome::Cancelled => Failure::Cancelled,
        CredentialOutcome::Saturated => Failure::Busy,
        CredentialOutcome::OwnerShutdown => Failure::Worker,
        CredentialOutcome::MutationUncertain => Failure::Transport,
        _ => Failure::Transport,
    }
}
#[derive(Default)]
pub(super) struct JobState {
    pub(super) cancel: Option<CancelHandle>,
    pub(super) finished: bool,
    pub(super) abandoned: bool,
}
pub(super) struct Lease {
    pub(super) info: Info,
    pub(super) generation: RequestGeneration,
    pub(super) state: Arc<Mutex<JobState>>,
}
impl Lease {
    pub(super) fn cancel(&self) {
        if let Ok(state) = self.state.lock()
            && !state.finished
        {
            self.generation.retire(CancelCause::Cancelled);
            if let Some(cancel) = &state.cancel {
                cancel.cancel();
            }
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) struct WorkerGuard {
    pub(super) generation: RequestGeneration,
    pub(super) state: Arc<Mutex<JobState>>,
    pub(super) complete: bool,
}
impl WorkerGuard {
    pub(super) fn finish(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.finished = true;
        }
        self.complete = true;
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if !self.complete
            && let Ok(mut state) = self.state.lock()
        {
            state.abandoned = true;
            self.generation.retire(CancelCause::Cancelled);
            if let Some(cancel) = &state.cancel {
                cancel.cancel();
            }
        }
    }
}

/// What a close is still waiting for, in user-visible terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseDebt {
    /// A session or preferences publication is still running.
    Publishing,
    /// Submitted calls still occupy the bounded pool.
    Draining,
    /// The last attempt to write owed state failed.
    Failed,
    // Accepted keyring mutations or reconciliation are still settling.
    CredentialMutations,
    // Provider state for at least one exact credential key is uncertain.
    CredentialUncertain,
    /// A closed session's processes, workers or descriptors are still owned.
    TeardownIncomplete,
}
impl CloseDebt {
    /// Bounded close-banner copy. It never names a path, host or secret.
    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Publishing => "Saved session data is still being written.",
            Self::Draining => "Background saving has not finished.",
            Self::Failed => "Session data could not be saved.",
            Self::CredentialMutations => "SSH credential changes have not settled.",
            Self::CredentialUncertain => {
                "SSH credential state is uncertain. Keep Terminal open and retry reconciliation."
            }
            Self::TeardownIncomplete => {
                "Terminal processes from a closed session have not finished shutting down. \
                 Keep Terminal open and retry cleanup."
            }
        }
    }
    pub(super) fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::CredentialUncertain)
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Publishing => "publishing",
            Self::Draining => "draining",
            Self::Failed => "failed",
            Self::CredentialMutations => "credential-mutations",
            Self::CredentialUncertain => "credential-uncertain",
            Self::TeardownIncomplete => "teardown-incomplete",
        }
    }
    // Stable identity of the item a guarded close is holding for.
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Publishing => "session-publication",
            Self::Draining => "background-save",
            Self::Failed => "unsaved-session-state",
            Self::CredentialMutations => "credential-mutations",
            Self::CredentialUncertain => "credential-uncertainty",
            Self::TeardownIncomplete => "pty-teardown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloseStage {
    Draining { deadline: Instant },
    Vetoed { debt: CloseDebt },
}

// One close request, held until its durable debt settles or the user decides.
pub(super) struct Closing {
    // Every window that was open when the request arrived, including the one it
    // named. None of them is destroyed before the close commits.
    pub(super) windows: Vec<iced::window::Id>,
    pub(super) requested: iced::window::Id,
    pub(super) stage: CloseStage,
    pub(super) attempts: u32,
    // A destroyed surface cannot show a veto, so teardown never claims one.
    pub(super) vetoable: bool,
    // The debt class already reported, so a held close reports each reason once
    // instead of once per reducer turn.
    pub(super) reported: Option<CloseDebt>,
}

pub(crate) struct SecurePins {
    pub(crate) capture_mode: bool,
    pub(crate) references: HashMap<String, Reference>,
    pub(crate) pending_deletes: Vec<Reference>,
    pub(super) restore_pending: HashSet<String>,
    pub(super) clear_requested: HashSet<String>,
    pub(super) failed: HashMap<String, Failure>,
    pub(super) metadata_failed: bool,
    pub(super) in_flight: Option<Lease>,
    pub(super) workspace: RequestGeneration,
    pub(super) closing: Option<Closing>,
    // The shared interception authority: only its Close action may produce a
    // window-close task.
    pub(super) guard: CloseGuard<&'static str>,
}
impl Default for SecurePins {
    fn default() -> Self {
        Self {
            capture_mode: false,
            references: HashMap::new(),
            pending_deletes: Vec::new(),
            restore_pending: HashSet::new(),
            clear_requested: HashSet::new(),
            failed: HashMap::new(),
            metadata_failed: false,
            in_flight: None,
            workspace: RequestGeneration::new(),
            closing: None,
            guard: CloseGuard::new(),
        }
    }
}
impl SecurePins {
    pub(crate) fn new(capture_mode: bool) -> Self {
        Self {
            capture_mode,
            ..Self::default()
        }
    }
    pub(crate) fn adopt(&mut self, state: &SessionState) {
        if let Some(lease) = &self.in_flight {
            lease.cancel();
        }
        self.workspace.retire(CancelCause::Disconnected);
        self.workspace = RequestGeneration::new();
        self.references = state
            .sessions
            .iter()
            .filter_map(|s| s.archive.clone().map(|r| (s.id.clone(), r)))
            .collect();
        self.pending_deletes = state.pending_deletes.clone();
        for reference in self
            .references
            .values()
            .filter(|reference| reference.marker.is_none())
        {
            if !self
                .pending_deletes
                .iter()
                .any(|item| item.key == reference.key)
            {
                self.pending_deletes.push(reference.clone());
            }
        }
        self.clear_requested.clear();
        self.restore_pending = state
            .sessions
            .iter()
            .filter(|s| s.archive.as_ref().is_some_and(|r| r.marker.is_some()))
            .map(|s| s.id.clone())
            .collect();
        self.failed.clear();
        self.metadata_failed = false;
    }
    pub(crate) fn retry(&mut self) {
        self.failed.clear();
        self.metadata_failed = false;
    }
    pub(crate) fn backend_ready(&mut self) {
        self.failed
            .retain(|_, failure| *failure != Failure::Unavailable);
    }
    #[cfg(test)]
    pub(crate) fn cancel(&self) {
        if let Some(lease) = &self.in_flight {
            lease.cancel();
        }
    }
    pub(crate) fn failed(&self) -> bool {
        self.metadata_failed || !self.failed.is_empty()
    }
    /// Why pinned output is not stored securely, when that is a failure the
    /// user can act on (a pin whose output was never stored is none).
    pub(crate) fn failure_notice(&self) -> Option<&'static str> {
        if self.capture_mode {
            return None;
        }
        if self.metadata_failed {
            Some(Failure::Metadata.label())
        } else if let Some(failure) = self.failed.values().next() {
            Some(failure.label())
        } else if !self.pending_deletes.is_empty() || !self.clear_requested.is_empty() {
            Some("Secure output removal is pending.")
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Info {
    Metadata,
    Save {
        owner: String,
        reference: Reference,
        marker: Marker,
    },
    Delete(Reference),
    Load(SavedSession),
}
pub(super) enum Work {
    Metadata,
    Save {
        info: Info,
        secret: enzui_services::credential_store::Secret,
    },
    Delete(Reference),
    Load(SavedSession),
}
impl Work {
    pub(super) fn info(&self) -> Info {
        match self {
            Self::Metadata => Info::Metadata,
            Self::Save { info, .. } => info.clone(),
            Self::Delete(reference) => Info::Delete(reference.clone()),
            Self::Load(session) => Info::Load(session.clone()),
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Completion {
    pub(super) generation: RequestGeneration,
    pub(super) workspace: RequestGeneration,
    pub(super) info: Info,
    pub(super) published: Option<SessionState>,
    pub(super) failure: Option<Failure>,
    pub(super) restored: Option<pinned_secret::Restored>,
}
pub(super) fn key(info: &Info) -> Option<&str> {
    match info {
        Info::Save { reference, .. } | Info::Delete(reference) => Some(&reference.key),
        Info::Load(session) => session.archive.as_ref().map(|r| r.key.as_str()),
        Info::Metadata => None,
    }
}
pub(super) fn pending(state: &mut SessionState, reference: &Reference) {
    if !state
        .pending_deletes
        .iter()
        .any(|item| item.key == reference.key)
    {
        state.pending_deletes.push(reference.clone());
    }
}
fn deleted(state: &mut SessionState, reference: &Reference) {
    state
        .pending_deletes
        .retain(|item| item.key != reference.key);
    for session in &mut state.sessions {
        if let Some(item) = &mut session.archive
            && item.key == reference.key
        {
            item.marker = None;
        }
    }
}
async fn run_op(
    op: PendingCredentialOp,
    generation: &RequestGeneration,
    state: &Mutex<JobState>,
) -> CredentialOutcome {
    if let Ok(mut state) = state.lock() {
        if !generation.is_live() {
            return CredentialOutcome::Cancelled;
        }
        state.cancel = Some(op.cancel_handle());
    } else {
        return CredentialOutcome::Cancelled;
    }
    let outcome = op.await;
    if let Ok(mut state) = state.lock() {
        state.cancel = None;
    }
    outcome
}
// The outcome of one session-state write.
enum Published {
    Yes,
    // A newer write of the same document took the key over and owns the debt.
    Superseded,
    Failed(Failure),
}
async fn publish(
    blocking: &Dispatch,
    store: &Arc<Store<SessionState>>,
    document: &SessionState,
) -> Published {
    let owned_store = Arc::clone(store);
    let owned_document = document.clone();
    let (work, generation) = blocking.submit_latest(Latest::SessionState, move |_| {
        owned_store.save_blocking(&owned_document)
    });
    let error = match work.await.into_result() {
        Ok(Ok(())) => return Published::Yes,
        Ok(Err(_)) => return Published::Failed(Failure::Metadata),
        Err(error) => error,
    };
    crate::blocking_dispatch::diagnostic(
        "session-state",
        crate::blocking_dispatch::terminal_name(error),
        blocking.live(),
    );
    match error {
        BlockingTaskError::Busy => Published::Failed(Failure::Busy),
        BlockingTaskError::Cancelled if !blocking.owns_latest(Latest::SessionState, generation) => {
            Published::Superseded
        }
        BlockingTaskError::Panicked => Published::Failed(Failure::Worker),
        _ => Published::Failed(Failure::Interrupted),
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "The worker receives captured capabilities and both cancellation generations"
)]
pub(super) async fn run(
    work: Work,
    mut document: SessionState,
    store: Arc<Store<SessionState>>,
    backend: Arc<dyn CredentialStore>,
    generation: RequestGeneration,
    workspace: RequestGeneration,
    job: Arc<Mutex<JobState>>,
    blocking: Arc<Dispatch>,
) -> Completion {
    let info = work.info();
    let mut completion = Completion {
        generation: generation.clone(),
        workspace: workspace.clone(),
        info: info.clone(),
        published: None,
        failure: None,
        restored: None,
    };
    if !workspace.is_live() {
        completion.failure = Some(Failure::Cancelled);
        return completion;
    }
    if !matches!(work, Work::Load(_)) {
        // Reserve the stable key (and deletion intent) without claiming a stored payload.
        match publish(&blocking, &store, &document).await {
            Published::Yes => completion.published = Some(document.clone()),
            Published::Superseded => {
                completion.failure = Some(Failure::Cancelled);
                return completion;
            }
            Published::Failed(failure) => {
                completion.failure = Some(failure);
                return completion;
            }
        }
    }
    let outcome = match work {
        Work::Metadata => None,
        Work::Save {
            secret,
            info: Info::Save { reference, .. },
        } => Some(
            run_op(
                backend.set(reference.credential_key(), secret),
                &generation,
                &job,
            )
            .await,
        ),
        Work::Save { .. } => {
            completion.failure = Some(Failure::Invalid);
            None
        }
        Work::Delete(reference) => Some(
            run_op(
                backend.delete(reference.credential_key()),
                &generation,
                &job,
            )
            .await,
        ),
        Work::Load(session) => match session.archive {
            Some(reference) => {
                Some(run_op(backend.get(reference.credential_key()), &generation, &job).await)
            }
            None => {
                completion.failure = Some(Failure::Invalid);
                None
            }
        },
    };
    match (&info, outcome) {
        (
            Info::Save {
                owner,
                reference,
                marker,
            },
            Some(CredentialOutcome::Stored),
        ) => {
            if let Some(session) = document.sessions.iter_mut().find(|s| &s.id == owner) {
                session.archive = Some(Reference {
                    key: reference.key.clone(),
                    marker: Some(marker.clone()),
                });
            }
        }
        (
            Info::Delete(reference),
            Some(CredentialOutcome::Deleted | CredentialOutcome::NotFound),
        ) => deleted(&mut document, reference),
        (Info::Load(saved), Some(CredentialOutcome::Retrieved(secret))) => {
            let Some(reference) = saved.archive.clone() else {
                completion.failure = Some(Failure::Invalid);
                return completion;
            };
            let saved = saved.clone();
            let decode =
                blocking.submit_fifo(move |_| pinned_secret::open(&secret, &saved, &reference));
            match decode.await.into_result() {
                Ok(Ok(restored)) => completion.restored = Some(restored),
                Ok(Err(())) => completion.failure = Some(Failure::Invalid),
                Err(error) => {
                    crate::blocking_dispatch::diagnostic(
                        "pinned-decode",
                        crate::blocking_dispatch::terminal_name(error),
                        blocking.live(),
                    );
                    completion.failure = Some(match error {
                        BlockingTaskError::Busy => Failure::Busy,
                        BlockingTaskError::Panicked => Failure::Worker,
                        _ => Failure::Interrupted,
                    });
                }
            }
        }
        (Info::Load(_), Some(CredentialOutcome::NotFound)) => {
            completion.failure = Some(Failure::Invalid);
        }
        (_, Some(outcome)) => completion.failure = Some(failure(&outcome)),
        _ => {}
    }
    // Cancellation is linearized against publication; no mutex is held across a credential await.
    let cancelled = job.lock().map_or(true, |state| {
        !generation.is_live() || !workspace.is_live() || state.finished
    });
    if cancelled {
        completion.restored = None;
        completion.failure = Some(Failure::Cancelled);
    }
    if matches!(
        completion.failure,
        Some(Failure::Cancelled | Failure::Transport)
    ) && let Info::Save { reference, .. } = &info
    {
        pending(&mut document, reference);
        match backend.delete(reference.credential_key()).await {
            CredentialOutcome::Deleted | CredentialOutcome::NotFound => {
                deleted(&mut document, reference);
            }
            _ => {}
        }
    }
    if !matches!(info, Info::Load(_)) && workspace.is_live() {
        let mut final_document = false;
        if let Ok(mut state) = job.lock() {
            if !generation.is_live() && completion.failure.is_none() {
                completion.failure = Some(Failure::Cancelled);
                if let Info::Save { reference, .. } = &info {
                    pending(&mut document, reference);
                    for session in &mut document.sessions {
                        if let Some(current) = &mut session.archive
                            && current.key == reference.key
                        {
                            current.marker = reference.marker.clone();
                        }
                    }
                }
            }
            // The document is settled before the write leaves this turn, so a
            // cancellation racing the worker can no longer unpublish it.
            state.finished = true;
            final_document = true;
        }
        // A document identical to the one already published (a metadata-only
        // save, or a credential step that changed nothing) is not written
        // twice.
        if final_document && completion.published.as_ref() != Some(&document) {
            match publish(&blocking, &store, &document).await {
                Published::Yes => completion.published = Some(document),
                Published::Superseded => completion.failure = Some(Failure::Cancelled),
                Published::Failed(failure) => completion.failure = Some(failure),
            }
        }
    } else if let Ok(mut state) = job.lock() {
        state.finished = true;
    }
    completion
}
