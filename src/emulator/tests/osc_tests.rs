//! OSC sequences.

use super::*;

#[test]
fn osc_seven_and_prompt_completion_publish_non_grid_effects() {
    let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
    screen.process(b"before\x1b]7;file://localhost/tmp/randomfolder\x07after");
    assert_eq!(
        screen.take_effects().cwd.as_deref(),
        Some("file://localhost/tmp/randomfolder")
    );
    screen.process(b"\x1b]133;D;17\x07");
    assert_eq!(screen.take_effects().command_finished, Some(17));
    let joined = screen
        .project("osc-effects")
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        joined.contains("beforeafter"),
        "grid was corrupted: {joined}"
    );
    assert!(!joined.contains("randomfolder") && !joined.contains("133;D"));
}

/// An OSC that never terminates stays bounded across chunk boundaries: the
/// parser holding its partial payload is replaced, the remainder of that
/// sequence never reaches the grid or the effects, the pane says so once, and
/// the next sequence parses normally.
///
/// # Sabotage
///
/// Make `StringSequenceGuard::split` always return one `Segment::Feed` over the
/// whole chunk: the discard count, the notice and the unpublished title all
/// redden while the parser keeps buffering the payload.
#[test]
fn an_unterminated_osc_is_bounded_across_chunks_and_the_pane_recovers() {
    let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
    screen.process(b"visible\r\n\x1b]0;");
    let chunk = vec![b'A'; 8192];
    let mut fed = 0usize;
    while fed <= escape_policy::MAX_STRING_SEQUENCE_BYTES {
        screen.process(&chunk);
        fed += chunk.len();
    }

    let effects = screen.take_effects();
    assert_eq!(effects.discarded_sequences, 1);
    assert!(effects.title.is_none());

    // The tail of the abandoned sequence, terminator included, is still not
    // input: it must not reappear as printed text or as a second report.
    screen.process(b"AAAA\x07");
    assert_eq!(screen.take_effects().discarded_sequences, 0);

    screen.process(b"\x1b]2;recovered\x07after");
    assert_eq!(screen.take_effects().title.as_deref(), Some("recovered"));

    let joined = screen
        .project("overflow")
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(joined.contains("visible"), "grid was lost: {joined}");
    assert!(
        joined.contains("[oversized control sequence discarded]"),
        "the refusal must be visible in the pane: {joined}"
    );
    assert!(joined.contains("after"), "the reader wedged: {joined}");
    assert!(
        !joined.contains("AAAA"),
        "payload reached the grid: {joined}"
    );
}

/// A status report is answered "ready" and a position report with the
/// cursor, in order, so a program that writes output and then asks can tell
/// when the terminal has parsed it (kitty's benchmark ends this way).
///
/// # Sabotage
///
/// Drop the `ESC [ 0 n` arm of `admitted_reply`: the replies lose `0n`.
#[test]
fn status_and_position_reports_are_answered_in_order() {
    let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
    screen.process(b"abc\x1b[6n\x1b[5n\x1b[5n");
    let replies: Vec<Vec<u8>> = screen.take_terminal_replies().into();
    assert_eq!(
        replies,
        [
            b"\x1b[1;4R".to_vec(),
            b"\x1b[0n".to_vec(),
            b"\x1b[0n".to_vec()
        ]
    );
}

/// OSC 52 may neither write the desktop clipboard nor read it back over the
/// PTY, and OSC 9 raises no desktop notification. Every payload is dropped at
/// the parser, so none of it reaches effects, replies or the grid.
///
/// # Sabotage
///
/// Replace the refusal arm's body with `self.queue_reply(b"\x1b]52;c;\x07".to_vec())`:
/// the empty-replies assertion reddens.
#[test]
fn osc_52_and_osc_9_are_refused_without_effect_reply_or_grid_text() {
    let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
    screen.process(
        b"before\x1b]52;c;SU5KRUNURUQ=\x07\x1b]52;c;?\x07\x1b]52;p;\x07\x1b]9;notify\x07after",
    );

    assert_eq!(screen.take_effects(), TerminalEffects::default());
    assert!(
        screen.take_terminal_replies().is_empty(),
        "a selection query must not answer on the PTY"
    );

    let joined = screen
        .project("osc-52")
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        joined.contains("beforeafter"),
        "grid was corrupted: {joined}"
    );
    assert!(!joined.contains("INJECTED") && !joined.contains("notify"));
}
