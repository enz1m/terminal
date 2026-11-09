//! PTY reader threads and output loops.

use super::*;

/// UI threads waiting for a `Screen` lock. A reader under continuous output
/// re-locks before a woken waiter is scheduled, so it yields to them.
static UI_SCREEN_WAITERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

// Bounds one yield, so a stuck waiter count cannot stall output.
const UI_YIELD_LIMIT: Duration = Duration::from_millis(4);

/// Lock a screen from the UI thread, announcing the wait to output readers.
pub(crate) fn lock_screen_for_ui(
    screen: &Mutex<emulator::Screen>,
) -> std::sync::LockResult<std::sync::MutexGuard<'_, emulator::Screen>> {
    UI_SCREEN_WAITERS.fetch_add(1, Ordering::SeqCst);
    let guard = screen.lock();
    UI_SCREEN_WAITERS.fetch_sub(1, Ordering::SeqCst);
    guard
}

fn yield_to_ui() {
    let deadline = Instant::now() + UI_YIELD_LIMIT;
    while UI_SCREEN_WAITERS.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        thread::yield_now();
    }
}

#[cfg(test)]
pub(crate) fn real_hup_ignoring_pair_and_cmd(
    marker: &str,
    with_descendant: bool,
) -> Option<(PtyPair, CommandBuilder)> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .ok()?;
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.arg("-c");
    if with_descendant {
        cmd = CommandBuilder::new("/bin/bash");
        cmd.arg("--noprofile");
        cmd.arg("--norc");
        cmd.arg("-i");
        cmd.arg("-c");
        // The escaped descendant scrubs its environment with `env -i` before
        // `setsid -f` double-forks it: no inherited marker string survives, its
        // session and parent links are gone, and only the inherited capability
        // descriptor still ties it to this PTY.
        cmd.arg(
            "trap '' HUP TERM; \
             set +m; /usr/bin/env -i /usr/bin/setsid -f /bin/bash -c 'trap \"\" HUP TERM; exec -a \"$1-scrubbed\" /bin/sleep 20' shutdown-probe \"$1\" & \
             set -m; (trap '' HUP TERM; exec -a \"$1-background\" /bin/sleep 20) & \
             printf 'enzui-job-control-ready\\n'; \
             exec -a \"$1-leader\" /bin/sleep 20",
        );
    } else {
        cmd.arg(
            "trap '' HUP; n=0; while [ $n -lt 100 ]; do n=$((n+1)); \
             command sleep 0.2; done; echo \"$1\"",
        );
    }
    cmd.arg("shutdown-probe");
    cmd.arg(marker);
    cmd.env("TERM", "xterm-256color");
    Some((pair, cmd))
}

// ---------------------------------------------------------------------------
// Reader thread
// ---------------------------------------------------------------------------

/// The reader thread's body, boxed so a spawner can be taken as a plain `FnOnce`
/// rather than a generic-over-closure-type parameter.
pub(crate) type ReaderBody = Box<dyn FnOnce() + Send + 'static>;

/// The production spawner: a named OS thread via `thread::Builder`.
///
/// `thread::Builder::spawn` returns `io::Result` precisely because the spawn can
/// fail transiently (EAGAIN). That result is propagated, not unwrapped — see
/// [`PtyError::SpawnReader`].
pub(crate) fn default_reader_spawner(body: ReaderBody) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("terminal-emulator-pty-reader".into())
        .spawn(body)
}

/// Build the reader-thread body and hand it to `spawner`.
///
/// Split from the common spawn path so the body a test's failing spawner is
/// handed is the SAME body production runs.
pub(crate) fn spawn_reader_thread_with<S>(
    spawner: S,
    reader: Box<dyn Read + Send>,
    #[cfg(target_os = "linux")] control: LinuxReaderControl,
    screen: Arc<Mutex<emulator::Screen>>,
    dirty: Arc<AtomicU64>,
    #[cfg(test)] done: Arc<AtomicBool>,
    wake: &'static WakeBus,
) -> std::io::Result<thread::JoinHandle<()>>
where
    S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
{
    spawner(Box::new(move || {
        let mut reader = reader;
        #[cfg(target_os = "linux")]
        run_polling_reader_loop(&mut *reader, &control, &screen, &dirty, wake);
        #[cfg(not(target_os = "linux"))]
        run_reader_loop(&mut *reader, &screen, &dirty, wake, thread::sleep);
        #[cfg(test)]
        done.store(true, Ordering::Release);
    }))
}

/// Linux production reader: wait on a `dup` of the unchanged blocking PTY
/// master and a private cancellation pipe. Poll readiness prevents the subsequent blocking
/// read from stranding shutdown when a non-owned process keeps the slave open.
#[cfg(target_os = "linux")]
fn run_polling_reader_loop<R: Read + ?Sized>(
    reader: &mut R,
    control: &LinuxReaderControl,
    screen: &Mutex<emulator::Screen>,
    dirty: &AtomicU64,
    wake: &WakeBus,
) {
    let mut buf = [0u8; READ_CHUNK];
    let mut backoff = thread::sleep;
    let watch = |fd: RawFd| filedescriptor::pollfd {
        fd,
        events: filedescriptor::POLLIN,
        revents: 0,
    };
    // A live pane's last write of a burst is owed until output pauses; poll
    // with a short timeout then, so it lands without waiting for more bytes.
    let live_pending = screen.lock().map(|s| s.live_pending.clone()).ok();
    loop {
        let mut ready = [
            watch(control.master.as_raw_fd()),
            watch(control.cancel.as_raw_fd()),
        ];
        let timeout = live_pending
            .as_ref()
            .filter(|pending| pending.load(Ordering::Acquire))
            .map(|_| emulator::LIVE_PERIOD);
        match filedescriptor::poll(&mut ready, timeout) {
            Ok(_) => {}
            Err(filedescriptor::Error::Poll(err))
                if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                report_reader_error(dirty, wake);
                return;
            }
        }

        if ready[1].revents != 0 {
            return;
        }
        if ready[0].revents == 0 {
            // Timed out with a live write owed: flush it.
            let live = screen.lock().ok().and_then(|mut s| s.live_write(true));
            if let Some(live) = live {
                live.apply();
            }
            continue;
        }
        if ready[0].revents != 0
            && !read_and_process_once(reader, &mut buf, screen, dirty, wake, &mut backoff)
        {
            return;
        }
    }
}

/// The portable reader loop used by non-Linux production builds and by Linux
/// tests with a synthetic reader.
///
/// `backoff` is injected so a test counts sleeps instead of actually sleeping;
/// Linux production uses the cancellation-aware polling loop above.
///
/// Returns when the reader reports EOF or a real error.
#[cfg(any(test, not(target_os = "linux")))]
pub(crate) fn run_reader_loop<R, B>(
    reader: &mut R,
    screen: &Mutex<emulator::Screen>,
    dirty: &AtomicU64,
    wake: &WakeBus,
    mut backoff: B,
) where
    R: Read + ?Sized,
    B: FnMut(Duration),
{
    let mut buf = [0u8; READ_CHUNK];
    while read_and_process_once(reader, &mut buf, screen, dirty, wake, &mut backoff) {}
}

/// Perform one already-readable PTY read. `true` asks the caller to continue;
/// EOF and permanent errors publish their terminal update and return `false`.
fn read_and_process_once<R, B>(
    reader: &mut R,
    buf: &mut [u8; READ_CHUNK],
    screen: &Mutex<emulator::Screen>,
    dirty: &AtomicU64,
    wake: &WakeBus,
    backoff: &mut B,
) -> bool
where
    R: Read + ?Sized,
    B: FnMut(Duration),
{
    match reader.read(buf) {
        Ok(0) => {
            if let Ok(mut s) = screen.lock() {
                s.process(b"\r\n[process exited]\r\n");
            }
            dirty.fetch_add(1, Ordering::SeqCst);
            // The "[process exited]" banner is a screen change like
            // any other. Without this bell it would sit unrendered until
            // some unrelated event happened to repaint.
            wake.notify();
            false
        }
        Ok(n) => {
            let (live, replies) = match screen.lock() {
                Ok(mut s) => {
                    s.process(&buf[..n]);
                    (s.live_write(false), s.has_terminal_replies())
                }
                Err(_) => (None, false),
            };
            // A live pane's view is written outside the screen lock.
            if let Some(live) = live {
                live.apply();
            }
            yield_to_ui();
            // Ring AFTER the bytes are in the `Screen`, so the drain
            // this wakes projects a screen that already contains them.
            // Ringing first would race the app to an unchanged snapshot
            // and drop the output until the next byte arrived.
            if replies {
                wake.notify_reply();
            } else {
                wake.notify();
            }
            true
        }
        // A non-blocking master fd returns `WouldBlock` *instantly* and
        // forever while the child is quiet. Retrying with a bare `continue`
        // therefore pegs one core at 100% and starves the UI thread of the
        // scheduler. Park instead; on the blocking fd
        // this module expects, this arm is never reached at all.
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            backoff(WOULD_BLOCK_BACKOFF);
            true
        }
        // `Interrupted` is a real, self-limiting retry (a signal landed
        // mid-`read`), not a hot condition: retry immediately, with NO
        // backoff, so a signal-heavy host does not add latency to output.
        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => true,
        Err(_) => {
            report_reader_error(dirty, wake);
            false
        }
    }
}

fn report_reader_error(dirty: &AtomicU64, wake: &WakeBus) {
    dirty.fetch_add(1, Ordering::SeqCst);
    wake.notify();
}

/// Quote one value for assignment by the remote POSIX shell.
///
/// The prompt command itself contains single quotes around the `printf` format;
/// interpolating it into another pair of single quotes strips the backslashes
/// from `\\033` and turns the OSC markers into printable text. The standard
/// close-quote / escaped-quote / reopen sequence preserves the bytes.
pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
