//! Terminal failed and scripted pane fixtures.

use crate::{
    model::{self, Pane, SessionStatus, TranscriptLine, TranscriptRow},
    pane_endpoint, scrollback,
};

/// A pane whose shell never started: the reason, plus what to do about it.
///
/// A failed spawn used to leave the optimistic "Starting shell…"
/// transcript in place (or, before the `PtyError` conversion, crash the whole
/// process), so a user saw either a dead window or a pane that looked like it
/// was still loading. Both are silent failures. The reason is rendered as
/// `LineTone::Error` so it reads as a failure and not as shell output, and the
/// retry hint is included because the condition is transient — the honest advice
/// really is "try again".
pub(crate) fn failed_pane(id: &str, reason: &str) -> Pane {
    let lines = vec![
        TranscriptRow::new(TranscriptLine::monochrome(
            format!("{id}-err0"),
            reason,
            model::LineTone::Error,
            false,
        )),
        TranscriptRow::new(TranscriptLine::monochrome(
            format!("{id}-err1"),
            SHELL_SPAWN_RETRY_HINT,
            model::LineTone::Muted,
            false,
        )),
    ];
    Pane {
        endpoint: pane_endpoint::PaneEndpoint::Local,
        id: id.to_owned(),
        title: String::new(),
        subtitle: String::new(),
        status: SessionStatus::Error,
        transcript: lines,
        scroll: scrollback::ScrollPosition::pinned_to_tail(),
        application_cursor_keys: false,
        bracketed_paste: false,
        mouse_protocol: crate::mouse::MouseProtocol::default(),
        alt_screen: false,
        cursor_hidden: false,
        preserved_transcript_rows: 0,
        prompt: String::new(),
        footer: String::new(),
        cwd: String::new(),
    }
}

/// The advice shown under a failed spawn. Transient failures are retryable, so
/// this is actionable rather than decorative.
pub(crate) const SHELL_SPAWN_RETRY_HINT: &str =
    "The system could not start the shell process. Close a tab and try again.";

pub(crate) fn scripted_pane(id: &str, body: &str, cwd: &str) -> Pane {
    let transcript: Vec<TranscriptRow> = body
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let is_prompt = i == body.lines().count().saturating_sub(1);
            TranscriptRow::new(TranscriptLine::monochrome(
                format!("{id}-r{i}"),
                line,
                model::LineTone::Default,
                is_prompt,
            ))
        })
        .collect();
    Pane {
        endpoint: pane_endpoint::PaneEndpoint::Local,
        id: id.to_owned(),
        title: String::new(),
        subtitle: String::new(),
        status: SessionStatus::Idle,
        transcript,
        scroll: scrollback::ScrollPosition::pinned_to_tail(),
        application_cursor_keys: false,
        bracketed_paste: false,
        mouse_protocol: crate::mouse::MouseProtocol::default(),
        alt_screen: false,
        cursor_hidden: false,
        preserved_transcript_rows: 0,
        prompt: String::new(),
        footer: String::new(),
        cwd: cwd.to_owned(),
    }
}
