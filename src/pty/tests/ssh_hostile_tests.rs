//! Hostile SSH cwd.

use super::*;

/// The hostile persisted cwd is inert when routed through the FULL production
/// `build_ssh_command_with` seam (not only `build_remote_command`): the remote
/// command argument that reaches `ssh` single-quotes the path so `$()`,
/// backticks, `;` and newlines cannot execute.
///
/// Sabotage: restore `cd {:?}` in `build_remote_command` → the metacharacters
/// execute in the `/bin/sh` fixture → MARK appears → this test reddens.
#[test]
fn ssh_2511_hostile_cwd_is_inert_through_build_ssh_command() {
    let dir = ssh_2511_tmp_dir("cwd-full");
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("MARK");
    let hostile = format!("a b'c\"d$(touch {m})`touch {m}`;e\nf", m = marker.display());
    let ssh_bin = ssh_2511_stub_executable("cwd-ssh");
    let host = ssh_2511_host("h", &hostile);
    let (cmd, _none) =
        build_ssh_command_with(&host, None, ssh_2511_resolver(vec![("ssh", ssh_bin)]))
            .expect("build direct command");
    let argv = argv_strings(&cmd);
    let remote = argv.last().expect("a remote command argument").clone();

    let _ = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&remote)
        .env("SHELL", "/bin/true")
        .current_dir(&dir)
        .status()
        .expect("run remote command in /bin/sh");
    assert!(
        !marker.exists(),
        "hostile cwd executed through build_ssh_command: MARK created. remote={remote:?}"
    );
    assert!(
        remote.contains(&shell_single_quote(&hostile)),
        "cwd was not POSIX single-quoted in the built ssh command: {remote:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
