//! Terminal shell-status words and paste encoding.

/// The one place the "the shell would not start" wording lives.
///
/// A single constant rather than three format strings so the sidebar row, the
/// content pane and the tests cannot drift apart — and so a test asserting the
/// rendered text is not restating a literal typed twice.
pub(crate) const SHELL_SPAWN_FAILED_PREFIX: &str = "Shell failed to start: ";

/// The one place the "your shell exited" wording lives.
///
/// A single constant so the sidebar row, the content pane and the tests cannot
/// drift apart.
pub(crate) const SHELL_EXITED_PREFIX: &str = "Shell exited: ";

/// Whether the app's own key handler claims this Enter while the find bar is
/// open.
///
/// Extracted so the polarity can be measured without synthesising a whole
/// window event: `App::update`'s find branch calls this, so a test and
/// production cannot disagree about which Enters are claimed.
pub(crate) fn find_enter_is_handled_globally(
    key: &iced::keyboard::Key,
    mods: iced::keyboard::Modifiers,
) -> bool {
    matches!(
        key,
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter)
    ) && mods.shift()
}

/// Frame an already-normalised paste payload for the PTY.
///
/// `text` must come from `escape_policy::normalized_paste`, which is what
/// makes its line endings unambiguous and guarantees it cannot carry its own
/// bracketed-paste terminator. This function only adds framing.
///
/// When the pane's shell has DECSET 2004 (bracketed paste) engaged, the payload
/// is wrapped in `ESC [ 200 ~` … `ESC [ 201 ~` so the shell (readline, zsh's
/// ZLE, vim, a REPL) can tell pasted bytes from typed ones — the guard is what
/// stops a multi-line paste from auto-executing every line as its newline
/// lands, and what lets an editor insert a paste verbatim instead of running
/// its keybindings. Without the mode, the text is sent raw, exactly as if typed.
pub(crate) fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut bytes = Vec::with_capacity(
            text.len() + BRACKETED_PASTE_START.len() + BRACKETED_PASTE_END.len(),
        );
        bytes.extend_from_slice(BRACKETED_PASTE_START);
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(BRACKETED_PASTE_END);
        bytes
    } else {
        text.as_bytes().to_vec()
    }
}

/// The bracketed-paste guard sequences (DECSET 2004). `ESC [ 200 ~` opens a
/// paste, `ESC [ 201 ~` closes it.
pub(crate) const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
pub(crate) const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

/// The bytes that put a history entry on the shell's input line, unexecuted.
///
/// Empty when the entry normalises to nothing.
///
/// # Errors
///
/// Refuses an oversized entry, and a multi-line one unless bracketed paste is
/// engaged, because each raw newline would run the line before it.
pub(crate) fn history_paste_bytes(
    command: &str,
    bracketed: bool,
) -> Result<Vec<u8>, crate::escape_policy::PasteRefusal> {
    let payload = crate::escape_policy::normalized_paste(command)?;
    if payload.is_empty() {
        return Ok(Vec::new());
    }
    if payload.contains('\n') && !bracketed {
        return Err(crate::escape_policy::PasteRefusal::MultiLineCommand);
    }
    Ok(paste_bytes(&payload, bracketed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_line_history_entries_never_reach_an_unbracketed_shell() {
        use crate::escape_policy::PasteRefusal;
        for command in ["true\nrm -rf ~", "true\rrm -rf ~", "true\r\nrm -rf ~"] {
            assert_eq!(
                history_paste_bytes(command, false),
                Err(PasteRefusal::MultiLineCommand)
            );
            let bracketed = history_paste_bytes(command, true).unwrap();
            assert!(bracketed.starts_with(BRACKETED_PASTE_START));
            assert!(bracketed.ends_with(BRACKETED_PASTE_END));
        }
        assert_eq!(history_paste_bytes("ls -l", false).unwrap(), b"ls -l");
        assert!(history_paste_bytes("\u{7}", false).unwrap().is_empty());
    }
}
