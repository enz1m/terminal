//! Production sidebar titlebar composition.
//!
//! Kept out of `main.rs` as one coherent view concern: alignment and window
//! dragging are both properties of the same shell-owned header band.

use super::*;

impl App {
    /// Session commands and Settings in the shell's sidebar titlebar slot.
    /// The shell supplies the collapse toggle and band geometry. There is no
    /// visible title: the list below names itself ("Sessions") to assistive
    /// technology, and the band keeps its room for the commands.
    pub(super) fn sidebar_header(&self) -> Element<'_, Message> {
        enz_title_bar()
            .actions(
                vec![
                    TitlebarActionItem::icon(
                        "session.new",
                        "plus",
                        crate::i18n::label("New local terminal"),
                        Message::NewLocal,
                    ),
                    TitlebarActionItem::icon(
                        "app.ssh",
                        "server",
                        crate::i18n::label(SIDEBAR_SSH_LABEL),
                        Message::OpenSshManager,
                    ),
                    TitlebarActionItem::icon(
                        "app.settings",
                        "settings",
                        crate::i18n::label("Settings"),
                        Message::OpenSettings,
                    ),
                ],
                Message::SidebarTitlebarOverflow(!self.sidebar_titlebar_overflow_open),
            )
            .overflow_open(self.sidebar_titlebar_overflow_open)
            .action_dismiss(Message::SidebarTitlebarOverflow(false))
            .action_menu_id_prefix("terminal-sidebar-actions")
            .into_element()
    }

    /// The session sidebar body (session list).
    ///
    /// The header is now in `sidebar_header()` and fed to the shell's
    /// `sidebar_titlebar` slot; this method provides the scrollable body.
    /// Mark the sidebar cache dirty so the next frame rebuilds it.
    pub(super) fn invalidate_sidebar(&self) {
        self.sidebar_cache.invalidate();
    }

    /// The sidebar wrapped in a retained subtree boundary.
    ///
    /// The closure clones only the sidebar-visible subset of each session
    /// (no transcript data) and builds the row list from owned data, so the
    /// retained `Element<'static>` outlives any single frame.
    pub(super) fn sidebar_body_cached(&self) -> Element<'_, Message> {
        use session_row::SidebarSessionSnapshot;

        // During an active rename the text field borrows the draft string,
        // which prevents the `'static` closure `enz_cached_subtree` requires.
        // Renames are brief and invalidate on every keystroke, so the uncached
        // path loses nothing.
        if self.session_rename.is_some() {
            return self.sidebar_body();
        }

        let snapshots: Vec<SidebarSessionSnapshot> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                SidebarSessionSnapshot::from_session(session)
                    .window_focused(self.session_is_focused(index))
                    .in_own_window(self.in_own_window(index))
                    .branch(self.session_branch(session))
            })
            .collect();
        let active_id: Option<String> = self.sessions.get(self.active).map(|s| s.id.clone());
        let sp = self.enz_theme().spacing;
        let drag = self.drag_channel.clone();

        enz_cached_subtree(&self.sidebar_cache, move || {
            let body = enz_sidebar_content()
                .spacing_scale(sp)
                .section_gap_px(sidebar_session_tree::section_gap_before_tree_caption(&sp))
                .height(Length::Shrink);
            let body = sidebar_session_tree::push_session_groups(
                body,
                &snapshots,
                active_id.clone(),
                None,
                sp,
                &drag,
            );
            enz_scroll_view(body.into_element())
                .height(Length::Fill)
                .width(Length::Fill)
                .into()
        })
        .into()
    }

    pub(super) fn sidebar_body(&self) -> Element<'_, Message> {
        use session_row::SidebarSessionSnapshot;

        let sp = self.enz_theme().spacing;
        let snapshots: Vec<SidebarSessionSnapshot> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                SidebarSessionSnapshot::from_session(session)
                    .window_focused(self.session_is_focused(index))
                    .in_own_window(self.in_own_window(index))
                    .branch(self.session_branch(session))
            })
            .collect();
        let active_id: Option<String> = self.sessions.get(self.active).map(|s| s.id.clone());
        // The rename target lives on `&self` as `(String, String)`; the tree
        // owns the draft by clone. That is uncached-path-only — the cached
        // path is short-circuited during rename — so the extra `String::clone`
        // is one per keystroke, not per frame.
        let rename: Option<(String, String)> = self.session_rename.clone();

        let body = enz_sidebar_content()
            .spacing_scale(sp)
            .section_gap_px(sidebar_session_tree::section_gap_before_tree_caption(&sp))
            .height(Length::Shrink);
        let body = sidebar_session_tree::push_session_groups(
            body,
            &snapshots,
            active_id,
            rename,
            sp,
            &self.drag_channel,
        );
        enz_scroll_view(body.into_element())
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    }

    /// The git branch shown under a session row: its active pane's, for a
    /// session that is not a group (a group's second line is its topology).
    fn session_branch(&self, session: &model::Session) -> Option<String> {
        if !session.group_members.is_empty() {
            return None;
        }
        let pane = session.panes.get(session.active_pane)?;
        self.git_branches.get(&pane.id).cloned()
    }
}
