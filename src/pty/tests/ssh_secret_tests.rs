//! SSH password channels.

use super::*;

/// The SSH password appears only in the anonymous `memfd` — never in argv
/// (program or args) or the environment — and the `-f` argument names the
/// non-secret `/proc/<pid>/fd/<n>` magic path, which reads back exactly the
/// password line.
///
/// Sabotage: restore `c.arg("-p"); c.arg(password)` (and drop the `-f`/memfd)
/// → the sentinel appears in argv → the scan reddens.
#[test]
#[cfg(target_os = "linux")]
fn ssh_2511_password_lives_only_in_memfd() {
    let sentinel = "S3NT-INEL-pw";
    let host = ssh_2511_host("example.net", "");
    let ssh_bin = ssh_2511_stub_executable("pw-ssh");
    let sshpass_bin = ssh_2511_stub_executable("pw-sshpass");
    let (cmd, secret) = build_ssh_command_with(
        &host,
        Some(sentinel),
        ssh_2511_resolver(vec![("ssh", ssh_bin), ("sshpass", sshpass_bin)]),
    )
    .expect("build sshpass command");
    let secret = secret.expect("the sshpass branch must produce a password memfd");

    for arg in cmd.get_argv() {
        assert!(
            !arg.to_string_lossy().contains(sentinel),
            "password leaked into argv item {arg:?}"
        );
    }
    for (k, v) in cmd.iter_full_env_as_str() {
        assert!(
            !k.contains(sentinel) && !v.contains(sentinel),
            "password leaked into environment {k}={v}"
        );
    }

    let argv = argv_strings(&cmd);
    assert!(
        !argv.iter().any(|a| a == "-p"),
        "sshpass `-p` (argv password) must be gone; argv={argv:?}"
    );
    let fpos = argv
        .iter()
        .position(|a| a == "-f")
        .expect("sshpass must deliver the password via `-f`");
    let proc_path = secret.proc_path().to_string_lossy().into_owned();
    assert_eq!(
        argv.get(fpos + 1).map(String::as_str),
        Some(proc_path.as_str()),
        "`-f` must name the memfd's /proc/<pid>/fd/<n> path"
    );
    // The `-f` argument is a non-secret magic path, not the secret itself.
    assert!(
        proc_path.starts_with(&format!("/proc/{}/fd/", std::process::id())),
        "the -f path must name THIS process's own descriptor: {proc_path:?}"
    );
    assert!(
        !proc_path.contains(sentinel),
        "the -f path must contain no secret bytes: {proc_path:?}"
    );

    // The bytes read back through the /proc path are exactly the password line.
    let mut reader = std::io::BufReader::new(
        std::fs::File::open(secret.proc_path()).expect("open the memfd via /proc"),
    );
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line).expect("read memfd payload");
    assert_eq!(
        line,
        format!("{sentinel}\n"),
        "the memfd payload must be the password followed by a newline"
    );
}

/// `resolve_executable` skips a non-executable first PATH candidate and
/// resolves the executable regular file to its absolute path.
///
/// Sabotage: revert `resolve_executable` to `is_file()` → it accepts the
/// non-executable dir1 candidate → the equality reddens.
#[test]
fn ssh_2511_resolve_skips_non_executable_candidate() {
    use std::os::unix::fs::PermissionsExt;
    let base = ssh_2511_tmp_dir("exec-id");
    let dir1 = base.join("dir1");
    let dir2 = base.join("dir2");
    std::fs::create_dir_all(&dir1).unwrap();
    std::fs::create_dir_all(&dir2).unwrap();
    let non_exec = dir1.join("sshpass");
    std::fs::write(&non_exec, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&non_exec, std::fs::Permissions::from_mode(0o644)).unwrap();
    let exec = dir2.join("sshpass");
    enzui_test_support::fake_executable::write_executable(&exec, "#!/bin/sh\nexit 0\n");

    let path = std::env::join_paths([&dir1, &dir2]).unwrap();
    let resolved = resolve_executable_in("sshpass", &path);
    assert_eq!(
        resolved.as_deref(),
        Some(exec.as_path()),
        "must skip the non-executable dir1 candidate and resolve the executable dir2 one"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// BOTH launch branches use the once-resolved absolute path, so a PATH
/// poisoned after resolution cannot re-resolve the launched program:
/// - direct branch: `argv[0]` is the absolute `ssh`, never a basename;
/// - sshpass branch: `argv[0]` is the absolute `sshpass`, and the command
///   argument handed to sshpass is the absolute `ssh` (not the basename
///   "ssh" that sshpass would otherwise `execvp` through PATH).
///
/// Sabotage: restore `c.arg("ssh")` in the sshpass branch → the basename
/// assertion reddens; restore the direct branch's basename fallback and pass
/// an unresolved name → argv[0] is a basename → the direct assertion reddens.
#[test]
fn ssh_2511_both_branches_launch_absolute_paths_no_reresolve() {
    let ssh_bin = ssh_2511_stub_executable("id-ssh");
    let sshpass_bin = ssh_2511_stub_executable("id-sshpass");
    let host = ssh_2511_host("h", "");
    let ssh_str = ssh_bin.to_string_lossy().into_owned();
    let sshpass_str = sshpass_bin.to_string_lossy().into_owned();

    // Direct branch.
    let (direct, _none) = build_ssh_command_with(
        &host,
        None,
        ssh_2511_resolver(vec![("ssh", ssh_bin.clone())]),
    )
    .expect("build direct command");
    let dargv = argv_strings(&direct);
    assert_eq!(
        dargv.first().map(String::as_str),
        Some(ssh_str.as_str()),
        "direct argv[0] must be the resolved absolute ssh; argv={dargv:?}"
    );
    assert!(
        std::path::Path::new(&dargv[0]).is_absolute(),
        "direct program must be an absolute path, not a PATH-re-resolvable basename"
    );

    // sshpass branch.
    let (via_pass, _secret) = build_ssh_command_with(
        &host,
        Some("pw"),
        ssh_2511_resolver(vec![("ssh", ssh_bin), ("sshpass", sshpass_bin)]),
    )
    .expect("build sshpass command");
    let pargv = argv_strings(&via_pass);
    assert_eq!(
        pargv.first().map(String::as_str),
        Some(sshpass_str.as_str()),
        "sshpass argv[0] must be the resolved absolute sshpass; argv={pargv:?}"
    );
    assert!(
        pargv.iter().any(|a| a == &ssh_str),
        "sshpass must receive the absolute ssh path as its command argument; argv={pargv:?}"
    );
    assert!(
        !pargv.iter().any(|a| a == "ssh"),
        "no bare `ssh` basename may reach sshpass; argv={pargv:?}"
    );
}
