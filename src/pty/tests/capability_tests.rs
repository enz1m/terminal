//! Capability handshakes.

use super::*;

#[test]
#[cfg(target_os = "linux")]
fn capability_path_is_removed_at_every_setup_failure() {
    for failure in [
        PtySetupFailure::Mkfifo,
        PtySetupFailure::Open,
        PtySetupFailure::Metadata,
        PtySetupFailure::MasterDup,
        PtySetupFailure::CancelPipe,
        PtySetupFailure::Spawn,
    ] {
        let label = format!("{failure:?}").to_ascii_lowercase();
        let marker = format!("enzui-capability-setup-{}-{label}", std::process::id());
        let _cleanup = MarkedProcessCleanup(marker.clone());
        let (fixture, path) = capability_test_path(&label);
        let path_was_visible = Arc::new(AtomicBool::new(false));
        let (pair, mut cmd) =
            real_pair_and_cmd(&marker).expect("the setup-failure witness requires a real PTY pair");
        fixture.configure(&mut cmd);
        let result = PtySession::finish_spawn_with_wake_bus_controlled(
            format!("setup-{label}"),
            pair,
            cmd,
            wake_bus(),
            default_reader_spawner,
            None,
            PtySpawnTestControl {
                capability_path: Some(path.clone()),
                path_was_visible: Some(Arc::clone(&path_was_visible)),
                setup_failure: Some(failure),
                ..PtySpawnTestControl::default()
            },
        );

        if matches!(
            failure,
            PtySetupFailure::Mkfifo | PtySetupFailure::Open | PtySetupFailure::Metadata
        ) {
            let session = result.expect("capability creation failure must use safe fallback");
            assert!(
                !session.has_capability(),
                "{failure:?} retained capability ownership that was never established"
            );
            drop(session);
        } else {
            assert!(result.is_err(), "{failure:?} injection did not fail setup");
        }
        if failure == PtySetupFailure::Mkfifo {
            assert!(
                !path_was_visible.load(Ordering::Acquire),
                "a failing mkfifo unexpectedly created a pathname"
            );
        } else {
            assert!(
                path_was_visible.load(Ordering::Acquire),
                "{failure:?} did not run after the FIFO became visible"
            );
        }
        assert!(
            !path.exists(),
            "{failure:?} leaked capability pathname {}",
            path.display()
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn failed_capability_handshake_unlinks_and_uses_identity_fallback() {
    let marker = format!("enzui-capability-no-handshake-{}", std::process::id());
    let _cleanup = MarkedProcessCleanup(marker.clone());
    let (fixture, path) = capability_test_path("failed-handshake");
    let path_was_visible = Arc::new(AtomicBool::new(false));
    let session = PtySession::spawn_failed_capability_handshake_probe(
        "failed-handshake",
        &marker,
        path.clone(),
        Arc::clone(&path_was_visible),
        &fixture,
    )
    .expect("a failed capability handshake must fall back to leader identity");
    let leader = session.process_id().expect("probe must have a Linux pid");

    assert!(
        path_was_visible.load(Ordering::Acquire),
        "the handshake witness never created its FIFO"
    );
    assert!(!path.exists(), "failed handshake leaked {}", path.display());
    assert!(
        !session.has_capability(),
        "failed handshake retained fictitious capability ownership"
    );
    assert!(
        session.owned_process_ids().contains(&leader),
        "leader/SID fallback did not own the live child"
    );
    assert!(
        std::path::Path::new(&format!("/proc/{leader}")).exists(),
        "fallback witness was not alive before teardown"
    );

    // Teardown runs on the process-wide executor, possibly queued behind
    // other sessions' TERM grace.
    drop(session);
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::path::Path::new(&format!("/proc/{leader}")).exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !std::path::Path::new(&format!("/proc/{leader}")).exists(),
        "identity fallback did not reap leader {leader}"
    );
}
