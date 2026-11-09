//! SSH spawn routing.

use super::*;

/// PRODUCTION-ROUTED: `PtySession::spawn_ssh_with` (the injectable seam behind
/// `spawn_ssh`) drives openpty → `build_ssh_command_with` → `finish_spawn_with`
/// → retain-the-secret end to end. It proves (a) the child is launched with the
/// exact `-f <proc_path> <ssh> … -- <dest> <remote>` argv, (b) the session OWNS
/// the memfd whose `-f` path it hands sshpass, (c) a same-uid child launched
/// THROUGH portable-pty can open that `/proc/<pid>/fd/<n>` path and read the
/// exact secret bytes, and (d) `shutdown` releases the descriptor after child
/// teardown while the session object is still alive.
///
/// Sabotage: drop the `session._password_secret = password_channel` assignment
/// in `spawn_ssh_with` → (b) reddens; delete `c.arg("--")` → (a) reddens; break
/// the memfd seal/`/proc` path so the child cannot read → (c) reddens; remove
/// the `_password_secret.take()` in `shutdown` → (d) reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_spawn_ssh_routes_command_owns_and_reads_secret() {
    let argv_dir = ssh_2511_tmp_dir("spawnssh-argv");
    std::fs::create_dir_all(&argv_dir).unwrap();
    let argv_file = argv_dir.join("argv");
    let secret_file = argv_dir.join("secret");

    let ssh_bin = ssh_2511_stub_executable("spawnssh-ssh");
    let sshpass_bin = ssh_2511_secret_capture_stub("spawnssh-sshpass", &argv_file, &secret_file);
    let host = ssh_2511_host("example.net", "");
    let resolve = ssh_2511_resolver(vec![("ssh", ssh_bin.clone()), ("sshpass", sshpass_bin)]);

    let session = match PtySession::spawn_ssh_with(
        "spawnssh-route",
        &host,
        Some("S3NT-INEL"),
        resolve,
        default_reader_spawner,
    ) {
        Ok(session) => session,
        Err(PtyError::OpenPty(err)) => {
            let _ = std::fs::remove_dir_all(&argv_dir);
            panic!(
                "this test REQUIRES a real PTY; openpty failed: {err}. A routed \
                 test must fail loudly, never report green by skipping"
            );
        }
        Err(other) => panic!("spawn_ssh_with failed unexpectedly: {other:?}"),
    };
    let mut session = session;

    // (b) the session owns the secret; its -f path names our own descriptor.
    let proc_path = session
        ._password_secret
        .as_ref()
        .expect("spawn_ssh must retain the password memfd in the session")
        .proc_path()
        .to_path_buf();
    assert!(
        proc_path.starts_with(format!("/proc/{}/fd/", std::process::id())),
        "the retained memfd -f path must name THIS process's descriptor: {proc_path:?}"
    );

    // (a) the child was launched with the routed argv. Poll for the record.
    let argv = ssh_2511_wait_for_file_lines(&argv_file);
    let dd = argv
        .iter()
        .position(|a| a == "--")
        .unwrap_or_else(|| panic!("routed argv must contain `--`; argv={argv:?}"));
    assert_eq!(
        argv.get(dd + 1).map(String::as_str),
        Some("example.net"),
        "the destination must be isolated immediately after `--`; argv={argv:?}"
    );
    assert_eq!(
        argv.first().map(String::as_str),
        Some("-f"),
        "argv={argv:?}"
    );
    assert_eq!(
        argv.get(1).map(String::as_str),
        Some(proc_path.to_string_lossy().as_ref()),
        "sshpass `-f` must name the retained memfd path; argv={argv:?}"
    );
    assert!(
        argv.iter().any(|a| a == ssh_bin.to_string_lossy().as_ref()),
        "the resolved absolute ssh path must be the sshpass command arg; argv={argv:?}"
    );

    // (c) the same-uid child, launched through portable-pty (fd >= 3 closed),
    // opened the /proc path and read the EXACT secret bytes.
    let captured = ssh_2511_wait_for_bytes(&secret_file);
    assert_eq!(
        captured, b"S3NT-INEL\n",
        "the child must read exactly the password line through the /proc memfd path"
    );

    // (d) shutdown releases the descriptor after teardown, session still in
    // scope. A loaded run may need more than one pass to finish the teardown.
    let teardown_deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        session.shutdown();
        if session._password_secret.is_none() || std::time::Instant::now() >= teardown_deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        session._password_secret.is_none(),
        "shutdown must take the secret out once child teardown completes"
    );
    // A parallel test may reuse the descriptor number at once, so the claim is
    // that the entry no longer names this memfd.
    let still_ours = std::fs::read_link(&proc_path)
        .is_ok_and(|target| target.to_string_lossy().starts_with("/memfd:"))
        && std::fs::read(&proc_path).is_ok_and(|bytes| bytes == b"S3NT-INEL\n");
    assert!(
        !still_ours,
        "shutdown must release the retained memfd immediately"
    );
    let _ = std::fs::remove_dir_all(&argv_dir);
}

/// PRODUCTION-ROUTED spawn-failure cleanup: when the reader-thread spawn fails
/// (the injected `eagain_spawner`, the honest way to force a `finish_spawn`
/// error), `spawn_ssh_with` propagates the error and the password channel is
/// dropped as it unwinds — the descriptor is released and no secret channel
/// survives the failed spawn.
///
/// Sabotage: retain the channel past the error (e.g. `std::mem::forget` the
/// password channel on the failure path) → the exact memfd `(dev,ino)` remains
/// in the after-inventory → the identity assertion reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_spawn_ssh_failure_leaks_no_secret_descriptor() {
    // POSITIVE CONTROL: the inventory can actually see a live secret
    // descriptor, so "no new secret descriptor" below is a real absence claim
    // and not a blind probe that never observes anything.
    let held = PasswordSecret::create("S3NT-INEL").expect("control secret");
    let held_identity = held.identity;
    assert!(
        ssh_2511_secret_identity_set().contains(&held_identity),
        "control: a retained secret must be visible in the /proc/self/fd inventory"
    );
    drop(held);
    assert!(
        !ssh_2511_secret_identity_set().contains(&held_identity),
        "control: dropping the secret must remove it from the inventory"
    );

    let ssh_bin = ssh_2511_stub_executable("failssh-ssh");
    let sshpass_bin = ssh_2511_stub_executable("failssh-sshpass");
    let host = ssh_2511_host("example.net", "");
    let resolve = ssh_2511_resolver(vec![("ssh", ssh_bin), ("sshpass", sshpass_bin)]);

    let before = ssh_2511_secret_identity_set();
    let result = PtySession::spawn_ssh_with(
        "spawnssh-fail",
        &host,
        Some("S3NT-INEL"),
        resolve,
        eagain_spawner,
    );
    match result {
        Err(PtyError::OpenPty(err)) => panic!(
            "this test REQUIRES a real PTY; openpty failed: {err}. A routed test \
             must fail loudly, never report green by skipping"
        ),
        Err(PtyError::SpawnReader(_)) => {}
        other => panic!("expected a reader-spawn failure, got {other:?}"),
    }
    assert!(
        ssh_2511_no_new_secret_identity(&before),
        "a failed spawn must leak no password memfd descriptor; one outside the \
         baseline {before:?} was still open"
    );
}

/// Poll for a stub's line-oriented dump (the child execs asynchronously after
/// `spawn_command` returns), returning the recorded lines.
#[cfg(target_os = "linux")]
fn ssh_2511_wait_for_file_lines(path: &std::path::Path) -> Vec<String> {
    for _ in 0..200 {
        if let Ok(contents) = std::fs::read_to_string(path)
            && !contents.is_empty()
        {
            return contents.lines().map(str::to_owned).collect();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the routed child never recorded its output at {path:?}");
}

/// Poll for a stub's raw byte capture, returning the recorded bytes.
#[cfg(target_os = "linux")]
fn ssh_2511_wait_for_bytes(path: &std::path::Path) -> Vec<u8> {
    for _ in 0..200 {
        if let Ok(bytes) = std::fs::read(path)
            && !bytes.is_empty()
        {
            return bytes;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the routed child never captured the secret bytes at {path:?}");
}
