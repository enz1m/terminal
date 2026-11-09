//! Terminal PTY byte-encoding methods.

use super::*;

impl App {
    /// The bytes (if any) that a key press should send to the active PTY.
    /// Returns `None` when a focused widget already `captured` the event — the
    /// single guard that stops find-bar / modal typing from echoing into the
    /// shell. `update` calls exactly this, so a test on it is a test of the
    /// real routing decision, not a parallel copy.
    ///
    /// A cursor key (Up/Down/Left/Right) has TWO encodings and the
    /// choice is DECCKM, tracked per pane as `application_cursor_keys`. In
    /// application-cursor-keys mode (`\x1b[?1h`, terminfo `smkx`) — which
    /// `less`, `vim` and `htop` set — the down key is `ESC O B`, and a program
    /// in that mode has its arrow terminfo bound to the `ESC O` form only, so
    /// the plain `ESC [ B` this used to send unconditionally is an unknown
    /// sequence it silently drops. That is why arrows moved shell history
    /// (normal mode, `ESC [ A`) but did nothing in `less` (application mode)
    /// on the live binary. Routing cursor keys through `cursor_key_bytes`,
    /// which reads the active pane's flag, is the fix; `cursor_key_bytes`
    /// returns `None` for every non-cursor key so all other keys keep the
    /// plain `key_to_pty_bytes` path unchanged.
    pub(crate) fn pty_bytes_for_key(
        &self,
        captured: bool,
        key: &keyboard::Key,
        mods: Modifiers,
    ) -> Option<Vec<u8>> {
        self.pty_bytes_for_application_mode(
            captured,
            key,
            mods,
            self.active_application_cursor_keys(),
        )
    }

    /// The same key encoder for an explicitly targeted pane. Detached-window
    /// input cannot consult `active_pane()`: that names the docked shell and
    /// can change between event publication and reduction.
    pub(crate) fn pty_bytes_for_application_mode(
        &self,
        captured: bool,
        key: &keyboard::Key,
        mods: Modifiers,
        application_cursor_keys: bool,
    ) -> Option<Vec<u8>> {
        if captured {
            return None;
        }
        keymap::key_to_pty_bytes_with_mode(key, mods, application_cursor_keys)
    }
}
