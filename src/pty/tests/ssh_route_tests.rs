//! SSH destinations.

use super::*;

/// The user-controlled destination must be positional after a literal `--` in
/// BOTH the sshpass and direct branches, so `-oProxyCommand=…` cannot be
/// parsed as an option that runs a local command.
///
/// Sabotage: delete the `c.arg("--")` in either branch → that branch's
/// fixture reddens (no `--` found / destination not isolated).
#[test]
fn ssh_2511_destination_is_isolated_after_double_dash() {
    let host = ssh_2511_host("-oProxyCommand=printf PWN", "");

    let ssh_bin = ssh_2511_stub_executable("dd-ssh");
    let sshpass_bin = ssh_2511_stub_executable("dd-sshpass");

    // sshpass branch (password present, both helpers resolvable).
    let (sshpass_cmd, _secret) = build_ssh_command_with(
        &host,
        Some("pw"),
        ssh_2511_resolver(vec![("ssh", ssh_bin.clone()), ("sshpass", sshpass_bin)]),
    )
    .expect("build sshpass ssh command");
    assert_double_dash_isolates_destination(&sshpass_cmd, &host.destination);

    // direct branch (no password).
    let (direct_cmd, _none) =
        build_ssh_command_with(&host, None, ssh_2511_resolver(vec![("ssh", ssh_bin)]))
            .expect("build direct ssh command");
    assert_double_dash_isolates_destination(&direct_cmd, &host.destination);
}

/// The persisted remote cwd is POSIX single-quoted, so `$()`, backticks, `;`
/// and newlines reach `cd` literally and cannot execute.
///
/// Sabotage: restore `cd {:?}` in `build_remote_command` → the double-quoted
/// form runs `$(touch MARK)` → MARK appears → the fixture reddens.
#[test]
fn ssh_2511_remote_cwd_is_posix_quoted_and_inert() {
    let dir = ssh_2511_tmp_dir("cwd-quote");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let marker = dir.join("MARK");
    let hostile = format!("a b'c\"d$(touch {m})`touch {m}`;e\nf", m = marker.display());
    let remote = build_remote_command("export X=1;", &hostile);

    // Run the built command in a controlled /bin/sh. SHELL=/bin/true makes the
    // trailing `exec "$SHELL" -l` harmless; `cd` into the literal path fails,
    // but must fail WITHOUT any substitution having run.
    let _ = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&remote)
        .env("SHELL", "/bin/true")
        .current_dir(&dir)
        .status()
        .expect("run remote command in /bin/sh");

    assert!(
        !marker.exists(),
        "remote cwd metacharacters executed: MARK was created. remote={remote:?}"
    );
    assert!(
        remote.contains(&shell_single_quote(&hostile)),
        "cwd was not POSIX single-quoted: {remote:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
