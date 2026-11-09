//! Start time identity.

use super::*;

/// Calibrate both the real `/proc` start-time reader and the injected state
/// seam. A stale numeric leader must classify as replaced even before any
/// session enumeration, while ENOENT is the distinct gone state.
#[test]
#[cfg(target_os = "linux")]
fn start_time_identity_state_seam_is_calibrated() {
    let identity = LinuxProcessIdentity::capture(std::process::id(), None, None)
        .expect("the test process must have a pidfd and proc stat");
    assert_eq!(
        process_start_ticks(identity.pid).expect("read current process start time"),
        identity.start_ticks
    );
    assert_eq!(identity.state(), LeaderIdentityState::Original);
    assert_eq!(
        identity.state_with(|_| Ok(identity.start_ticks)),
        LeaderIdentityState::Original
    );
    assert_eq!(
        identity.state_with(|_| Ok(identity.start_ticks.saturating_add(1))),
        LeaderIdentityState::Replaced
    );
    assert_eq!(
        identity.state_with(|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
        LeaderIdentityState::Gone
    );
    assert_eq!(
        identity.state_with(|_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))),
        LeaderIdentityState::Unknown,
        "an unreadable leader is neither gone nor recycled, so it stays debt"
    );
}

/// A child discovered after STOP resets stability: final teardown must scan
/// again and then observe two consecutive empty snapshots before KILL. This is
/// the deterministic seam for a process already crossing `fork` as STOP lands.
#[test]
#[cfg(target_os = "linux")]
fn final_freeze_rescans_after_a_late_fork_until_stable() {
    let discoveries = [1, 1, 0, 0, 0];
    let mut index = 0;
    let outcome = rescan_until_stable(
        Instant::now() + Duration::from_secs(1),
        || {
            let discovered = discoveries[index];
            index += 1;
            discovered
        },
        || {},
    );
    assert_eq!(
        outcome.scans, 4,
        "late discoveries must reset the stable scan count"
    );
    assert!(
        outcome.reached_stability,
        "two consecutive empty snapshots are the stable state"
    );
    assert_eq!(index, 4, "KILL may follow only two stable snapshots");
    let exhausted = rescan_until_stable(Instant::now(), || 0, || {});
    assert_eq!(exhausted.scans, 0);
    assert!(
        !exhausted.reached_stability,
        "a sweep that ran out of deadline has not proved the fork window closed"
    );
}
