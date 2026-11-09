//! The one bounded owner for this app's finite synchronous calls.
//!
//! Every synchronous filesystem, durability or descriptor call is submitted
//! here, so none of them occupies the thread that polls PTY output. Two
//! admission behaviours are offered:
//!
//! * [`Dispatch::submit_latest`] is latest wins per [`Latest`] key. An admitted
//!   submission supersedes the key's previous generation, which settles as
//!   [`BlockingTaskError::Cancelled`]; [`Dispatch::owns_latest`] is what proves
//!   a cancellation was that supersession rather than an unexplained stop.
//! * [`Dispatch::submit_fifo`] keeps submission order for independent requests.
//!
//! Both refuse immediately with [`BlockingTaskError::Busy`] once
//! [`QUEUE_CAPACITY`] submissions are queued behind the worker. Every terminal
//! is typed and must reach visible state: a refusal leaves its work owed.
//! Cancellation settles a future before its closure returns, so
//! [`Dispatch::live`] keeps charging the slot until the call really exits.
//!
//! Long-lived loops belong elsewhere: the PTY reader and exit waiter live for
//! their session, and OpenSSH discovery owns a single-slot pool for its parked
//! loop. A loop admitted here would hold the only worker forever.

use enzui_core::blocking_task::{
    BlockingTask, BlockingTaskContext, BlockingTaskError, BlockingTaskOwner, BlockingTaskSnapshot,
};
#[cfg(not(test))]
use std::sync::OnceLock;
use std::{
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// Simultaneously running synchronous calls.
///
/// One worker keeps every durability write serialized, so an older document can
/// never land after a newer one.
pub(crate) const WORKERS: NonZeroUsize = NonZeroUsize::MIN;

/// Submissions admitted behind the running call.
///
/// Three is the number of distinct requests that can be owed at once: the
/// preferences document, the session-state document, and one local link open.
pub(crate) const QUEUE_CAPACITY: NonZeroUsize = NonZeroUsize::new(3).unwrap();

/// A document whose writes coalesce to the newest submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Latest {
    /// Appearance, shortcut overrides and SSH host records.
    Preferences,
    /// Pinned session topology and its secure-storage markers.
    SessionState,
}

// Settles a superseded generation without knowing its value type.
type Retire = Box<dyn FnOnce() + Send>;

/// The stable name of a dispatcher terminal, for diagnostics only.
pub(crate) fn terminal_name(error: BlockingTaskError) -> &'static str {
    match error {
        BlockingTaskError::Busy => "busy",
        BlockingTaskError::Cancelled => "cancelled",
        BlockingTaskError::Shutdown => "shutdown",
        BlockingTaskError::Panicked => "panicked",
        _ => "unrecognised",
    }
}

/// Records one bounded operational line: no path, host or secret may appear.
pub(crate) fn diagnostic(class: &str, terminal: &str, live: usize) {
    eprintln!("terminal-blocking: class={class} terminal={terminal} live={live}");
}

/// The app's bounded blocking-call owner.
pub(crate) struct Dispatch {
    owner: Arc<BlockingTaskOwner>,
    latest: Mutex<Vec<(Latest, u64, Retire)>>,
    next_generation: AtomicU64,
}

impl std::fmt::Debug for Dispatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Dispatch")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl Dispatch {
    fn with_owner(owner: Arc<BlockingTaskOwner>) -> Arc<Self> {
        Arc::new(Self {
            owner,
            latest: Mutex::new(Vec::new()),
            next_generation: AtomicU64::new(1),
        })
    }

    fn create() -> Arc<Self> {
        Self::with_owner(Arc::new(
            BlockingTaskOwner::new("terminal-blocking", WORKERS, QUEUE_CAPACITY)
                .unwrap_or_else(|_| BlockingTaskOwner::unavailable(QUEUE_CAPACITY)),
        ))
    }

    /// The dispatcher every route in this app shares.
    ///
    /// Production keeps one pool for the life of the process; a test build
    /// returns a fresh pool per call so one test's held job cannot saturate
    /// another's.
    pub(crate) fn shared() -> Arc<Self> {
        #[cfg(test)]
        {
            Self::create()
        }
        #[cfg(not(test))]
        {
            static DISPATCH: OnceLock<Arc<Dispatch>> = OnceLock::new();
            DISPATCH.get_or_init(Self::create).clone()
        }
    }

    /// A dispatcher whose every submission settles as shut down.
    #[cfg(test)]
    pub(crate) fn unavailable() -> Arc<Self> {
        Self::with_owner(Arc::new(BlockingTaskOwner::unavailable(QUEUE_CAPACITY)))
    }

    /// Capacities, occupancy and lifetime admission counters.
    pub(crate) fn snapshot(&self) -> BlockingTaskSnapshot {
        self.owner.dispatcher().snapshot()
    }

    /// Submissions still holding a worker or queue slot.
    ///
    /// Cancellation settles a future before its closure returns, so this stays
    /// charged until the synchronous call actually exits. Close settlement must
    /// read this rather than the app's own in-flight bookkeeping.
    pub(crate) fn live(&self) -> usize {
        self.snapshot().live()
    }

    /// Whether `generation` is still the newest submission of `key`.
    ///
    /// A false answer for a cancelled generation is the proof that a newer
    /// write took the key over and now owns its debt.
    pub(crate) fn owns_latest(&self, key: Latest, generation: u64) -> bool {
        let slots = self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots
            .iter()
            .any(|(slot, stamp, _)| *slot == key && *stamp == generation)
    }

    fn stamp(&self) -> u64 {
        self.next_generation.fetch_add(1, Ordering::Relaxed)
    }

    /// Submits one synchronous call, preserving submission order.
    pub(crate) fn submit_fifo<T, F>(&self, work: F) -> BlockingTask<T>
    where
        T: Send + 'static,
        F: FnOnce(BlockingTaskContext) -> T + Send + 'static,
    {
        let _admission = self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.owner.submit(self.stamp(), work)
    }

    /// Submits one synchronous call as the newest write of `key`, returning it
    /// with the generation [`Dispatch::owns_latest`] answers for.
    ///
    /// The key's previous generation is retired only once this submission is
    /// admitted, so a refusal leaves the older write running and authoritative.
    pub(crate) fn submit_latest<T, F>(&self, key: Latest, work: F) -> (BlockingTask<T>, u64)
    where
        T: Send + 'static,
        F: FnOnce(BlockingTaskContext) -> T + Send + 'static,
    {
        let mut slots = self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.stamp();
        let accepted = self.snapshot().accepted;
        let task = self.owner.submit(generation, work);
        if self.snapshot().accepted == accepted {
            return (task, generation);
        }
        let cancellation = task.cancellation();
        let retire: Retire = Box::new(move || cancellation.cancel());
        let superseded = match slots.iter_mut().find(|(slot, ..)| *slot == key) {
            Some(slot) => {
                slot.1 = generation;
                Some(std::mem::replace(&mut slot.2, retire))
            }
            None => {
                slots.push((key, generation, retire));
                None
            }
        };
        drop(slots);
        if let Some(superseded) = superseded {
            superseded();
        }
        (task, generation)
    }
}
