//! SSH confirmations.

use super::*;

/// Both branches force one confirming host-key policy, so an unknown or
/// changed key ends the launch until the user accepts that exact fingerprint.
/// A weaker value anywhere in argv would let an sshpass-delivered secret reach
/// an unverified host.
#[test]
fn ssh_host_key_confirmation_is_forced_in_both_branches() {
    let host = ssh_2511_host("h", "");
    let ssh_bin = ssh_2511_stub_executable("hostkey-ssh");
    let sshpass_bin = ssh_2511_stub_executable("hostkey-sshpass");

    let (via_pass, _secret) = build_ssh_command_with(
        &host,
        Some("pw"),
        ssh_2511_resolver(vec![("ssh", ssh_bin.clone()), ("sshpass", sshpass_bin)]),
    )
    .expect("build sshpass command");
    let (direct, _none) =
        build_ssh_command_with(&host, None, ssh_2511_resolver(vec![("ssh", ssh_bin)]))
            .expect("build direct command");

    for (branch, cmd) in [("sshpass", &via_pass), ("direct", &direct)] {
        let argv = argv_strings(cmd);
        let boundary = argv
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or_else(|| panic!("{branch} argv must contain a literal `--`; argv={argv:?}"));
        let policies: Vec<&str> = argv[..boundary]
            .windows(2)
            .filter(|pair| pair[0] == "-o" && pair[1].starts_with("StrictHostKeyChecking="))
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(
            policies,
            ["StrictHostKeyChecking=ask"],
            "{branch} branch must pass exactly one confirming host-key policy; argv={argv:?}"
        );
        for arg in &argv {
            assert!(
                !arg.starts_with("StrictHostKeyChecking=")
                    || arg.as_str() == "StrictHostKeyChecking=ask",
                "{branch} branch admits an unconfirmed host key: {arg:?}"
            );
        }
    }
}

/// A missing `ssh` executable is a typed setup failure resolved once before
/// branch selection — never a silent basename fallback that PATH could
/// re-resolve at spawn. Covers both branches (password present and absent).
///
/// Sabotage: restore the direct branch's `CommandBuilder::new("ssh")` fallback
/// → the direct case returns `Ok` → the `unwrap_err` reddens.
#[test]
fn ssh_2511_missing_ssh_is_typed_failure_not_basename_fallback() {
    let host = ssh_2511_host("h", "");

    // Direct branch: no ssh resolvable.
    let err = build_ssh_command_with(&host, None, ssh_2511_resolver(vec![])).unwrap_err();
    assert!(
        matches!(err, PtyError::Spawn(_)),
        "missing ssh (direct) must be a typed Spawn failure, got {err:?}"
    );

    // sshpass branch: sshpass resolvable but ssh missing still fails typed,
    // because ssh is resolved once before branch selection.
    let sshpass_bin = ssh_2511_stub_executable("missing-ssh-sshpass");
    let err = build_ssh_command_with(
        &host,
        Some("pw"),
        ssh_2511_resolver(vec![("sshpass", sshpass_bin)]),
    )
    .unwrap_err();
    assert!(
        matches!(err, PtyError::Spawn(_)),
        "missing ssh (sshpass) must be a typed Spawn failure, got {err:?}"
    );
}

/// A password at or above the delivery cap is refused as a typed error BEFORE
/// the `memfd` is created or any byte is written, so an oversized secret can
/// never be silently truncated or block a write.
///
/// Sabotage: delete the `MAX_PASSWORD_LEN` guard in `create_with` → the
/// oversized create returns `Ok` → the `unwrap_err` reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_password_over_limit_is_refused() {
    let huge = "a".repeat(PasswordSecret::MAX_PASSWORD_LEN);
    let err = PasswordSecret::create(&huge).unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::InvalidInput,
        "an over-limit password must be an InvalidInput refusal, got {err:?}"
    );
    // A password one byte under the cap is still accepted.
    let ok = PasswordSecret::create(&"a".repeat(PasswordSecret::MAX_PASSWORD_LEN - 1));
    assert!(
        ok.is_ok(),
        "a password under the cap must still be accepted"
    );
}

/// A failure to create the anonymous object (the injected `memfd_create` seam)
/// is a typed I/O error and no secret is written — the setup path is bounded.
///
/// Sabotage: make `create` ignore the seam and always succeed → the injected
/// failure no longer propagates → the `unwrap_err` reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_memfd_setup_failure_is_typed() {
    let err = PasswordSecret::create_with(
        "pw",
        |_flags| Err(std::io::Error::other("injected memfd_create failure")),
        PasswordSecret::proc_path_for,
    )
    .unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::Other,
        "an injected memfd_create failure must propagate as a typed error, got {err:?}"
    );
}
