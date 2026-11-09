//! SSH memfd descriptors.

use super::*;

/// Direct controls for the descriptor itself, not inferred from successful
/// delivery: FD_CLOEXEC is set, the kernel-visible name is neutral, and the
/// complete seal set is exactly WRITE|GROW|SHRINK|SEAL plus EXEC iff creation
/// used `MFD_NOEXEC_SEAL`.
///
/// Sabotage controls (independent assertions): remove CLOEXEC → fd-flags red;
/// restore a credential-bearing memfd name → neutral-name red; omit or add any
/// seal → exact-set red; stop requesting NOEXEC on a supporting kernel → the
/// EXEC/noexec correspondence reds.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_memfd_has_cloexec_neutral_name_and_exact_seals() {
    let secret = PasswordSecret::create("S3NT-INEL").expect("create production memfd");
    let fd_flags = rustix::io::fcntl_getfd(&secret._memfd).expect("read fd flags");
    assert!(
        fd_flags.contains(rustix::io::FdFlags::CLOEXEC),
        "the retained password descriptor must be FD_CLOEXEC: {fd_flags:?}"
    );

    let target = std::fs::read_link(secret.proc_path())
        .expect("read the memfd magic-link target")
        .to_string_lossy()
        .into_owned();
    assert!(
        target.contains("memfd:enzui-pty-data")
            && !target.to_ascii_lowercase().contains("password")
            && !target.to_ascii_lowercase().contains("secret")
            && !target.to_ascii_lowercase().contains("ssh"),
        "the kernel-visible memfd name must be neutral: {target:?}"
    );

    let mut expected = rustix::fs::SealFlags::WRITE
        | rustix::fs::SealFlags::GROW
        | rustix::fs::SealFlags::SHRINK
        | rustix::fs::SealFlags::SEAL;
    if secret._used_noexec_seal {
        expected |= rustix::fs::SealFlags::EXEC;
    }
    let actual = rustix::fs::fcntl_get_seals(&secret._memfd).expect("read memfd seals");
    assert_eq!(
        actual, expected,
        "seal set must be exact; NOEXEC path must carry EXEC and fallback must not"
    );
}

/// `MFD_NOEXEC_SEAL` is attempted first. Only exact EINVAL may trigger one
/// retry with the base CLOEXEC|ALLOW_SEALING flags; any other error is returned
/// immediately. The successful EINVAL fixture then traverses the full
/// write/seek/seal/proc-preflight path, proving the fallback is usable rather
/// than merely selected.
///
/// Sabotage: omit NOEXEC on attempt 1, broaden the fallback condition, or retry
/// more than once → the attempt/flag assertions red independently.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_noexec_uses_einval_only_compatibility_fallback() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&attempts);
    let mut call = 0usize;
    let secret = PasswordSecret::create_with(
        "S3NT-INEL",
        move |flags| {
            seen.lock().unwrap().push(flags);
            call += 1;
            if call == 1 {
                Err(std::io::Error::from_raw_os_error(
                    std::io::Error::from(rustix::io::Errno::INVAL)
                        .raw_os_error()
                        .expect("EINVAL has an OS code"),
                ))
            } else {
                rustix::fs::memfd_create("enzui-pty-data", flags).map_err(std::io::Error::from)
            }
        },
        PasswordSecret::proc_path_for,
    )
    .expect("EINVAL-only compatibility fallback must complete");
    assert!(
        !secret._used_noexec_seal,
        "the EINVAL fallback must record that NOEXEC was not used"
    );
    let attempts = attempts.lock().unwrap();
    assert_eq!(attempts.len(), 2, "exactly one compatibility retry");
    assert!(
        attempts[0].contains(rustix::fs::MemfdFlags::NOEXEC_SEAL),
        "first attempt must request NOEXEC_SEAL"
    );
    assert!(
        !attempts[1].contains(rustix::fs::MemfdFlags::NOEXEC_SEAL),
        "EINVAL retry must remove only NOEXEC_SEAL"
    );
    assert_eq!(
        attempts[0] & !rustix::fs::MemfdFlags::NOEXEC_SEAL,
        attempts[1],
        "fallback must preserve every base flag"
    );
    drop(attempts);
    drop(secret);

    let calls = Arc::new(AtomicU64::new(0));
    let seen_calls = Arc::clone(&calls);
    let err = PasswordSecret::create_with(
        "S3NT-INEL",
        move |_flags| {
            seen_calls.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::from_raw_os_error(1))
        },
        PasswordSecret::proc_path_for,
    )
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "non-EINVAL must not retry");
    assert_ne!(
        err.raw_os_error(),
        std::io::Error::from(rustix::io::Errno::INVAL).raw_os_error(),
        "control error must be non-EINVAL"
    );
}
