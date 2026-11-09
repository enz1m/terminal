//! Edge-triggered PTY wake bus.

use super::*;

// The wake bus
// ---------------------------------------------------------------------------

/// The edge-triggered "a PTY produced bytes" bell.
///
/// # Why this exists
///
/// A fixed-interval poll such as `iced::time::every(33ms)` would repaint an
/// idle terminal ~30 times a second, because iced repaints a window whenever
/// `update` runs (measured: 100.4% of one core under the software renderer,
/// 15.25 committed frames/second on the Wayland wire, dominated by
/// `cosmic_text`/`fontdb` re-shaping identical text).
///
/// A readable fd is an *event*. The reader threads already block in `read()`
/// and already know the instant bytes arrive, so they ring this bell; the app's
/// [`wake_stream`] subscription awaits it and emits exactly one drain per
/// burst. Idle costs nothing and output is **not** capped at 30 Hz.
///
/// # Why a level-plus-waker pair rather than a channel
///
/// `notify` must never block and must never allocate: it runs on the reader
/// thread between a `read()` and the next, and a full-channel stall there would
/// throttle the shell itself. A `bool` flag plus an [`AtomicWaker`] cannot fill
/// up, and coalesces a burst of a thousand reads into one wake — which is what
/// we want, since `drain_ptys` reprojects the *current* screen, not a backlog.
///
/// The flag is checked BEFORE parking (`take`) so a wake that lands between two
/// polls is not lost: that race is the classic way an event-driven redraw
/// silently becomes a hang.
#[derive(Default)]
pub(crate) struct WakeBus {
    /// Set by `notify`, cleared by `take`. Level, so a wake cannot be missed.
    pending: AtomicBool,
    /// Woken by `notify` so a parked stream is polled again.
    waker: AtomicWaker,
    /// Monotonic count of [`WakeBus::notify`] calls, never cleared.
    ///
    /// `pending` is a LEVEL that `take` consumes, so it cannot answer "did
    /// anything ring during this window?" — the very question an idle-cost
    /// gate asks. This counter is the edge history: an idle terminal must
    /// leave it standing still, and any message the subscription produces
    /// while it stands still came from a timer, not from the PTY.
    rings: AtomicU64,
    /// Set by [`WakeBus::notify_reply`]: the pending drain carries a reply the
    /// program is waiting for, so it waits at most [`FRAME_INTERVAL`].
    reply: AtomicBool,
}

impl WakeBus {
    /// Ring the bell. Called from reader threads; never blocks, never allocates.
    ///
    /// Only the ring that raises the level wakes the stream. While a wake is
    /// already pending the stream is either about to drain or holding for the
    /// frame cap with its trailing wake scheduled, so waking it again on every
    /// read would only re-poll it to answer "still in cooldown" (a flood of
    /// output did that some 20 000 times a second).
    pub(crate) fn notify(&self) {
        self.rings.fetch_add(1, Ordering::SeqCst);
        if !self.pending.swap(true, Ordering::SeqCst) {
            self.waker.wake();
        }
    }

    /// Ring the bell for output that queued a terminal reply (a status,
    /// position or attributes report). The reply is sent by the drain, and
    /// the program that asked is blocked on it, so the drain waits at most
    /// [`FRAME_INTERVAL`] even while the relaxed cap would hold it longer.
    pub(crate) fn notify_reply(&self) {
        self.reply.store(true, Ordering::SeqCst);
        self.rings.fetch_add(1, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        // Always wake: a stream holding for the relaxed cap must re-poll to
        // shorten its wait.
        self.waker.wake();
    }

    #[cfg(test)]
    pub(crate) fn reply_pending(&self) -> bool {
        self.reply.load(Ordering::SeqCst)
    }

    /// How many times this bus has been rung since the process started.
    ///
    /// Monotonic and never reset, so two reads bracket a window: equal reads
    /// mean the PTY produced nothing at all in between.
    // Read by the `idle_cost` gate rather than by the binary: the
    // production path RINGS the bell, and this counter exists so a gate can
    // prove that an idle terminal did not.
    #[allow(
        dead_code,
        reason = "Wake diagnostics read the counter; normal delivery consumes the pending flags."
    )]
    pub(crate) fn rings(&self) -> u64 {
        self.rings.load(Ordering::SeqCst)
    }

    /// Consume a pending wake, if any. `true` means "drain now".
    pub(crate) fn take(&self) -> bool {
        self.pending.swap(false, Ordering::SeqCst)
    }

    /// Peek the pending flag WITHOUT consuming it.
    ///
    /// `wake_stream_with` needs this: during a frame-cap cooldown the
    /// stream must know a wake is pending — so it can schedule a trailing
    /// paint — WITHOUT clearing the flag. Clearing it mid-cooldown loses the
    /// signal: the next poll after the cooldown expires would see `pending`
    /// false, park with no new notify to unstick it, and the terminal freezes
    /// with the last burst's bytes unrendered.
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    /// Register `waker` to be woken by the next [`WakeBus::notify`].
    ///
    /// Split out of [`WakeBus::register_and_take`] for the cap path: during
    /// cooldown we register without consuming.
    pub(crate) fn register(&self, waker: &std::task::Waker) {
        self.waker.register(waker);
    }

    /// Register `waker` to be woken by the next [`WakeBus::notify`], then
    /// re-check the flag.
    ///
    /// The re-check is the whole point: `notify` can land between a failed
    /// `take` and the `register`, and without re-checking afterwards that wake
    /// is dropped and the terminal freezes until the next unrelated event.
    ///
    /// The stream splits this into `register` + `is_pending` + `take` so it can
    /// peek without consuming during a cooldown; this pair-op stays available
    /// as a public API for a caller that wants the simple, non-capped
    /// semantics.
    #[allow(
        dead_code,
        reason = "The uncapped wake API remains available; the rate-limited stream uses its split operations."
    )]
    pub(crate) fn register_and_take(&self, waker: &std::task::Waker) -> bool {
        self.waker.register(waker);
        self.take()
    }
}

/// The process-global wake bus every PTY reader thread rings.
pub(crate) fn wake_bus() -> &'static WakeBus {
    static BUS: OnceLock<WakeBus> = OnceLock::new();
    BUS.get_or_init(WakeBus::default)
}

/// Frame-rate cap for [`wake_stream`].
///
/// Once the stream has yielded one item, it will not yield another for this
/// long even if the reader thread notifies again. During the cooldown the
/// reader keeps reading and accumulating into the [`emulator::Screen`];
/// when the cooldown expires, `drain_ptys` projects the *current* screen and
/// every intermediate update is coalesced for free — which is exactly what
/// alacritty does.
///
/// 16 ms = ~60 Hz, the frame rate of every non-exotic display on this system.
/// Fixed rather than adaptive on purpose: adaptive would need to observe the
/// compositor's refresh rate, and no widget-tree-level API exposes it. 16 ms is
/// simple, correct on every mainstream display, and an uncapped burst runs at
/// ~300 fps, so anywhere in the 60–120 Hz range closes essentially the entire
/// gap.
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// A stream that yields one item per burst of PTY output, and **nothing at all**
/// while every PTY is quiet.
///
/// This is the replacement for `iced::time::every(33ms)`. It is a free function
/// over `&'static WakeBus` rather than a closure over the app so a test can
/// drive this exact code with its own bus.
///
/// Frame-rate capped at [`FRAME_INTERVAL`]: between two yields of a
/// burst, at least that much time must pass. The **first** frame after an idle
/// period is NEVER capped — a single keystroke echo must paint immediately.
/// The last frame of a burst is ALSO NEVER dropped: when the cooldown expires
/// with the pending flag still set (a mid-cooldown notify that arrived after
/// the cap engaged), a scheduled wake fires and the stream yields the trailing
/// frame. Without that scheduled wake the last line of `ls` output would sit
/// invisible until some unrelated event repainted — the worst possible bug
/// here.
pub(crate) fn wake_stream(bus: &'static WakeBus) -> impl iced::futures::Stream<Item = ()> {
    wake_stream_with_interval(bus, current_interval, Instant::now, default_delayed_wake)
}

/// Whether every running pane's view is written by its PTY reader (see
/// `PtySession::set_live`). The app's drain then only keeps the transcript,
/// titles, bells and the working directory current, which does not need a
/// display-rate cap.
static RELAXED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The drain cap while every pane is written by its reader.
pub(super) const RELAXED_INTERVAL: Duration = Duration::from_millis(100);

/// Records whether every running pane's view is written by its reader.
pub(crate) fn set_relaxed(relaxed: bool) {
    RELAXED.store(relaxed, std::sync::atomic::Ordering::Relaxed);
}

fn current_interval() -> Duration {
    if RELAXED.load(std::sync::atomic::Ordering::Relaxed) {
        RELAXED_INTERVAL
    } else {
        FRAME_INTERVAL
    }
}

/// Production delayed-wake: hand the deadline to the one cooldown timer thread.
///
/// This used to spawn a fresh OS thread per cooldown, which during sustained
/// output is one thread creation per painted frame (60 a second). The timer
/// thread is started on the first cooldown, parks on a condition variable while
/// nothing is scheduled (an idle terminal pays nothing), and holds at most the
/// one pending deadline: the single wake stream replaces it rather than queueing.
fn default_delayed_wake(delay: Duration, waker: std::task::Waker) {
    match cooldown_timer() {
        Some(timer) => timer.schedule(Instant::now() + delay, waker),
        // No timer thread could be started: wait the cooldown out here rather
        // than wake early, which would leave the trailing frame unpainted.
        None => {
            thread::sleep(delay);
            waker.wake();
        }
    }
}

#[derive(Default)]
struct CooldownTimer {
    pending: Mutex<Option<(Instant, std::task::Waker)>>,
    changed: Condvar,
}

impl CooldownTimer {
    fn schedule(&self, deadline: Instant, waker: std::task::Waker) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some((deadline, waker));
            self.changed.notify_one();
        }
    }

    fn run(&self) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        loop {
            let due = pending.as_ref().map(|(deadline, _)| *deadline);
            pending = match due {
                None => match self.changed.wait(pending) {
                    Ok(guard) => guard,
                    Err(_) => return,
                },
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        if let Some((_, waker)) = pending.take() {
                            drop(pending);
                            waker.wake();
                        }
                        match self.pending.lock() {
                            Ok(guard) => guard,
                            Err(_) => return,
                        }
                    } else {
                        match self.changed.wait_timeout(pending, deadline - now) {
                            Ok((guard, _)) => guard,
                            Err(_) => return,
                        }
                    }
                }
            };
        }
    }
}

fn cooldown_timer() -> Option<&'static CooldownTimer> {
    static TIMER: OnceLock<Option<&'static CooldownTimer>> = OnceLock::new();
    *TIMER.get_or_init(|| {
        let timer: &'static CooldownTimer = Box::leak(Box::default());
        thread::Builder::new()
            .name("terminal-emulator-frame-cooldown".into())
            .spawn(move || timer.run())
            .ok()
            .map(|_| timer)
    })
}

/// [`wake_stream_with`] with the cap read on each poll.
fn wake_stream_with_interval<I, C, S>(
    bus: &'static WakeBus,
    interval: I,
    clock: C,
    scheduler: S,
) -> impl iced::futures::Stream<Item = ()>
where
    I: Fn() -> Duration + 'static,
    C: FnMut() -> Instant + 'static,
    S: FnMut(Duration, std::task::Waker) + 'static,
{
    let mut clock = clock;
    let mut scheduler = scheduler;
    // The instant of the last yielded frame. `None` means "has never yielded"
    // — the FIRST frame after startup and the first frame after an idle window
    // long enough to reset `last_yield` are both indistinguishable at this
    // layer and both intentionally uncapped.
    let mut last_yield: Option<Instant> = None;
    // The deadline (`last_yield + interval`) for which we have already handed a
    // delayed-wake to `scheduler`. Guards against spawning a fresh cooldown
    // thread on every poll during a burst — an executor may poll many times
    // between two `notify`s.
    let mut scheduled_deadline: Option<Instant> = None;

    iced::futures::stream::poll_fn(move |cx| {
        // Fast path: no wake pending. Park — with the belt-and-braces re-check
        // that `register_and_take` documents, so a notify racing the register
        // is not lost.
        if !bus.is_pending() {
            bus.register(cx.waker());
            if !bus.is_pending() {
                return std::task::Poll::Pending;
            }
        }

        // A wake IS pending. Decide whether to yield now or hold for the cap.
        let now = clock();
        let cap = if bus.reply.load(Ordering::SeqCst) {
            interval().min(FRAME_INTERVAL)
        } else {
            interval()
        };
        let elapsed_ok = match last_yield {
            None => true,
            Some(prev) => now.saturating_duration_since(prev) >= cap,
        };

        if elapsed_ok {
            // Consume the pending flag and yield. Cooldown starts now.
            bus.reply.store(false, Ordering::SeqCst);
            let _ = bus.take();
            last_yield = Some(now);
            scheduled_deadline = None;
            std::task::Poll::Ready(Some(()))
        } else {
            // In-cooldown. Register the waker so a later notify re-polls us,
            // and schedule a wake at the cooldown deadline so the TRAILING
            // frame paints even if the reader has already gone quiet.
            //
            // Critically: do NOT `take()` here. The pending flag must survive
            // the cooldown — when the deadline fires and we re-poll, we look at
            // this same flag to decide there is work to do.
            bus.register(cx.waker());
            let prev = last_yield.expect("elapsed_ok=false implies Some(prev)");
            let deadline = prev + cap;
            if scheduled_deadline != Some(deadline) {
                let delay = deadline.saturating_duration_since(now);
                scheduler(delay, cx.waker().clone());
                scheduled_deadline = Some(deadline);
            }
            std::task::Poll::Pending
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingWaker(AtomicU64);

    impl std::task::Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}
