//! Session teardown.

use super::*;

#[test]
#[cfg(target_os = "linux")]
fn reducer_submission_returns_quickly_while_a_resistant_owner_stays_charged() {
    // Capacity and token exhaustion are refused before any child exists.
    let full: &'static TeardownSupervisor = Box::leak(Box::new(TeardownSupervisor::for_test(0, 0)));
    assert!(
        full.reserve().is_none(),
        "a reservation at capacity must be refused before any child exists"
    );
    let exhausted: &'static TeardownSupervisor =
        Box::leak(Box::new(TeardownSupervisor::for_test(4, u64::MAX)));
    assert!(
        exhausted.reserve().is_none(),
        "token exhaustion must refuse rather than wrap"
    );
    assert_eq!(
        PtyError::TeardownCapacity.kind(),
        PtyFailureKind::SpawnRefused
    );
    let one: &'static TeardownSupervisor = Box::leak(Box::new(TeardownSupervisor::for_test(1, 0)));
    let held = one.reserve().expect("the first slot is available");
    assert!(
        one.reserve().is_none(),
        "an unresolved teardown keeps its slot charged"
    );
    assert_eq!(
        one.debt(),
        ProcessTeardownDebt {
            queued: 0,
            claimed: 0,
            stalled: 0,
            reserved: 1,
            lost: 0,
        }
    );
    // A charged slot alone withholds the exit: it is proof that some session or
    // teardown exists even when every other count reads empty.
    assert!(
        matches!(one.poll_exit_authority(), ExitAuthority::Held(_)),
        "a charged reservation must never read as clear"
    );
    drop(held);
    assert_eq!(
        one.debt().reserved,
        0,
        "a completed teardown returns its slot"
    );
    assert_eq!(one.poll_exit_authority(), ExitAuthority::Clear);

    // A child that ignores TERM: the pass that settles it is long, so it must
    // not run on the thread that hands the owner over.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let marker = format!(
        "enzui-pty-submit-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let _cleanup = MarkedProcessCleanup(marker.clone());
    let resistant = PtySession::spawn_shutdown_probe("submit-probe", &marker)
        .expect("the real PTY shutdown probe must spawn");
    let resistant_token = resistant.teardown_token();
    assert!(
        teardown_supervisor().debt().reserved >= 1,
        "a live session charges the process capacity"
    );

    // The process executor is shared by every test in this binary, so its
    // admission may be full for a moment; a refusal hands the owner back and
    // it is offered again. Only the admitted call is timed.
    let mut owner = resistant.into_debt_owner(PtyTeardownOutcome::pending());
    let admission_deadline = Instant::now() + Duration::from_secs(30);
    let submitted = loop {
        let started = Instant::now();
        match teardown_supervisor().submit(owner) {
            Ok(()) => break started.elapsed(),
            Err(returned) => {
                assert!(
                    Instant::now() < admission_deadline,
                    "the executor never admitted the owner: {returned:?}"
                );
                owner = returned;
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    assert!(
        submitted <= Duration::from_millis(100),
        "submission must run no signal, wait or join pass: {submitted:?}"
    );
    assert!(
        teardown_supervisor().holds_token(resistant_token),
        "the executor holds the exact owner it was handed"
    );
    assert!(
        teardown_supervisor().debt().reserved >= 1,
        "an unresolved teardown stays charged against capacity"
    );

    // One token's settlement is its own. A private executor is used so the
    // process executor's own rounds cannot claim these owners first.
    let private = TeardownSupervisor::for_test(8, 0);
    let settling = PtySession::spawn_shell("submit-settling").expect("a real PTY session");
    let settling_token = settling.teardown_token();
    let mut blocked = PtySession::spawn_shell("submit-blocked").expect("a real PTY session");
    blocked.force_incomplete_teardown();
    let blocked_token = blocked.teardown_token();
    assert_ne!(
        settling_token, blocked_token,
        "each reservation must mint its own token"
    );
    private.adopt(settling.into_debt_owner(PtyTeardownOutcome::pending()));
    private.adopt(blocked.into_debt_owner(PtyTeardownOutcome::pending()));
    assert_eq!(
        private.sweep_token(settling_token),
        None,
        "an owner settles on its own retained resources"
    );
    assert!(
        private
            .sweep_token(blocked_token)
            .is_some_and(|outcome| !outcome.is_complete()),
        "one token's settlement must not discharge another owner"
    );
    assert!(private.allow_complete_for_test(blocked_token));
    // The allowed pass settles once its child is reaped, which a loaded run
    // may not do on the first sweep.
    let sweep_deadline = Instant::now() + Duration::from_secs(30);
    while private.sweep_token(blocked_token).is_some() {
        assert!(
            Instant::now() < sweep_deadline,
            "the allowed owner never settled"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        private.sweep_token(blocked_token),
        None,
        "a settled token is no longer retained"
    );

    let settle_deadline = Instant::now() + Duration::from_secs(12);
    while teardown_supervisor().holds_token(resistant_token) && Instant::now() < settle_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !teardown_supervisor().holds_token(resistant_token),
        "the executor must finish the resistant owner on its own thread"
    );
}

/// A replacement root is a hard stop before `/proc` membership can authorize
/// a numeric SID. This deterministic state seam models PID/SID reuse after the
/// original leader was reaped and proves the replacement has no signal path.
#[test]
#[cfg(target_os = "linux")]
fn reused_session_number_cannot_authorize_signals() {
    let identity = LinuxProcessIdentity::capture(std::process::id(), None, None)
        .expect("the test process must have a pidfd and proc stat");
    let signals = AtomicU64::new(0);
    let state = identity.state_with(|_| Ok(identity.start_ticks.saturating_add(1)));
    let emitted = identity
        .signal_session_with(state, true, &mut Vec::new(), |_| {
            signals.fetch_add(1, Ordering::Relaxed);
            true
        })
        .expect("a replacement is rejected before enumeration");
    assert_eq!(state, LeaderIdentityState::Replaced);
    assert_eq!(emitted, 0);
    assert_eq!(signals.load(Ordering::Relaxed), 0);
}

/// Dropping a live PTY owns its leader, job-control group, setsid descendant,
/// and both worker threads, but not an unrelated process opening that slave.
/// Sabotage by removing the capability check from `process_belongs_to_pty`:
/// shutdown kills the path-only process and its positive liveness witness
/// fails, while RAII still reaps the fixture. Removing the reader cancellation
/// signal instead strands join on that surviving holder until its bounded sleep.
#[test]
#[cfg(target_os = "linux")]
fn dropping_live_session_reaps_group_and_joins_workers_within_budget() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let marker = format!(
        "enzui-pty-drop-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let _cleanup = MarkedProcessCleanup(marker.clone());
    let session = PtySession::spawn_shutdown_probe("drop-probe", &marker)
        .expect("the real PTY shutdown probe must spawn");
    let leader = session.process_id().expect("Unix PTY child has a pid");
    let witnesses = session.worker_witnesses();

    let live_deadline = Instant::now() + Duration::from_secs(2);
    let (job, escaped) = loop {
        let pids = all_marked_pids(&marker);
        let job = pids.iter().copied().find(|pid| {
            *pid != leader
                && process_session_of(*pid) == Some(leader)
                && process_group_of(*pid) != Some(leader)
        });
        let escaped = pids
            .iter()
            .copied()
            .find(|pid| *pid != leader && process_session_of(*pid) != Some(leader));
        if pids.contains(&leader)
            && let (Some(job), Some(escaped)) = (job, escaped)
        {
            break (job, escaped);
        }
        assert!(
            Instant::now() < live_deadline,
            "fixture must expose its leader, job-control child, and setsid child; got {pids:?}"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        process_group_of(leader),
        Some(leader),
        "PTY child must be the process-group leader"
    );
    assert_ne!(
        process_group_of(job),
        Some(leader),
        "job-control fixture must use another process group"
    );
    assert_eq!(process_session_of(job), Some(leader));
    assert_ne!(
        process_session_of(escaped),
        Some(leader),
        "env-scrubbed setsid descendant must have escaped the leader's session"
    );
    assert_ne!(
        process_parent_of(escaped),
        Some(leader),
        "double-forked setsid fixture must require exact PTY-fd ownership"
    );

    let tty = session
        .tty_name_for_tests()
        .expect("Linux PTY must expose its slave path");
    let mut unrelated =
        UnrelatedPtyHolder::spawn(&tty, &marker).expect("spawn unrelated same-user PTY holder");
    let unrelated_pid = unrelated.pid();
    assert!(
        process_has_open_path(unrelated_pid, &tty),
        "unrelated process never opened the PTY slave"
    );
    assert!(
        !session.capability_held_by(unrelated_pid),
        "an unrelated opener of the slave must never hold the capability; it holds {:#?}",
        unrelated.descriptors()
    );
    assert!(
        session.capability_held_by(escaped),
        "the env-scrubbed setsid descendant must still hold the inherited \
         capability descriptor"
    );
    let scrubbed_environment =
        std::fs::read(format!("/proc/{escaped}/environ")).expect("read descendant environment");
    assert!(
        !String::from_utf8_lossy(&scrubbed_environment).contains("ENZUI"),
        "the fixture descendant must have scrubbed every inherited marker string"
    );

    let owned = session.owned_process_ids();
    assert!(owned.contains(&leader), "leader missing from ownership set");
    assert!(
        owned.contains(&job),
        "job-control child missing from ownership set"
    );
    assert!(
        owned.contains(&escaped),
        "setsid child retaining the PTY slave missing from ownership set"
    );
    let token = session.teardown_token();
    let started = Instant::now();
    drop(session);
    let submitted = started.elapsed();
    assert!(
        unrelated.is_alive(),
        "closing the PTY killed an unrelated same-user slave-fd holder"
    );
    // Drop may run on the reducer, so it hands exact custody over instead of
    // signalling, waiting and joining in place.
    assert!(
        submitted <= Duration::from_millis(250),
        "a fallback drop must transfer custody rather than tear down in place: {submitted:?}"
    );

    // Shared with every parallel test's teardown, each allowed its TERM grace.
    let settle_deadline = Instant::now() + Duration::from_secs(30);
    while teardown_supervisor().holds_token(token) && Instant::now() < settle_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !teardown_supervisor().holds_token(token),
        "the executor must settle the transferred owner"
    );
    let elapsed = started.elapsed();

    let gone_deadline = Instant::now() + Duration::from_secs(1);
    while [leader, job, escaped]
        .iter()
        .any(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists())
        && Instant::now() < gone_deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    for (kind, pid) in [
        ("leader", leader),
        ("job", job),
        ("setsid descendant", escaped),
    ] {
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "the marked {kind} {pid} survived its PTY session"
        );
    }
    assert!(
        elapsed <= Duration::from_secs(12),
        "owned shutdown exceeded its measured ceiling: {elapsed:?}"
    );
    assert!(witnesses.exit_waiter_completed(), "waiter did not complete");
    assert!(witnesses.reader_completed(), "reader did not complete");
    assert!(witnesses.exit_waiter_joined(), "waiter was left detached");
    assert!(witnesses.reader_joined(), "reader was left detached");
    assert!(
        witnesses.final_freeze_scans() >= 2,
        "final pre-KILL freeze did not rescan to a stable snapshot"
    );
    assert!(
        witnesses.final_freeze_reached_stability(),
        "final pre-KILL freeze ran out of deadline instead of observing two \
         consecutive empty snapshots"
    );
}
