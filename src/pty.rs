// No module-level `#![allow(dead_code)]`. The items below that really
// are unused carry a targeted marker with a reason.
//! PTY / VT services — the portable-pty + termwiz integration (hard user
//! requirement: keep termwiz + portable-pty, do not reinvent the terminal
//! core).
//!
//! Architecture:
//!   * `PtySession` owns both halves of a `portable_pty::PtyPair`, a bounded
//!     input queue and the writer thread behind it, shared child-exit state, and
//!     an `Arc<Mutex<emulator::Screen>>`.
//!   * A background thread reads the master reader and feeds bytes into the
//!     termwiz `Screen`. The fd is expected to be blocking, so the thread parks
//!     in the kernel between bursts and costs zero CPU while idle, pinned by
//!     `pty_master_reader_fd_is_blocking`, which fails if a `portable-pty`
//!     change ever hands over an `O_NONBLOCK` master. Because a non-blocking fd
//!     cannot be ruled out forever, the loop's `WouldBlock` arm sleeps
//!     [`WOULD_BLOCK_BACKOFF`] instead of retrying immediately: an unthrottled
//!     `continue` on a non-blocking fd spins one core at 100% forever and
//!     starves the UI thread. `Interrupted` is a genuine, self-limiting retry
//!     and deliberately does not back off.
//!   * A second background thread waits for child exit and publishes the exit
//!     detail before ringing the wake bus. On Linux it observes with
//!     `waitid(WNOWAIT)` and defers reaping until shutdown, keeping the process
//!     leader identity reserved while a naturally exited tab remains open. Child
//!     status and PTY readability are distinct events; a one-shot `try_wait` on
//!     an output wake can run too early and never run again.
//!   * The iced app does not poll on a timer. Every reader thread rings
//!     [`wake_bus()`] after it has fed bytes into a `Screen`; child waiters ring
//!     the same bus after publishing exit state. The app hosts a [`wake_stream`]
//!     subscription that awaits that bell and only then emits a drain message,
//!     which calls `take_snapshot()` and reprojects the screen into
//!     `Vec<TranscriptLine>` for `update` to install on the active pane. An idle
//!     PTY therefore produces zero drains and zero repaints, where a fixed 33 ms
//!     `iced::time::every` would produce ~30 a second forever. `wake_bus()` is
//!     process-global on purpose: the bell is edge-triggered "some pane has new
//!     bytes", and `drain_ptys` already visits every pane, so one bell for N
//!     panes is exactly one drain rather than N.
//!   * Keystrokes go the other way: the terminal grid captures key events and
//!     `PtySession::write` hands bytes to this session's retained input writer
//!     thread through a bounded queue; the reducer never blocks on the PTY. A
//!     session's entire input memory is the one chunk that writer is writing
//!     plus the chunks still queued behind it, capped together by
//!     [`MAX_INPUT_QUEUE_CHUNKS`] and [`MAX_INPUT_QUEUE_BYTES`].
//!
//! termwiz is the only VT backend.

#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::num::NonZeroU64;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(all(test, target_os = "linux"))]
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use iced::futures::task::AtomicWaker;

#[cfg(not(target_os = "linux"))]
use portable_pty::ChildKiller;
use portable_pty::{CommandBuilder, MasterPty, PtyPair, PtySize, SlavePty};
#[cfg(target_os = "linux")]
use rustix::fd::AsFd;

use crate::emulator;
use crate::model::{SshAuthMode, SshHostRecord, TranscriptRow};
use crate::shell_history::BASH_PROMPT_COMMAND;

#[cfg(target_os = "linux")]
#[path = "pty_capability.rs"]
mod capability;
#[cfg(target_os = "linux")]
use capability::{PtyCapability, wrap_with_capability};

/// Bytes-per-read cap. The reader thread reads freely; the poll just takes the
/// latest projection each tick.
pub(crate) const READ_CHUNK: usize = 4096;

/// Default grid geometry we ask the child shell to render into before the first
/// real resize from the measured cell size.
pub(crate) const DEFAULT_ROWS: u16 = 30;
pub(crate) const DEFAULT_COLS: u16 = 120;

/// Backoff applied when the master reader reports [`std::io::ErrorKind::WouldBlock`]
///.
///
/// On a blocking fd — what this module expects and what
/// `pty_master_reader_fd_is_blocking` pins — this path is never taken, so the
/// cost is zero. On a non-blocking fd it is the difference between parking the
/// reader thread ~200 times a second and pegging a whole core in a tight
/// `read()` → `WouldBlock` → `continue` loop. 5 ms is far below the 33 ms UI
/// poll cadence, so a live-output latency cost is not observable.
pub(crate) const WOULD_BLOCK_BACKOFF: Duration = Duration::from_millis(5);

/// Bytes one accepted input chunk may carry.
///
/// The largest legitimate chunk is a normalised paste plus its bracketed-paste
/// envelope, so this is derived from the paste cap rather than chosen again.
pub(crate) const MAX_INPUT_CHUNK_BYTES: usize =
    crate::escape_policy::MAX_PASTE_BYTES + BRACKETED_ENVELOPE_BYTES;

/// Bytes the two bracketed-paste guards add to a payload.
const BRACKETED_ENVELOPE_BYTES: usize = 12;

/// Chunks one session's input queue may hold before it refuses.
///
/// Counted from admission until the write completes, so the chunk the writer
/// thread is actively writing is still one of these.
pub(crate) const MAX_INPUT_QUEUE_CHUNKS: usize = 8;

/// Bytes one session's input queue may hold across all queued chunks.
///
/// This is the whole input footprint of one session, four maximum pastes: the
/// active chunk the writer holds plus the chunks queued behind it are charged
/// against the same total.
pub(crate) const MAX_INPUT_QUEUE_BYTES: usize = 4 * MAX_INPUT_CHUNK_BYTES;

/// Grace period for a PTY process session to handle an orderly termination.
const SHUTDOWN_TERM_GRACE: Duration = Duration::from_millis(250);
/// Maximum wait for captured descendants to disappear after an uncatchable signal.
const SHUTDOWN_KILL_GRACE: Duration = Duration::from_secs(3);
/// Bounded wait for one worker thread before its handle is parked.
const SHUTDOWN_JOIN_GRACE: Duration = Duration::from_millis(500);
/// Grace for a retry pass, which runs on a reducer tick rather than once at
/// close, so it must not spend the first pass's budget again.
const RETRY_KILL_GRACE: Duration = Duration::from_millis(100);
const RETRY_JOIN_GRACE: Duration = Duration::from_millis(50);
/// Poll interval while waiting for a worker to finish.
const WORKER_JOIN_POLL: Duration = Duration::from_millis(5);
/// Live PTY sessions plus unresolved teardowns this process may own at once.
#[cfg(not(test))]
const TEARDOWN_REAPER_CAPACITY: usize = 128;
/// The test binary runs about a thousand tests in parallel, hundreds of which
/// spawn real PTYs, faster than the single executor reaps them: at the
/// production bound the process-wide supervisor filled mid-run (101 owners
/// queued) and unrelated tests failed to spawn. Capacity refusal itself is
/// exercised on private `TeardownSupervisor::for_test` instances.
#[cfg(test)]
const TEARDOWN_REAPER_CAPACITY: usize = 1024;
/// Owners one executor round may claim at once, which bounds how much work a
/// single wake performs.
///
/// One, because an owner whose pass is still incomplete goes to the back of the
/// queue: a newly retired session then waits at most one other owner's pass
/// rather than a whole batch of them.
#[cfg(test)]
const TEARDOWN_BATCH: usize = 1;
/// Owners the executor admits at once. A refused owner stays with its caller,
/// visible as that caller's own debt, rather than joining an unbounded backlog.
const TEARDOWN_ADMISSION: usize = 8;
/// Pause after an executor round that settled nothing, so a stuck owner cannot
/// turn repeated passes into a busy loop.
const TEARDOWN_ROUND_PAUSE: Duration = Duration::from_millis(250);
/// Unwound passes one owner may take before it leaves the rotation. It stays
/// owned and stays charged; it simply stops being retried.
const TEARDOWN_PANIC_LIMIT: u32 = 3;
/// How long the executor parks when it has nothing to do.
const TEARDOWN_IDLE_PARK: Duration = Duration::from_millis(500);
/// Handshake emitted after the deterministic exit probe has disabled tty echo.
#[cfg(test)]
pub(crate) const EXIT_PROBE_READY: &str = "enzui-exit-probe-ready";

/// Markers emitted by the deterministic one-byte input probe.
#[cfg(test)]
pub(crate) const INPUT_PROBE_READY: &str = "enzui-input-probe-ready";
#[cfg(test)]
pub(crate) const INPUT_PROBE_ESCAPE: &str = "enzui-input-probe-byte-27";

#[path = "pty/input.rs"]
mod input;
pub(crate) use input::InputAdmission;
pub(crate) use input::InputRefusal;
#[cfg(test)]
pub(crate) use input::WriterGate;
pub(crate) use input::{INPUT_GENERATIONS, InputCharge, InputChunk, PtyInput, run_input_writer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CustodyState {
    Held,
    InOwner,
    Moved,
    Lost,
}

/// What identifies a live target: the view's screen id, focus, cursor
/// style, colour scheme and rows above the emulator's.
type LiveKey = (u64, bool, String, &'static str, u16);

/// One live PTY-backed session (one per pane).
pub(crate) struct PtySession {
    pub session_id: String,
    input: Option<PtyInput>,
    #[cfg(unix)]
    shell_startup: Option<crate::shell_history::local::Startup>,
    screen: Arc<Mutex<emulator::Screen>>,
    dirty: Arc<AtomicU64>,
    last_seen: AtomicU64,
    /// The live target last handed to the screen, so an unchanged frame does
    /// not take the screen lock.
    live: Mutex<Option<LiveKey>>,
    // A test-only tap on the WRITER, so a gate can drive a real key
    // through `App::update` and assert the exact bytes that reached the PTY's
    // input — the level at which arrow keys were lost. Recording on write is
    // deterministic whatever the child does with them.
    #[cfg(test)]
    recorded_input: Mutex<Vec<u8>>,
    /// The master half, retained for resize and for a stable master identity
    /// until both workers have joined.
    master: Option<Box<dyn MasterPty + Send>>,
    /// Our copy of the slave half, released at the start of termination.
    ///
    /// It is held separately from the master because while any slave fd is
    /// open, a master write to a child that stopped reading blocks in the
    /// kernel with no timeout and no cancellation. Closing the last slave
    /// holder is what turns that write into an error return, which is how the
    /// writer thread becomes joinable at all.
    slave: Option<Box<dyn SlavePty + Send>>,
    child_exit: Arc<Mutex<Option<ChildExit>>>,
    // A secret channel makes `sshpass` the leader, so the published status is
    // sshpass's own rather than ssh's.
    secret_delivery: bool,
    #[cfg(target_os = "linux")]
    process_identity: LinuxProcessIdentity,
    #[cfg(target_os = "linux")]
    reap_request: Arc<(Mutex<bool>, Condvar)>,
    #[cfg(target_os = "linux")]
    reader_cancel: Option<filedescriptor::FileDescriptor>,
    #[cfg(not(target_os = "linux"))]
    child_killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    reader_thread: Option<thread::JoinHandle<()>>,
    exit_thread: Option<thread::JoinHandle<()>>,
    #[cfg(test)]
    reader_done: Arc<AtomicBool>,
    #[cfg(any(test, not(target_os = "linux")))]
    exit_done: Arc<AtomicBool>,
    reader_joined: Arc<AtomicBool>,
    exit_joined: Arc<AtomicBool>,
    // Set only by an observed wait result — this process's own reap, or an
    // `ECHILD` proving somebody completed it. A vanished `/proc` entry is not
    // proof and never sets this.
    leader_reaped: Arc<AtomicBool>,
    // Reserved before the child existed and never moved out, so an unresolved
    // teardown is always charged and never needs an unreserved fallback.
    teardown_token: TeardownToken,
    _reaper_slot: ReaperReservation,
    // Only ever set once the teardown is complete: an incomplete pass must stay
    // retryable rather than cache itself as terminal.
    teardown_outcome: Option<PtyTeardownOutcome>,
    // Descendant pidfds retained across passes, so a retry keeps the exact
    // cleanup authority the first pass discovered.
    #[cfg(target_os = "linux")]
    owned: Vec<OwnedSessionMember>,
    // Where this session's teardown custody is. The fallback `Drop` reads it to
    // tell "still mine" from "already handed on" from "an owner around me is
    // unwinding".
    custody: CustodyState,
    terminate_started: bool,
    settled_once: bool,
    freeze_stable: bool,
    freeze_scans: u64,
    reader_panicked: bool,
    waiter_panicked: bool,
    // Withholds leader settlement so a route control can exercise real owner
    // custody without depending on a genuinely stuck process.
    #[cfg(test)]
    force_incomplete: bool,
    // Unwinds the next pass once, so a control can observe the executor's
    // owner-boundary recovery without arranging a real teardown fault.
    #[cfg(test)]
    force_sweep_panic: bool,
    // sshpass opens the memfd path lazily, so reserve its fd number through child
    // teardown and both worker joins.
    #[cfg(target_os = "linux")]
    _password_secret: Option<PasswordSecret>,
    // A timed-out child can still open its /proc handoff path; reserve the fd number
    // without granting ownership.
    #[cfg(target_os = "linux")]
    _capability_handoff: Option<Arc<std::fs::File>>,
    #[cfg(test)]
    final_freeze_scans: Arc<AtomicU64>,
    #[cfg(test)]
    final_freeze_stable: Arc<AtomicBool>,
}

#[path = "pty/identity.rs"]
mod identity;
pub(crate) use identity::{
    BorrowedMasterFd, LinuxProcStat, LinuxProcessIdentity, LinuxReaderControl, OwnedSessionMember,
};
#[cfg(all(test, target_os = "linux"))]
pub(crate) use identity::{
    MemfdIdentity, PtySetupFailure, PtySpawnTestControl, SecretReleaseObservation,
    SecretReleaseProbe,
};

#[path = "pty/teardown.rs"]
mod teardown;
pub(crate) use teardown::{
    ExitAuthority, PtyTeardownDebtOwner, PtyTeardownOutcome, ReaperReservation, TeardownClass,
    TeardownSupervisor, TeardownToken, WorkerSettlement, join_worker_bounded, report_teardown,
    teardown_supervisor,
};

#[path = "pty/exit.rs"]
mod exit;
use exit::process_belongs_to_pty;
#[path = "pty/session_teardown.rs"]
mod session_teardown;
#[path = "pty/spawn_shell.rs"]
mod spawn_shell;
#[path = "pty/spawn_ssh.rs"]
mod spawn_ssh;
#[cfg(test)]
pub(crate) use exit::WorkerWitnesses;
#[cfg(all(test, target_os = "linux"))]
pub(crate) use exit::process_has_open_path;
pub(crate) use exit::{ChildExit, LocalCwdResolution, resolve_local_cwd};
pub(crate) use exit::{describe_child_exit, open_pidfd, process_start_ticks, read_process_stat};

pub(crate) struct PtyUpdate {
    pub(crate) snapshot: Option<Vec<TranscriptRow>>,
    pub(crate) effects: emulator::TerminalEffects,
    pub(crate) alt_screen: bool,
    pub(crate) cursor_hidden: bool,
    pub(crate) application_cursor_keys: bool,
    pub(crate) bracketed_paste: bool,
    pub(crate) mouse_protocol: crate::mouse::MouseProtocol,
}

impl PtySession {
    /// The shell has shown its first prompt, so it has read its startup
    /// files and they can go.
    #[cfg(unix)]
    pub(crate) fn shell_started(&mut self) {
        self.shell_startup = None;
    }

    /// Whether the reader writes this pane's view.
    pub(crate) fn is_live(&self) -> bool {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Lets the reader write this pane's visible rows straight into `target`'s
    /// screen (`Some`), below the newest `rows_above` scrollback rows, or
    /// stops it (`None`). Only a change takes the screen lock; the view calls
    /// this every frame.
    pub(crate) fn set_live(
        &self,
        target: Option<(&enzui_terminal::TerminalScreen, bool, &str)>,
        rows_above: u16,
    ) {
        // The colour scheme is part of the key: a scheme change rewrites the
        // screen even while the pane prints nothing.
        let scheme = crate::color_scheme::active().id;
        let wanted = target.map(|(screen, focused, style)| {
            (screen.id(), focused, style.to_owned(), scheme, rows_above)
        });
        {
            let mut current = self
                .live
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *current == wanted {
                return;
            }
            *current = wanted;
        }
        let write = reader::lock_screen_for_ui(&self.screen)
            .ok()
            .and_then(|mut screen| {
                screen.set_live(target.map(|(view, focused, style)| {
                    emulator::LiveTarget::new(view.clone(), focused, style).rows_above(rows_above)
                }))
            });
        if let Some(write) = write {
            write.apply();
        }
    }

    /// Update the ring-buffer bound that the backend evicts against
    ///. Called from the settings-dialog writer path in `main.rs`
    /// whenever the user changes `Settings::scrollback_lines`, so a live PTY
    /// starts obeying the new cap right away rather than staying pinned to
    /// the value it was born with. `n` is expected pre-clamped (the caller
    /// uses `Settings::clamped_scrollback`).
    pub(crate) fn set_max_scrollback(&self, n: usize) {
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            screen.set_max_scrollback(n);
        }
    }

    #[cfg(test)]
    pub(crate) fn max_scrollback(&self) -> usize {
        reader::lock_screen_for_ui(&self.screen).map_or(0, |s| s.max_scrollback())
    }

    /// Drain the bytes this session has been asked to write to its PTY
    /// since the last drain, in order. A gate reads this after driving a key
    /// through `App::update` to assert the exact sequence that reached the
    /// writer; draining keeps one key's bytes from bleeding into the next
    /// assertion.
    #[cfg(test)]
    pub(crate) fn take_recorded_input(&self) -> Vec<u8> {
        self.recorded_input
            .lock()
            .map(|mut r| std::mem::take(&mut *r))
            .unwrap_or_default()
    }

    /// Take the exit published by the child waiter exactly once.
    ///
    /// The waiter rings the wake bus only after this slot is populated. This
    /// ordering matters: an output wake can reach the app just before the child
    /// becomes waitable, and the app has no polling timer that would otherwise
    /// retry a premature `try_wait`.
    pub(crate) fn take_child_exit(&self) -> Option<ChildExit> {
        self.child_exit.lock().ok()?.take()
    }

    /// Whether this child received its secret through the sealed `memfd`
    /// channel, which makes `sshpass` the leader whose status
    /// [`ChildExit::code`] carries. A session with no secret runs `ssh`
    /// directly, so its status is the remote command's.
    pub(crate) fn secret_delivery(&self) -> bool {
        self.secret_delivery
    }

    /// Hand terminal input to this session's writer without retaining it as
    /// command history.
    ///
    /// Returns as soon as the chunk is admitted to the bounded queue; the PTY
    /// write itself happens on the session's writer thread, so no reducer turn
    /// can block on a child that has stopped reading.
    ///
    /// # Errors
    ///
    /// [`PtyError::InputRefused`] when the chunk is oversized, the queue is
    /// saturated, or the writer is gone. Nothing is written and nothing is
    /// silently dropped.
    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.write_as(bytes, InputAdmission::Whole)
    }

    /// [`Self::write`] for keys, IME commits and pointer reports: the bytes may
    /// join the newest unstarted typed chunk instead of taking a chunk of
    /// their own (see [`InputAdmission::Typed`]).
    ///
    /// # Errors
    ///
    /// As [`Self::write`].
    pub(crate) fn write_typed(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.write_as(bytes, InputAdmission::Typed)
    }

    fn write_as(&self, bytes: &[u8], admission: InputAdmission) -> Result<(), PtyError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let input = self
            .input
            .as_ref()
            .ok_or(PtyError::InputRefused(InputRefusal::Closed))?;
        #[cfg(target_os = "linux")]
        let shell_owns_input = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
            .and_then(|pid| u32::try_from(pid).ok())
            == Some(self.process_identity.pid);
        #[cfg(not(target_os = "linux"))]
        let shell_owns_input = false;
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            // The reader cannot process accepted input before capture records its boundary.
            input
                .enqueue(bytes, admission)
                .map_err(PtyError::InputRefused)?;
            screen.pinned_capture.input(bytes, shell_owns_input);
        } else {
            input
                .enqueue(bytes, admission)
                .map_err(PtyError::InputRefused)?;
        }

        // Record exactly what was handed to the PTY writer, so an
        // update-driven gate can prove a cursor key's bytes travelled the
        // full `App::update` -> `write_active_pty` -> `PtySession::write`
        // path (not just the routing helper) and arrived in the right form.
        #[cfg(test)]
        if let Ok(mut rec) = self.recorded_input.lock() {
            rec.extend_from_slice(bytes);
        }

        Ok(())
    }

    /// The generation stamped on every chunk this session admits.
    ///
    /// A replacement session for the same pane owns a different queue and a
    /// different generation. `0` is not a generation any session can hold, so
    /// it reports a session whose input owner is already gone.
    #[cfg(test)]
    pub(crate) fn input_generation(&self) -> u64 {
        self.input
            .as_ref()
            .map_or(0, |input| input.generation.get())
    }

    /// Queued `(chunks, bytes)` not yet written.
    #[cfg(test)]
    pub(crate) fn queued_input(&self) -> (usize, usize) {
        self.input
            .as_ref()
            .map(|input| input.charge.queued())
            .unwrap_or_default()
    }

    /// Hold or release the writer thread before its next write.
    #[cfg(test)]
    pub(crate) fn hold_input_writer(&self, hold: bool) {
        if let Some(input) = self.input.as_ref() {
            input.gate.set(hold);
        }
    }

    /// Bytes the writer thread has actually written and flushed, in order.
    #[cfg(test)]
    pub(crate) fn take_written_input(&self) -> Vec<u8> {
        self.input
            .as_ref()
            .and_then(|input| {
                input
                    .drained
                    .lock()
                    .ok()
                    .map(|mut log| std::mem::take(&mut *log))
            })
            .unwrap_or_default()
    }

    pub(crate) fn set_pinned_capture(&self, enabled: bool) {
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            screen.pinned_capture.enable(enabled);
        }
    }

    pub(crate) fn exclude_pinned_input(&self) {
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            screen.pinned_capture.exclude_input();
        }
    }

    pub(crate) fn clear_pinned_output(&self) {
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            screen.pinned_capture.clear_retained();
        }
    }

    pub(crate) fn pinned_output(&self) -> Arc<crate::pinned_output::PinnedOutput> {
        reader::lock_screen_for_ui(&self.screen)
            .map(|screen| screen.pinned_capture.snapshot())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn pinned_capture_paused(&self) -> bool {
        reader::lock_screen_for_ui(&self.screen)
            .map_or(true, |screen| screen.pinned_capture.paused())
    }

    pub(crate) fn coordinates_invalidated(&self) -> bool {
        reader::lock_screen_for_ui(&self.screen)
            .is_ok_and(|screen| screen.coordinates_invalidated())
    }

    /// This session's live link authority, or `0` when the screen cannot be
    /// read. A pane with no live session therefore opens no retained link.
    pub(crate) fn link_generation(&self) -> u64 {
        reader::lock_screen_for_ui(&self.screen).map_or(0, |screen| screen.link_generation())
    }

    fn answer_terminal_queries(&self, screen: &mut emulator::Screen) {
        let replies = screen.take_terminal_replies();
        let Some(input) = self.input.as_ref() else {
            return;
        };
        // Terminal-generated replies are not user input and must not taint
        // accepted-command capture, so they bypass `write` while still sharing
        // the one writer that keeps every chunk whole and ordered.
        for reply in replies {
            if input.enqueue(&reply, InputAdmission::Whole).is_err() {
                break;
            }
        }
    }

    // Coordinates, cells, and mode bits cross the reader/UI boundary under one screen lock.
    pub(crate) fn take_update(&self) -> Option<PtyUpdate> {
        let mut screen = reader::lock_screen_for_ui(&self.screen).ok()?;
        self.answer_terminal_queries(&mut screen);
        let current = self.dirty.load(Ordering::SeqCst);
        let effects = screen.take_effects();
        let changed =
            current != self.last_seen.load(Ordering::SeqCst) || effects.coordinates_invalidated;
        let snapshot = changed.then(|| screen.project(&self.session_id));
        self.last_seen.store(current, Ordering::SeqCst);
        Some(PtyUpdate {
            snapshot,
            effects,
            alt_screen: screen.is_alt_screen(),
            cursor_hidden: !screen.cursor_shown(),
            application_cursor_keys: screen.application_cursor_keys(),
            bracketed_paste: screen.bracketed_paste(),
            mouse_protocol: screen.mouse_protocol(),
        })
    }

    // Query-time coordinates and effects cross the reader/UI boundary
    // under the same screen lock.
    #[cfg(test)]
    pub(crate) fn take_terminal_effects(&self) -> emulator::TerminalEffects {
        reader::lock_screen_for_ui(&self.screen)
            .map(|mut screen| {
                self.answer_terminal_queries(&mut screen);
                screen.take_effects()
            })
            .unwrap_or_default()
    }

    /// Reproject the screen only if it changed since the last poll.
    #[cfg(test)]
    pub(crate) fn take_snapshot(&self) -> Option<Vec<TranscriptRow>> {
        let current = self.dirty.load(Ordering::SeqCst);
        let last = self.last_seen.load(Ordering::SeqCst);
        if current == last {
            return None;
        }
        self.last_seen.store(current, Ordering::SeqCst);
        Some(self.project_screen())
    }

    /// Unconditional projection (headless / test callers).
    #[cfg(test)] // Test-only synchronous projection; the app path is `take_snapshot`.
    pub(crate) fn snapshot_now(&self) -> Option<Vec<TranscriptRow>> {
        Some(self.project_screen())
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn stop_output_reader_for_test(&mut self) {
        self.reader_cancel.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.reader_done.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "reader cancellation did not settle"
            );
            thread::sleep(Duration::from_millis(1));
        }
        if let Some(reader) = self.reader_thread.take() {
            reader.join().expect("output reader panicked");
            self.reader_joined.store(true, Ordering::SeqCst);
        }
    }

    #[cfg(test)]
    pub(crate) fn feed_screen_for_test(&self, bytes: &[u8]) {
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            screen.process(bytes);
        }
    }

    /// Whether this pane's shell has DECCKM (application cursor keys) engaged
    /// — `false` on a poisoned mutex, the normal-mode default.
    #[cfg(test)]
    pub(crate) fn application_cursor_keys(&self) -> bool {
        reader::lock_screen_for_ui(&self.screen)
            .is_ok_and(|screen| screen.application_cursor_keys())
    }

    /// Whether this pane's shell has DECSET 2004 (bracketed paste) engaged —
    /// `false` on a poisoned mutex, the conservative default (an unwrapped
    /// paste is never worse than a wrapped one to a shell that did not ask for
    /// it, since the guard bytes would print literally).
    #[cfg(test)]
    pub(crate) fn bracketed_paste(&self) -> bool {
        reader::lock_screen_for_ui(&self.screen).is_ok_and(|screen| screen.bracketed_paste())
    }

    /// This pane's live mouse-tracking `(mode, encoding)` pair — the
    /// default (no tracking) on a poisoned mutex, so a stuck pane never routes
    /// a pointer gesture into a shell that cannot read it.
    #[cfg(test)]
    pub(crate) fn mouse_protocol(&self) -> crate::mouse::MouseProtocol {
        reader::lock_screen_for_ui(&self.screen)
            .map(|screen| screen.mouse_protocol())
            .unwrap_or_default()
    }

    /// Whether this pane's shell has the ALTERNATE SCREEN engaged.
    ///
    /// Forwards [`emulator::Screen::is_alt_screen`] across the mutex so
    /// the reducer can record it on the pane each drain. A poisoned mutex
    /// answers `false` (primary), which is the conservative side: scrollback
    /// keys/wheel keep working rather than being silently routed into a shell.
    #[cfg(test)]
    pub(crate) fn alt_screen(&self) -> bool {
        reader::lock_screen_for_ui(&self.screen).is_ok_and(|screen| screen.is_alt_screen())
    }

    #[cfg(test)]
    fn project_screen(&self) -> Vec<TranscriptRow> {
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            screen.project(&self.session_id)
        } else {
            Vec::new()
        }
    }

    // `PtySession::cell_probe` DELETED. It forwarded to
    // `emulator::TerminalScreen::cell_probe`, and every caller - all of
    // them tests - already holds the screen and calls that directly
    // (`screen.lock().unwrap().cell_probe(..)`). A forwarder with no caller is
    // not an API, it is a second name for one.

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn assert_grid_geometry(&self, rows: u16, cols: u16) {
        let size = self
            .master
            .as_ref()
            .expect("live PTY")
            .get_size()
            .expect("kernel PTY geometry");
        assert_eq!(
            (size.rows, size.cols),
            (rows, cols),
            "kernel PTY geometry changed"
        );
        let (active, inactive) = self
            .screen
            .lock()
            .expect("screen geometry")
            .buffer_dimensions();
        let expected = (usize::from(cols), usize::from(rows));
        assert_eq!(active, expected, "active buffer geometry changed");
        if let Some(inactive) = inactive {
            assert_eq!(inactive, expected, "inactive buffer geometry changed");
        }
    }

    /// Resize the PTY and the termwiz surface to a new grid geometry.
    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<(), PtyError> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master
            .as_ref()
            .ok_or(PtyError::SessionClosed)?
            .resize(size)
            .map_err(|err| PtyError::Resize(err.to_string()))?;
        if let Ok(mut screen) = reader::lock_screen_for_ui(&self.screen) {
            screen.resize(rows as usize, cols as usize);
        }
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Nothing is owed, or this shell already handed everything on.
        if self
            .teardown_outcome
            .is_some_and(PtyTeardownOutcome::is_complete)
            || matches!(self.custody, CustodyState::Moved | CustodyState::Lost)
        {
            return;
        }
        if self.custody == CustodyState::InOwner {
            // The owner around this session is being unwound. Minting a second
            // owner here would give one reservation two holders and could
            // recurse, so the loss is recorded exactly once and the slot stays
            // charged so no new child takes the place of one nobody can
            // account for.
            self.custody = CustodyState::Lost;
            self._reaper_slot.strand();
            teardown_supervisor().record_lost(self.teardown_token);
            return;
        }
        // No signal, wait or join runs here: this may be the reducer thread.
        // Releasing the fields instead would close a master, slave and
        // cancellation pipe a live child still holds, abandon two join handles,
        // and hand the capacity slot back while the child is still running, so
        // exact custody moves to the process executor instead.
        let outcome = self
            .teardown_outcome
            .unwrap_or_else(PtyTeardownOutcome::pending);
        teardown_supervisor()
            .adopt_final(PtyTeardownDebtOwner::new(self.transfer_custody(), outcome));
    }
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("session_id", &self.session_id)
            .field("dirty", &self.dirty.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
#[path = "pty/errors.rs"]
mod errors;
pub(crate) use errors::PtyError;

#[path = "pty/wake.rs"]
mod wake;
pub(crate) use wake::{WakeBus, set_relaxed, wake_bus, wake_stream};

/// The owned password channel handed back with the built command. On Linux it
/// is a retained anonymous sealed `memfd`; elsewhere no equivalently
/// non-observable anonymous channel exists, so sshpass password delivery is
/// unavailable and this is always `None`.
///
/// It is handed to the spawn path by value, because once a child exists ONLY
/// an owner that can kill and reap that child may release the descriptor
/// number (see `PasswordSecret`'s descriptor-number lifetime note).
#[path = "pty/reader.rs"]
mod reader;
pub(crate) use reader::ReaderBody;
#[cfg(test)]
pub(crate) use reader::real_hup_ignoring_pair_and_cmd;
#[cfg(test)]
pub(crate) use reader::run_reader_loop;
pub(crate) use reader::{default_reader_spawner, shell_single_quote, spawn_reader_thread_with};

#[cfg(target_os = "linux")]
pub(crate) type PasswordChannel = Option<PasswordSecret>;
#[cfg(not(target_os = "linux"))]
pub(crate) type PasswordChannel = Option<()>;

#[path = "pty/ssh.rs"]
mod ssh;
pub(crate) use ssh::{
    PasswordSecret, build_ssh_command_with, kill_reap_then_release, resolve_executable,
    resolve_executable_in,
};

#[cfg(test)]
mod tests;

pub(crate) fn pin_ssh_hostname(cmd: &mut CommandBuilder, hostname: &str) -> Result<(), PtyError> {
    let argv = cmd.get_argv_mut();
    let at = argv
        .len()
        .checked_sub(3)
        .filter(|at| argv[*at] == "--")
        .ok_or_else(|| PtyError::Other("SSH command has no destination boundary".into()))?;
    // Keep the alias as the lookup key for ProxyJump and other config options,
    // but pin HostName so a config edit cannot change the selected destination.
    argv.splice(at..at, ["-o".into(), format!("HostName={hostname}").into()]);
    Ok(())
}

#[cfg(test)]
pub(crate) use errors::PtyFailureKind;
#[cfg(test)]
use identity::{LeaderIdentityState, rescan_until_stable};
#[cfg(test)]
use ssh::build_remote_command;
#[cfg(test)]
pub(crate) use teardown::ProcessTeardownDebt;
