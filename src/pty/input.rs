//! PTY input queue and writer gate.

use super::*;

/// Why one session refused to accept input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputRefusal {
    /// The chunk alone exceeds [`MAX_INPUT_CHUNK_BYTES`].
    ChunkTooLarge,
    /// The queue is at its chunk or byte bound; earlier input is still being
    /// written.
    Busy,
    /// This session's writer is gone, so nothing further can be accepted.
    Closed,
}

impl InputRefusal {
    /// One bounded sentence naming the refusal for a pane banner.
    pub(crate) const fn user_summary(self) -> &'static str {
        match self {
            Self::ChunkTooLarge => "That input is too large to send to the terminal in one go.",
            Self::Busy => "The terminal is still sending earlier input. Try again in a moment.",
            Self::Closed => "The terminal session is closed.",
        }
    }
}

/// The process's input authority.
///
/// One generation per session, so a chunk admitted by one session can never be
/// written by another. A session that cannot be given a generation of its own
/// is refused rather than handed one a live session still stamps chunks with.
pub(crate) static INPUT_GENERATIONS: crate::escape_policy::AuthorityAllocator =
    crate::escape_policy::AuthorityAllocator::new(crate::escape_policy::AuthorityKind::Input);

/// How one piece of input may be admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputAdmission {
    /// One chunk of its own, written whole: a paste, a history recall, a
    /// terminal reply. Refused once the queue holds [`MAX_INPUT_QUEUE_CHUNKS`].
    Whole,
    /// Keys, IME commits and pointer reports. Joins the newest queued chunk
    /// when that chunk is also typed and the writer has not started it, so
    /// ordinary typing behind a starved writer costs bytes, not chunks
    ///. The byte bound still applies.
    Typed,
}

/// Bytes one typed chunk may grow to by joining later typed input.
pub(crate) const MAX_TYPED_CHUNK_BYTES: usize = 4096;

/// The bytes of one queued chunk, shared with the session while it is
/// unstarted so later typed input can join it.
pub(crate) struct ChunkBody {
    bytes: Vec<u8>,
    /// Cleared when the writer takes the chunk; nothing joins it after that.
    open: bool,
}

/// One admitted input chunk, tagged with the generation that admitted it.
///
/// The generation is checked again on the writer thread, so a chunk can only
/// ever be written by the session that accepted it.
pub(crate) struct InputChunk {
    pub(crate) generation: NonZeroU64,
    body: Arc<Mutex<ChunkBody>>,
}

impl InputChunk {
    /// Close the chunk to further typed input and take its bytes.
    fn seal(&self) -> Vec<u8> {
        let mut body = self
            .body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        body.open = false;
        std::mem::take(&mut body.bytes)
    }
}

#[derive(Default)]
struct QueueCharge {
    chunks: usize,
    bytes: usize,
}

/// Chunk and byte accounting for one input queue.
///
/// Admission is decided before the caller's slice is cloned, so a refused write
/// costs no allocation. The lock is held for the accounting only and never
/// across a PTY write.
#[derive(Default)]
pub(crate) struct InputCharge {
    state: Mutex<QueueCharge>,
}

impl InputCharge {
    fn admit(&self, len: usize) -> bool {
        let Ok(mut charge) = self.state.lock() else {
            return false;
        };
        if charge.chunks >= MAX_INPUT_QUEUE_CHUNKS
            || charge.bytes.saturating_add(len) > MAX_INPUT_QUEUE_BYTES
        {
            return false;
        }
        charge.chunks += 1;
        charge.bytes += len;
        true
    }

    /// Charge `len` more bytes to a chunk that is already counted.
    fn admit_bytes(&self, len: usize) -> bool {
        let Ok(mut charge) = self.state.lock() else {
            return false;
        };
        if charge.bytes.saturating_add(len) > MAX_INPUT_QUEUE_BYTES {
            return false;
        }
        charge.bytes += len;
        true
    }

    fn release(&self, len: usize) {
        if let Ok(mut charge) = self.state.lock() {
            charge.chunks = charge.chunks.saturating_sub(1);
            charge.bytes = charge.bytes.saturating_sub(len);
        }
    }

    #[cfg(test)]
    pub(crate) fn queued(&self) -> (usize, usize) {
        self.state
            .lock()
            .map(|charge| (charge.chunks, charge.bytes))
            .unwrap_or_default()
    }
}

/// Holds the writer thread before each write, emulating a child that has
/// stopped reading its PTY without depending on a real stall.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct WriterGate {
    held: Mutex<bool>,
    ready: Condvar,
}

#[cfg(test)]
impl WriterGate {
    pub(crate) fn wait(&self) {
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *held {
            held = self
                .ready
                .wait(held)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub(crate) fn set(&self, hold: bool) {
        if let Ok(mut held) = self.held.lock() {
            *held = hold;
        }
        self.ready.notify_all();
    }
}

/// One session's retained input owner: the bounded queue plus the thread that
/// owns the PTY writer.
///
/// One accepted chunk is written and flushed before the next is taken, so a
/// bracketed paste cannot interleave with a keystroke or with another paste.
/// The generation is minted per session and never reused, so a replacement
/// session has a different channel and a different generation and can never
/// inherit queued input.
///
/// The whole input footprint of one session lives here: the chunk the writer
/// thread is writing plus the chunks still queued behind it, bounded together
/// by [`MAX_INPUT_QUEUE_CHUNKS`] and [`MAX_INPUT_QUEUE_BYTES`].
pub(crate) struct PtyInput {
    pub(crate) generation: NonZeroU64,
    pub(crate) sender: Option<SyncSender<InputChunk>>,
    pub(crate) charge: Arc<InputCharge>,
    /// The newest queued chunk, while it is typed: the only chunk later typed
    /// input may join. Any other admission clears it, so input never
    /// overtakes a chunk queued before it.
    pub(crate) typed_tail: Mutex<Option<Arc<Mutex<ChunkBody>>>>,
    /// Set by teardown before the queue closes: the writer discards what it
    /// has not started, so input queued before a close never reaches a
    /// session that is being terminated.
    pub(crate) closing: Arc<AtomicBool>,
    pub(crate) thread: Option<thread::JoinHandle<()>>,
    pub(crate) joined: Arc<AtomicBool>,
    pub(crate) panicked: bool,
    #[cfg(test)]
    pub(crate) gate: Arc<WriterGate>,
    #[cfg(test)]
    pub(crate) drained: Arc<Mutex<Vec<u8>>>,
}

impl PtyInput {
    /// Admit `bytes` to the queue without blocking.
    ///
    /// # Errors
    ///
    /// Refuses an oversized chunk, a saturated queue, and a closed writer,
    /// always before the caller's slice is cloned.
    pub(crate) fn enqueue(
        &self,
        bytes: &[u8],
        admission: InputAdmission,
    ) -> Result<(), InputRefusal> {
        if bytes.len() > MAX_INPUT_CHUNK_BYTES {
            return Err(InputRefusal::ChunkTooLarge);
        }
        let sender = self.sender.as_ref().ok_or(InputRefusal::Closed)?;
        // Held for the whole admission, so two admissions cannot reorder
        // around the tail.
        let mut tail = self
            .typed_tail
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if admission == InputAdmission::Typed
            && let Some(body) = tail.as_ref()
        {
            // The body lock is held across the charge, so the writer cannot
            // take the chunk between the charge and the append.
            let mut body = body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if body.open && body.bytes.len() + bytes.len() <= MAX_TYPED_CHUNK_BYTES {
                if !self.charge.admit_bytes(bytes.len()) {
                    return Err(InputRefusal::Busy);
                }
                body.bytes.extend_from_slice(bytes);
                return Ok(());
            }
        }
        *tail = None;
        if !self.charge.admit(bytes.len()) {
            return Err(InputRefusal::Busy);
        }
        let body = Arc::new(Mutex::new(ChunkBody {
            bytes: bytes.to_vec(),
            open: true,
        }));
        let chunk = InputChunk {
            generation: self.generation,
            body: Arc::clone(&body),
        };
        match sender.try_send(chunk) {
            Ok(()) => {
                if admission == InputAdmission::Typed {
                    *tail = Some(body);
                }
                Ok(())
            }
            Err(TrySendError::Full(chunk)) => {
                self.charge.release(chunk.seal().len());
                Err(InputRefusal::Busy)
            }
            Err(TrySendError::Disconnected(chunk)) => {
                self.charge.release(chunk.seal().len());
                Err(InputRefusal::Closed)
            }
        }
    }
}

/// Run one session's PTY writer until its queue is closed.
///
/// Chunks are written whole and in order; a chunk from another generation is
/// dropped unwritten. After a write error, or once teardown has set
/// `closing`, the remaining queue is drained and discharged rather than
/// written, so a dying session neither strands its accounting nor receives a
/// paste during its termination grace, and the writer cannot park on a
/// master nobody reads. The writer handle is released when this thread ends,
/// which is why an unjoined writer is teardown debt.
pub(crate) fn run_input_writer(
    mut writer: Box<dyn Write + Send>,
    receiver: Receiver<InputChunk>,
    generation: NonZeroU64,
    charge: Arc<InputCharge>,
    closing: Arc<AtomicBool>,
    #[cfg(test)] gate: Arc<WriterGate>,
    #[cfg(test)] drained: Arc<Mutex<Vec<u8>>>,
) {
    let mut failed = false;
    while let Ok(chunk) = receiver.recv() {
        let bytes = chunk.seal();
        let len = bytes.len();
        if !failed && chunk.generation == generation {
            #[cfg(test)]
            gate.wait();
            if closing.load(Ordering::Acquire) {
                failed = true;
                charge.release(len);
                continue;
            }
            let mut result = writer.write_all(&bytes);
            if result.is_ok() {
                result = writer.flush();
            }
            match result {
                Ok(()) =>
                {
                    #[cfg(test)]
                    if let Ok(mut log) = drained.lock() {
                        log.extend_from_slice(&bytes);
                    }
                }
                Err(_) => failed = true,
            }
        }
        charge.release(len);
    }
}
