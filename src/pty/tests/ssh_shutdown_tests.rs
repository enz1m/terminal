//! SSH shutdown secrets.

use super::*;

/// Child-test entry point. It is ignored in the ordinary harness and can only
/// run when the parent supplies the private marker.
#[test]
#[ignore = "child-test entry point: launched only by its owner test via the private ENZUI_2511_SHUTDOWN_CHILD marker"]
#[cfg(target_os = "linux")]
fn ssh_2511_shutdown_child_probe() {
    assert_eq!(
        std::env::var_os("ENZUI_2511_SHUTDOWN_CHILD").as_deref(),
        Some(std::ffi::OsStr::new("1")),
        "this ignored probe must be launched by its owner test"
    );
    ssh_2511_shutdown_probe_body();
}

/// ORDERING + BOUND: own the potentially wedged shutdown in a subprocess. On
/// success, that child proves the `/proc` path stayed live through reader exit,
/// the exact PTY child was reaped before the exact memfd inode's release event,
/// and the path vanished afterwards. On timeout, this owner kills descendant
/// PTY processes first, then kills and waits for the child-test process, so no
/// detached Rust thread, session child or memfd remains in this process.
///
/// Sabotage: move `_password_secret.take()` above teardown → the child exits
/// red on the live-path/release event assertions. Wedge teardown → the parent
/// reaches its timeout, kills descendants and the child, reaps it, and reds.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_shutdown_releases_secret_only_after_teardown() {
    let exe = std::env::current_exe().expect("locate this unit-test executable");
    let module = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, path)| path);
    let probe = format!("{module}::ssh_2511_shutdown_child_probe");
    let listed = std::process::Command::new(&exe)
        .args(["--list", "--ignored", "--exact", &probe])
        .output()
        .expect("list the owned shutdown probe");
    assert!(listed.status.success(), "listing the child probe failed");
    assert!(
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .any(|line| line == format!("{probe}: test")),
        "the exact child probe must exist; an empty libtest run proves no shutdown behavior"
    );
    let mut child = std::process::Command::new(exe)
        .arg("--exact")
        .arg(&probe)
        .arg("--ignored")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env("ENZUI_2511_SHUTDOWN_CHILD", "1")
        .spawn()
        .expect("spawn owned shutdown child-test process");
    let owner_pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait().expect("poll shutdown child-test") {
            Some(status) => {
                assert!(status.success(), "shutdown child-test failed: {status}");
                break;
            }
            None if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            None => {
                // Backstop: kill every descendant while its parent still
                // exists, giving the harness a chance to reap its PTY child;
                // then kill+wait the harness itself. Kernel exit closes every
                // memfd owned by that process.
                let descendants = ssh_2511_descendants_of(owner_pid);
                for (pid, _start_ticks) in &descendants {
                    if let Ok(pidfd) = open_pidfd(*pid) {
                        let _ = rustix::process::pidfd_send_signal(
                            &pidfd,
                            rustix::process::Signal::KILL,
                        );
                    }
                }
                thread::sleep(Duration::from_millis(100));
                let _ = child.kill();
                let _ = child.wait();
                for (pid, start_ticks) in descendants {
                    let gone = (0..200).any(|_| {
                        if !read_process_stat(pid).is_ok_and(|stat| stat.start_ticks == start_ticks)
                        {
                            true
                        } else {
                            thread::sleep(Duration::from_millis(10));
                            false
                        }
                    });
                    assert!(
                        gone,
                        "shutdown timeout cleanup left exact descendant {pid}/{start_ticks} alive"
                    );
                }
                panic!("shutdown child-test exceeded the 20s bound; owner killed and reaped it");
            }
        }
    }
    assert!(
        read_process_stat(owner_pid).is_err(),
        "the child-test owner must be reaped after success"
    );
}

#[cfg(target_os = "linux")]
fn ssh_2511_descendants_of(root: u32) -> Vec<(u32, u64)> {
    let mut snapshot = std::collections::HashMap::new();
    for entry in std::fs::read_dir("/proc")
        .expect("scan /proc for timeout cleanup")
        .filter_map(Result::ok)
    {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if let Ok(stat) = read_process_stat(pid) {
            snapshot.insert(pid, stat);
        }
    }
    snapshot
        .iter()
        .filter_map(|(&pid, stat)| {
            let mut cursor = stat.parent;
            let mut seen = std::collections::HashSet::new();
            while cursor != 0 && seen.insert(cursor) {
                if cursor == root {
                    return Some((pid, stat.start_ticks));
                }
                cursor = snapshot.get(&cursor).map_or(0, |parent| parent.parent);
            }
            None
        })
        .collect()
}
