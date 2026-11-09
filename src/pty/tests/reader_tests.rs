//! Reader failures.

use super::*;

/// A reader-spawn failure must not leave an ORPHAN shell behind.
///
/// The child is spawned BEFORE the reader thread, so the failure path owns
/// it: nothing will ever read its output and no `PtySession` exists to close
/// it, so without an explicit kill every tab that failed to open leaks a
/// live process holding its pty slave open. On a host thrashing badly enough
/// to return EAGAIN, leaking a process per retry is the worst possible
/// response.
///
/// Observed from OUTSIDE the code under test — /proc, not a handle the module
/// hands back — because the whole point is to see a process the module has
/// stopped tracking. `finish_spawn_with` consumes the pair, so the pid is
/// only reachable this way.
///
/// Sabotage: drop the `child.kill()`/`child.wait()` pair from the `Err` arm
/// in `finish_spawn_with`.
#[test]
#[cfg(target_os = "linux")]
fn a_failing_reader_thread_spawn_leaves_no_orphan_child() {
    const MARKER: &str = "enzui-1005-orphan-probe";
    let Some((pair, cmd)) = real_pair_and_cmd(MARKER) else {
        eprintln!("openpty unavailable in this environment; skipping");
        return;
    };

    // CALIBRATION: the probe must be able to see this child at all. Without
    // this, a marker that never matches makes the assertion below a
    // guaranteed pass whatever the code does.
    let (pair2, cmd2) = real_pair_and_cmd(MARKER).expect("second pair");
    let live =
        PtySession::finish_spawn_with("calibrate-orphan", pair2, cmd2, default_reader_spawner)
            .expect("a successful spawn for the calibration");
    let mut seen = 0;
    for _ in 0..100 {
        seen = live_marked_children(MARKER);
        if seen > 0 {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        seen > 0,
        "harness precondition: the /proc probe cannot see a child it KNOWS \
         is running, so its zero below would prove nothing"
    );
    drop(live);

    // Successful sessions own their child too; dropping the calibration must
    // leave a clean baseline before the injected failure is measured. The
    // drop hands the child to the teardown executor, which reaps it within
    // the TERM grace rather than synchronously.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut before = live_marked_children(MARKER);
    while before != 0 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
        before = live_marked_children(MARKER);
    }
    assert_eq!(before, 0, "dropping the calibration session must reap it");

    let err =
        PtySession::finish_spawn_with("eagain-orphan", pair, cmd, eagain_spawner_retaining_body)
            .expect_err("the injected spawner must make finish_spawn fail");
    assert!(matches!(err, PtyError::SpawnReader(_)), "got {err:?}");

    // Sample over a WINDOW and keep the MAXIMUM, rather than polling until
    // the count drops back. `/proc/<pid>/cmdline` only carries the marker
    // once the child has `exec`ed, so a leaked child is invisible for a few
    // milliseconds after the fork — and an "is it back to baseline yet" loop
    // exits on that first sample and passes with the cleanup deleted
    // (measured: it did). The maximum over the window cannot be fooled that
    // way: `kill` + `wait` in the error path are synchronous, so a kept child
    // is already reaped before `finish_spawn_with` returns and can never
    // appear, while a LEAKED child ignores the hangup and sits in `sleep`
    // for a minute.
    let mut peak = 0usize;
    let settle = std::time::Instant::now();
    while settle.elapsed() < Duration::from_millis(1500) {
        peak = peak.max(live_marked_children(MARKER));
        thread::sleep(Duration::from_millis(20));
    }
    let extra = peak.saturating_sub(before);
    // Clean up whatever is still ours before asserting, so a red test does
    // not leave stray processes for the next run to count.
    kill_marked_children(MARKER);
    assert_eq!(
        extra, 0,
        "a failed reader spawn left {extra} extra live child process(es) \
         (baseline={before}, peak={peak}); every tab that failed to open \
         would leak a shell holding its pty slave"
    );
}

/// Pids of this process's live (non-zombie) children whose argv contains
/// `marker`.
///
/// Reads /proc rather than any handle the module hands back: the point of
/// the orphan guard is to observe a process the module has stopped tracking.
/// Matching on the marker rather than on `comm` keeps a parallel test's own
/// shells out of the count.
#[cfg(target_os = "linux")]
fn marked_child_pids(marker: &str) -> Vec<u32> {
    let me = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.filter_map(std::result::Result::ok) {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(cmdline) = std::fs::read(format!("/proc/{name}/cmdline")) else {
            continue;
        };
        if !String::from_utf8_lossy(&cmdline).contains(marker) {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{name}/stat")) else {
            continue;
        };
        // `comm` is parenthesised and may itself contain spaces or parens,
        // so the fields after it are found from the LAST ')'.
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        let mut rest = stat[close + 1..].split_whitespace();
        let state = rest.next().unwrap_or("");
        let ppid = rest.next().unwrap_or("");
        // A zombie is already reaped-pending, not a leak.
        if state != "Z" && ppid == me {
            out.extend(name.parse::<u32>().ok());
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn live_marked_children(marker: &str) -> usize {
    marked_child_pids(marker).len()
}
