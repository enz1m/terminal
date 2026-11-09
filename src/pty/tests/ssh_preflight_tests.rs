//! SSH preflights.

use super::*;

/// The preflight refuses a `-f` path that does not name the retained object.
/// A path pointing at a DIFFERENT descriptor of this process is openable and
/// readable, so only the `(dev,ino)` identity comparison can reject it — a
/// mere "can I open it" check would pass and hand sshpass a foreign object.
/// This is exactly the fd-number-reuse hazard in static form.
///
/// Sabotage: delete the `(dev,ino)` comparison in `preflight` (or delete the
/// whole preflight call) → the foreign path is accepted → the `unwrap_err`
/// reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_preflight_refuses_path_naming_a_foreign_object() {
    let dir = ssh_2511_tmp_dir("preflight-mismatch");
    std::fs::create_dir_all(&dir).unwrap();
    let foreign = dir.join("foreign");
    std::fs::write(&foreign, b"S3NT-INEL\n").unwrap();
    // A real, open, readable descriptor of THIS process that is not the memfd.
    let held = std::fs::File::open(&foreign).expect("open foreign file");
    let foreign_fd_path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        std::os::fd::AsRawFd::as_raw_fd(&held)
    ));
    // Control: that path really is openable and yields the same bytes, so the
    // refusal below cannot be explained by it being unreadable.
    assert_eq!(
        std::fs::read(&foreign_fd_path).expect("the foreign fd path must be readable"),
        b"S3NT-INEL\n",
        "control: the foreign path is openable and content-identical"
    );

    let err = PasswordSecret::create_with(
        "S3NT-INEL",
        |flags| rustix::fs::memfd_create("enzui-pty-data", flags).map_err(std::io::Error::from),
        |_memfd| foreign_fd_path.clone(),
    )
    .unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::InvalidInput,
        "a path naming a foreign object must be refused by identity, got {err:?}"
    );
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The preflight refuses an unopenable `-f` path (the shape a missing/masked
/// `/proc`, a wrong pid, or a ptrace-mode/LSM denial takes) as a typed setup
/// failure, instead of letting it become an sshpass hang.
///
/// Sabotage: delete the preflight call in `create_with` → the unopenable path
/// is accepted → the `unwrap_err` reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_preflight_refuses_unopenable_path() {
    let err = PasswordSecret::create_with(
        "S3NT-INEL",
        |flags| rustix::fs::memfd_create("enzui-pty-data", flags).map_err(std::io::Error::from),
        |_memfd| PathBuf::from("/proc/0/fd/0"),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("not openable by this process"),
        "an unopenable channel path must be a typed setup failure, got {err:?}"
    );
}

/// The preflight refuses a DECOY `memfd` — openable, same flavour of object,
/// but a different one holding different bytes. This is the closest analogue to
/// "sshpass opened something that is not our secret".
///
/// Scope note, deliberately not overclaimed: the `(dev,ino)` identity check
/// runs before the payload comparison, so this test reddens on the identity
/// branch. The payload comparison is defence in depth and is NOT independently
/// reddenable here, because a path that names the very same sealed object
/// cannot yield different bytes once `F_SEAL_WRITE` is set.
///
/// Sabotage: delete the preflight call (or its identity comparison) in
/// `create_with` → the decoy is accepted → the `unwrap_err` reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_preflight_refuses_decoy_object() {
    use std::io::Write as _;
    // A decoy memfd with different contents, kept open for the call.
    let decoy = rustix::fs::memfd_create("enzui-pty-data", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("decoy memfd");
    let mut decoy_file = std::fs::File::from(decoy);
    decoy_file
        .write_all(b"WRONG-CONTENTS\n")
        .expect("write decoy");
    let decoy_path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        std::os::fd::AsRawFd::as_raw_fd(&decoy_file)
    ));
    assert!(
        std::fs::File::open(&decoy_path).is_ok(),
        "control: the decoy path is openable"
    );

    let err = PasswordSecret::create_with(
        "S3NT-INEL",
        |flags| rustix::fs::memfd_create("enzui-pty-data", flags).map_err(std::io::Error::from),
        |_memfd| decoy_path.clone(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err.kind(),
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData
        ),
        "a wrong-content channel path must be refused, got {err:?}"
    );
    drop(decoy_file);
}

/// The fd-number reuse hazard is REAL, demonstrated directly: closing the
/// retained descriptor frees its NUMBER, and the very next open in this process
/// is handed the same number, so the previously-published
/// `/proc/<pid>/fd/<n>` path then names an unrelated file. This is the positive
/// control for the shutdown-ordering guarantee: it shows what an early release
/// would expose a lazily-opening sshpass to.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_closed_fd_number_is_recycled_by_a_foreign_object() {
    let dir = ssh_2511_tmp_dir("fd-recycle");
    std::fs::create_dir_all(&dir).unwrap();
    let impostor = dir.join("impostor");
    std::fs::write(&impostor, b"IMPOSTOR\n").unwrap();

    let secret = PasswordSecret::create("S3NT-INEL").expect("create memfd secret");
    let published = secret.proc_path().to_path_buf();
    assert_eq!(
        std::fs::read(&published).expect("read while retained"),
        b"S3NT-INEL\n",
        "while retained the published path yields the password"
    );

    // Early release: the number is freed.
    drop(secret);
    // The next open claims the lowest free number, i.e. the one just freed.
    let recycled = std::fs::File::open(&impostor).expect("open impostor");
    let recycled_path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        std::os::fd::AsRawFd::as_raw_fd(&recycled)
    ));
    if recycled_path == published {
        // The hazard materialised: the published path now names a FOREIGN file.
        assert_eq!(
            std::fs::read(&published).expect("read recycled path"),
            b"IMPOSTOR\n",
            "the recycled fd number makes the published path name a foreign object"
        );
    } else {
        // Number not reclaimed on this run; the published path must at least no
        // longer serve the password.
        assert_ne!(
            std::fs::read(&published).unwrap_or_default(),
            b"S3NT-INEL\n".to_vec(),
            "a released channel must not keep serving the password"
        );
    }
    drop(recycled);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The secret channel has NO filesystem pathname: the `-f` argument is a
/// `/proc/<pid>/fd/<n>` magic path, not a real file, and the anonymous object
/// is freed the moment its last reference closes. While the `PasswordSecret`
/// is held the path resolves and reads back the payload; once it is dropped
/// (and no child holds the object open) the path stops resolving.
///
/// Sabotage: give `PasswordSecret` a real on-disk file / leak the descriptor
/// (never close it) → the post-drop open no longer fails → the ENOENT
/// assertion reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_memfd_vanishes_when_refs_close() {
    let secret = PasswordSecret::create("S3NT-INEL").expect("create memfd secret");
    let proc_path = secret.proc_path().to_path_buf();

    // The magic path names no filesystem object: its lexical target is a memfd
    // magic link, and there is no directory entry to unlink or leak.
    assert!(
        proc_path.starts_with("/proc/"),
        "the channel must be a /proc magic path, not a filesystem file: {proc_path:?}"
    );
    // Held: it resolves and reads back the payload.
    assert_eq!(
        std::fs::read(&proc_path).expect("read held memfd"),
        b"S3NT-INEL\n",
        "the held memfd must read back the exact payload"
    );

    drop(secret);

    // Dropped and no child open: the last reference closed and the object is
    // gone. A parallel test may reuse the descriptor number at once, so the
    // claim is that the entry no longer names this memfd, not that it is empty.
    let still_ours = std::fs::read_link(&proc_path)
        .is_ok_and(|target| target.to_string_lossy().starts_with("/memfd:"))
        && std::fs::read(&proc_path).is_ok_and(|bytes| bytes == b"S3NT-INEL\n");
    assert!(
        !still_ours,
        "after the last reference closes, the memfd path must stop resolving: {proc_path:?}"
    );
}
