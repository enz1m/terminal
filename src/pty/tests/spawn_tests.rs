//! Spawn configuration.

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn capability_wrapper_preserves_the_complete_spawn_configuration() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut command = CommandBuilder::new("/bin/bash");
    command.args(["--noprofile", "--norc", "-i"]);
    command.env_clear();
    command.env("GENERATED_ONLY", "fixture-value");
    command.env("GENERATED_BYTES", OsString::from_vec(vec![0xff, b'x']));
    command.cwd("/generated-fixture-home");
    command.umask(Some(0o077));
    command.set_controlling_tty(false);
    let expected = command.clone();

    let mut wrapped = wrap_with_capability(command, std::path::Path::new("/proc/123/fd/42"));
    assert_eq!(wrapped.get_argv().len(), expected.get_argv().len() + 5);
    assert_eq!(wrapped.get_argv()[0], "/bin/sh");
    assert_eq!(wrapped.get_argv()[4], "/proc/123/fd/42");
    assert!(wrapped.get_env("PATH").is_none());
    wrapped.get_argv_mut().drain(..5);
    assert!(
        wrapped == expected,
        "capability wrapping changed environment, cwd, umask or controlling-tty options"
    );
}

#[test]
fn remote_prompt_command_quoting_preserves_embedded_single_quotes() {
    assert_eq!(shell_single_quote("printf 'x'"), "'printf '\\''x'\\'''");
    let assignment = format!("PROMPT_COMMAND={}", shell_single_quote(BASH_PROMPT_COMMAND));
    assert!(assignment.contains("'\\''\\033]133;D;%d"));
}

/// Output that asks the terminal something rings the reply wake, so the drain
/// that sends the answer is not held for the relaxed cap; plain output rings
/// the ordinary one.
///
/// # Sabotage
///
/// Ring `wake.notify()` for every read: the status query leaves no reply
/// wake.
#[test]
fn a_status_query_rings_the_reply_wake() {
    let read = |bytes: &[u8]| {
        let screen = Mutex::new(emulator::Screen::new(Arc::new(AtomicU64::new(0))));
        let wake = WakeBus::default();
        let mut reader = bytes;
        run_reader_loop(&mut reader, &screen, &AtomicU64::new(0), &wake, |_| {});
        wake.reply_pending()
    };
    assert!(read(b"ls\r\n\x1b[5n"));
    assert!(!read(b"ls\r\n"));
}
