//! Spawn configuration.

use super::*;

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
