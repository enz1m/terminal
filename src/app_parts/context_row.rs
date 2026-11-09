//! Terminal context-row apply method.

use super::*;

impl App {
    /// Apply one context-menu row to the session with `id`.
    ///
    /// Every row's effect lives here, in one exhaustive match keyed by the same
    /// [`context_menu::RowKind`] the surface renders — so a row labelled
    /// `Move up` cannot be wired to `Close tab`, and adding a row without an
    /// effect will not compile.
    ///
    /// Each arm re-checks its own availability rather than trusting the row's
    /// `enabled` flag: a session that moved (or vanished) between the frame
    /// that drew the menu and the message being handled must not be acted on
    /// out of position. The flag governs what the user is *offered*; this
    /// governs what actually happens.
    ///
    /// Returns the session-row [`Id`](iced::advanced::widget::Id) focus should
    /// move to — only the `Close` row produces one; every other row
    /// leaves focus where it is and returns `None`.
    pub(crate) fn apply_context_row(
        &mut self,
        id: &str,
        row: context_menu::RowKind,
    ) -> Option<SessionRowFocusTarget> {
        let index = self.sessions.iter().position(|s| s.id == id)?;
        match row {
            context_menu::RowKind::TogglePin => {
                self.sessions[index].pinned = !self.sessions[index].pinned;
                if !self.sessions[index].pinned {
                    let owner = self.sessions[index].id.clone();
                    self.note_pinned_removal(&owner);
                }
                self.invalidate_sidebar();
                None
            }
            // NOT handled here. `Detach tab` opens a real
            // window, and this function can only return a focus id — so the
            // reducer intercepts the row above and routes it through
            // `set_session_detached`. Kept as an explicit arm (rather than a
            // wildcard) so a new row cannot silently fall into detach.
            context_menu::RowKind::ToggleDetach => None,
            context_menu::RowKind::Rename => {
                // Seed the draft with the current title so the inline field
                // opens pre-filled (the user edits rather than retypes) —
                // the same shape the pane rename above uses.
                self.session_rename = Some((
                    self.sessions[index].id.clone(),
                    self.sessions[index].title.clone(),
                ));
                self.invalidate_sidebar();
                None
            }
            context_menu::RowKind::MoveUp => {
                if index > 0 {
                    self.swap_sessions(index, index - 1);
                }
                None
            }
            context_menu::RowKind::MoveDown => {
                if index + 1 < self.sessions.len() {
                    self.swap_sessions(index, index + 1);
                }
                None
            }
            // Group this session's running terminal into the
            // active session's content area on the named edge.
            context_menu::RowKind::GroupInto(edge) => {
                let target = self.sessions.get(self.active).map(|s| s.id.clone());
                if let Some(target) = target {
                    self.apply_group_into(&target, id, edge, None);
                }
                None
            }
            // Give back the most recently merged session.
            context_menu::RowKind::Ungroup => {
                let last = self.sessions[index].group_members.len().checked_sub(1);
                if let Some(member) = last {
                    self.apply_ungroup(index, member);
                }
                None
            }
            context_menu::RowKind::Close => self.close_session_at(index),
        }
    }
}
