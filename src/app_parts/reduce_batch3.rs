//! Terminal reducer batch 3: key arms.

use super::*;

impl App {
    pub(crate) fn reduce3_key_pressed_0(
        &mut self,
        key: keyboard::Key,
        mods: Modifiers,
        captured: bool,
    ) -> Task<Message> {
        self.mods = mods;
        // `enz_point_context_menu_host` claims Escape only WHILE
        // PRESENTING and reports it as `Dismissed(CloseCause::Escape)`
        // — the same funnel an outside press arrives through, which is
        // what stops a pointer dismissal and a keyboard dismissal from
        // drifting apart. The menu is the top-most surface, so it sees
        // the key first structurally rather than by arm order, and the
        // rename handlers below keep their Escape untouched whenever no
        // menu is up.
        //
        // Gated by `escape_dismisses_the_open_context_menu`, which now
        // presses the real key through the rendered frame, and by
        // `escape_still_cancels_a_rename_when_no_menu_is_open`.
        //
        // Titlebar overflow Escape is deliberately absent from
        // this reducer. The canonical action slot owns the scoped key
        // capture and emits its toggle only while the menu is open.
        // A session rename in progress owns Escape next (abandon the
        // rename), on the same rule as the pane rename below.
        if self.session_rename.is_some()
            && let keyboard::Key::Named(keyboard::key::Named::Escape) = &key
        {
            return self.update(Message::CancelSessionRename);
        }
        // A pane rename in progress owns Escape (abandon the rename)
        // before any other handler sees it.
        if self.pane_rename_draft.is_some()
            && let keyboard::Key::Named(keyboard::key::Named::Escape) = &key
        {
            return self.update(Message::CancelPaneRename);
        }
        // App-level shortcuts dispatch through the KEYBINDING
        // CATALOG (only when no modal owns the keyboard).
        //
        // This block used to match four hard-coded `Ctrl+<letter>`
        // chords (`b`, `,`, `f`, `h`) while `keybindings.rs` carried a
        // 32-action catalog, a `resolved_bindings` override merge and
        // an `action_for_chord` dispatcher that NOTHING called. The
        // Shortcuts dialog (`enz_shortcuts_dialog`) rendered all 32
        // from that same catalog, so the app advertised 32 shortcuts
        // and honoured 4 - and a user's override of any of the four it
        // did honour was ignored, because the match was on the literal
        // letter rather than on the resolved chord.
        //
        // `clipboard.paste` is honoured through the `RequestPaste`
        // interception below (an async clipboard read, not a reducer).
        // `clipboard.copy` is honoured through the `RequestCopy`
        // interception: it copies the current selection to the
        // system clipboard via an async write. Both have real
        // `action_message` interceptions now, so neither falls through
        // to the PTY.
        if self.input_context.is_none()
            && !self.settings_open
            && !self.ssh_manager_open
            && let Some(action) = self.shortcut_action_for_key(&key, mods)
        {
            // `clipboard.paste` cannot go through `action_message`: it
            // does not resolve to a plain reducer message but to an
            // async clipboard read (a `Task`). Route it to
            // `RequestPaste`, which owns that read. `clipboard.copy`
            // is the same shape: an async clipboard write.
            if action == "clipboard.paste" {
                return self.update(Message::RequestPaste);
            }
            if action == "clipboard.copy" {
                return self.update(Message::RequestCopy);
            }
            if let Some(message) = Self::action_message(&action) {
                return self.update(message);
            }
        }
        // Find bar key handling: Escape closes, and Shift+Enter goes
        // to the PREVIOUS match — the binding the "Previous
        // (Shift+Enter)" tooltip promises.
        //
        // # Plain Enter is not handled here
        //
        // The review: *"When I search for a text and click enter the
        // page reloads."* It was not reloading; it was advancing
        // **twice** per keypress. The titlebar search field carries
        // `.on_submit(Message::FindNext)` (`find_affordance`), so a
        // focused field already turns one Enter into one
        // `next_match()`. This branch then ran `next_match()` a second
        // time on the very same key event, so a single Enter skipped a
        // match and jumped the viewport two hits down the transcript —
        // which, on a screenful of scrollback, reads exactly like the
        // view being thrown away and rebuilt.
        //
        // The submit callback is the correct single authority: it
        // fires only when the field actually has focus. This branch
        // could not tell focused-field Enter from terminal Enter, so
        // besides double-advancing it also swallowed the Enter of a
        // user running a command while the find bar happened to be
        // open. Both go away by not handling plain Enter here.
        //
        // Shift+Enter stays, because a text field's submit carries no
        // modifiers and there is no other carrier for "previous".
        if self.find.open {
            match &key {
                keyboard::Key::Named(keyboard::key::Named::Escape) => {
                    return self.update(Message::CloseFind);
                }
                keyboard::Key::Named(keyboard::key::Named::Enter)
                    if find_enter_is_handled_globally(&key, mods) =>
                {
                    return self.update(Message::FindPrev);
                }
                _ => {}
            }
        }
        // History overlay: Escape closes.
        if self.history_open
            && let keyboard::Key::Named(keyboard::key::Named::Escape) = key
        {
            self.history_open = false;
            return Task::none();
        }
        if self.settings_open {
            if let keyboard::Key::Named(keyboard::key::Named::Escape) = key {
                return self.update(Message::SettingsModalEvent(ModalHostEvent::Dismissed(
                    enzui_core::modal::DismissCause::Escape,
                )));
            }
            return Task::none();
        }
        // A focused dialog button may leave a bare key uncaptured.
        if self.ssh_manager_open {
            return Task::none();
        }
        // PageUp/PageDown page the SCROLLBACK on the primary
        // screen, and are left to the PTY on the alternate one.
        //
        // The split is the same rule the wheel uses: `less`, `vim` and
        // `btop` run on the alternate screen and page themselves, and
        // the primary screen has a transcript to page through. Routing
        // both to one place is what breaks either a TUI or scrollback.
        if !captured
            && !mods.control()
            && !mods.alt()
            && !mods.logo()
            && !self.active_pane_is_alt_screen()
            && let keyboard::Key::Named(named) = &key
            && matches!(
                named,
                keyboard::key::Named::PageUp | keyboard::key::Named::PageDown
            )
        {
            let page = self.active_scroll_geometry().page_rows() as i64;
            let delta = if matches!(named, keyboard::key::Named::PageUp) {
                -page
            } else {
                page
            };
            return self.update(Message::ScrollTranscriptRows(delta));
        }
        // Route to the active PTY — but only if no focused widget
        // already consumed the key. Without this guard, typing into the
        // find-bar search field (or any modal text field) also echoes
        // into the shell. `captured_key_is_not_routed_to_pty` guards it.
        if let Some(bytes) = self.pty_bytes_for_key(captured, &key, mods) {
            self.type_active_pty(&bytes);
        }

        Task::none()
    }
}
