#[cfg(not(test))]
use crate::ssh_config;
use crate::{
    operation_origin::RequestGeneration,
    ssh_config::{OpenSshConfigError, OpenSshHost},
};
use enzui_core::{
    blocking_task::{BlockingTask, BlockingTaskOwner},
    cancellation::CancelCause,
    worker_bridge::{self, WorkerBridgeClose, WorkerReceiver},
};
use std::{
    num::NonZeroUsize,
    sync::{Arc, Condvar, Mutex, OnceLock},
};

pub(crate) type Answer = Result<Vec<OpenSshHost>, OpenSshConfigError>;
#[derive(Clone, Debug)]
pub(crate) struct Completion {
    pub generation: RequestGeneration,
    pub answer: Answer,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Phase {
    AwaitingFrame,
    Loading,
    Ready,
    Unavailable,
    Disabled,
}
struct Slot {
    pending: Option<RequestGeneration>,
    active: Option<RequestGeneration>,
    closed: bool,
}
struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
}
type Loader = Box<dyn FnMut(&RequestGeneration) -> Answer + Send>;
type Owner = Arc<Mutex<BlockingTaskOwner>>;
// The submitted closure is a parked loop, not a request: it holds its worker
// for the process lifetime, so it keeps this dedicated slot instead of the
// shared request pool in `blocking_dispatch`.
fn new_owner() -> Owner {
    Arc::new(Mutex::new(
        BlockingTaskOwner::new(
            "terminal-openssh-discovery",
            NonZeroUsize::MIN,
            NonZeroUsize::MIN,
        )
        .unwrap_or_else(|_| BlockingTaskOwner::unavailable(NonZeroUsize::MIN)),
    ))
}
fn process_owner() -> Owner {
    static OWNER: OnceLock<Owner> = OnceLock::new();
    OWNER.get_or_init(new_owner).clone()
}
struct Worker {
    shared: Arc<Shared>,
    close: WorkerBridgeClose<Completion>,
    owner: Owner,
    task: BlockingTask<()>,
}
impl Worker {
    fn start(
        owner: Owner,
        make_load: impl FnOnce() -> Loader,
    ) -> Result<(Self, WorkerReceiver<Completion>), OpenSshConfigError> {
        let retained = owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let snapshot = retained.dispatcher().snapshot();
        if snapshot.shutdown {
            return Err(OpenSshConfigError::unavailable());
        }
        if snapshot.live() != 0 {
            return Err(OpenSshConfigError::busy());
        }
        let mut load = make_load();
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                pending: None,
                active: None,
                closed: false,
            }),
            wake: Condvar::new(),
        });
        let (sender, receiver, close) = worker_bridge::channel(NonZeroUsize::MIN);
        let worker = shared.clone();
        let task = retained.submit(0, move |context| {
            loop {
                let generation = {
                    let mut slot = worker
                        .slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while slot.pending.is_none() && !slot.closed {
                        slot = worker
                            .wake
                            .wait(slot)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    if slot.closed || context.is_cancelled() {
                        break;
                    }
                    let Some(generation) = slot.pending.take() else {
                        continue;
                    };
                    slot.active = Some(generation.clone());
                    generation
                };
                let answer =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(&generation)))
                        .unwrap_or_else(|_| Err(OpenSshConfigError::unavailable()));
                if generation.is_live()
                    && sender
                        .send_progress(Completion { generation, answer })
                        .is_err()
                {
                    break;
                }
                worker
                    .slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .active = None;
            }
        });
        drop(retained);
        Ok((
            Self {
                shared,
                close,
                owner,
                task,
            },
            receiver,
        ))
    }
    fn request(&self, generation: RequestGeneration) -> Result<(), OpenSshConfigError> {
        let mut slot = self
            .shared
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.closed
            || self
                .owner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .dispatcher()
                .snapshot()
                .live()
                == 0
        {
            return Err(OpenSshConfigError::unavailable());
        }
        if let Some(active) = &slot.active {
            active.retire(CancelCause::Cancelled);
        }
        if let Some(old) = slot.pending.replace(generation) {
            old.retire(CancelCause::Cancelled);
        }
        self.shared.wake.notify_one();
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.close.close();
        let mut slot = self
            .shared
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.closed = true;
        if let Some(active) = &slot.active {
            active.retire(CancelCause::Disconnected);
        }
        if let Some(pending) = slot.pending.take() {
            pending.retire(CancelCause::Disconnected);
        }
        self.shared.wake.notify_one();
        drop(slot);
        // Cancellation settles the task, but the process-lived owner still charges
        // its occupied worker until the blocking reader actually exits.
        self.task.cancellation().cancel();
    }
}

pub(crate) struct Discovery {
    generation: RequestGeneration,
    pub phase: Phase,
    pub redraw_seen: bool,
    pub enabled: bool,
    worker: Option<Worker>,
    owner: Owner,
    #[cfg(test)]
    loader: Option<Loader>,
}
impl Discovery {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            generation: RequestGeneration::new(),
            phase: if enabled {
                Phase::AwaitingFrame
            } else {
                Phase::Disabled
            },
            redraw_seen: !enabled,
            enabled,
            worker: None,
            owner: process_owner(),
            #[cfg(test)]
            loader: None,
        }
    }
    pub(crate) fn generation(&self) -> RequestGeneration {
        self.generation.clone()
    }
    pub(crate) fn is_current(&self, generation: &RequestGeneration) -> bool {
        self.generation == *generation && generation.is_live()
    }
    pub(crate) fn invalidate(&mut self) {
        self.generation.retire(CancelCause::Cancelled);
        self.generation = RequestGeneration::new();
        if self.phase == Phase::Loading {
            self.phase = Phase::Unavailable;
        }
    }
    pub(crate) fn request(
        &mut self,
    ) -> Result<Option<WorkerReceiver<Completion>>, OpenSshConfigError> {
        self.invalidate();
        if !self.redraw_seen {
            return Ok(None);
        }
        self.phase = Phase::Loading;
        let mut receiver = None;
        if self.worker.is_none() {
            #[cfg(test)]
            let result = Worker::start(self.owner.clone(), || {
                self.loader
                    .take()
                    .unwrap_or_else(|| Box::new(|_| Err(OpenSshConfigError::unavailable())))
            });
            #[cfg(not(test))]
            let result = Worker::start(self.owner.clone(), || {
                Box::new(|generation| ssh_config::load_default(&|| generation.is_live()))
            });
            match result {
                Ok((worker, stream)) => {
                    self.worker = Some(worker);
                    receiver = Some(stream);
                }
                Err(error) => {
                    self.phase = Phase::Unavailable;
                    return Err(error);
                }
            }
        }
        let Some(worker) = self.worker.as_ref() else {
            self.phase = Phase::Unavailable;
            return Err(OpenSshConfigError::unavailable());
        };
        let result = worker.request(self.generation.clone());
        if result.is_err() {
            self.phase = Phase::Unavailable;
        }
        result?;
        Ok(receiver)
    }
    pub(crate) fn settle(&mut self, completion: Completion) -> Option<Answer> {
        if !self.is_current(&completion.generation) || self.phase != Phase::Loading {
            return None;
        }
        self.phase = if completion.answer.is_ok() {
            Phase::Ready
        } else {
            Phase::Unavailable
        };
        Some(completion.answer)
    }
    #[cfg(test)]
    pub(crate) fn with_loader(
        loader: impl FnMut(&RequestGeneration) -> Answer + Send + 'static,
    ) -> Self {
        let mut value = Self::new(true);
        value.owner = new_owner();
        value.loader = Some(Box::new(loader));
        value
    }
    #[cfg(test)]
    pub(crate) fn injected(&mut self, ok: bool) {
        self.invalidate();
        self.phase = if ok { Phase::Ready } else { Phase::Unavailable };
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.generation.retire(CancelCause::Disconnected);
    }
}

#[cfg(test)]
#[path = "ssh_discovery_tests.rs"]
mod tests;
