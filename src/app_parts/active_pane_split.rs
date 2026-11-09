//! Terminal pane-split methods.

use super::*;

impl App {
    pub(crate) fn split_active_pane(
        &mut self,
        direction: pane_layout::SplitDirection,
    ) -> Task<Message> {
        if self
            .active_pane_id()
            .is_some_and(|id| self.restored_outputs.contains_key(&id))
        {
            self.pane_feedback =
                Some("Use Reconnect or New shell before splitting this restored pane.".into());
            return Task::none();
        }
        if self
            .active_pane()
            .is_some_and(|pane| pane.endpoint != pane_endpoint::PaneEndpoint::Local)
        {
            return self.split_endpoint_pane(direction);
        }
        #[cfg(test)]
        if let Some(mut spawn) = self.endpoint_spawn_override.take() {
            self.split_active_pane_with(direction, |id, cwd| {
                spawn(
                    &id,
                    &pane_endpoint::PaneEndpoint::Local,
                    cwd.unwrap_or(""),
                    None,
                )
                .map(|pty| {
                    (
                        pty,
                        pty::LocalCwdResolution {
                            requested: cwd.map(std::path::PathBuf::from),
                            effective: std::path::PathBuf::from(cwd.unwrap_or("/")),
                            warning: None,
                        },
                    )
                })
            });
            self.endpoint_spawn_override = Some(spawn);
            return Task::none();
        }
        let cap = self.settings.clamped_scrollback() as usize;
        let Some((new_id, ticket, requested_cwd)) = self.begin_split(direction) else {
            return Task::none();
        };
        let fallback = session_spawn::placeholder_cwd(requested_cwd.as_deref());
        let worker_id = new_id.clone();
        self.spawn_pane_in_background(
            ticket,
            new_id,
            move || {
                session_spawn::PaneSpawnOutcome::Split(session_spawn::run_local_spawn(
                    worker_id,
                    false,
                    requested_cwd,
                    |id, cwd| pty::PtySession::spawn_shell_at_with_scrollback(id, cwd, cap),
                ))
            },
            move |error| {
                session_spawn::PaneSpawnOutcome::Split(session_spawn::LocalSpawnOutcome {
                    fallback_cwd: fallback.clone(),
                    result: Err(error),
                })
            },
        )
    }

    /// `split_active_pane` with the shell spawn injected and run inline. It
    /// shares every step but the worker with the production split.
    #[cfg(test)]
    pub(crate) fn split_active_pane_with<F>(
        &mut self,
        direction: pane_layout::SplitDirection,
        spawn: F,
    ) where
        F: FnOnce(String, Option<&str>) -> session_spawn::LocalSpawnResult,
    {
        let Some((new_id, ticket, requested_cwd)) = self.begin_split(direction) else {
            return;
        };
        let outcome = session_spawn::run_local_spawn(new_id.clone(), false, requested_cwd, spawn);
        self.finish_pane_spawn(
            ticket,
            &new_id,
            session_spawn::PaneSpawnOutcome::Split(outcome),
        );
    }

    /// Split the active pane and publish the new leaf waiting for its shell.
    /// Nothing here touches the filesystem or a process.
    ///
    /// Returns the new pane id, the ticket its spawn completion must present,
    /// and the directory request the spawn resolves; `None` when no split was
    /// made (the refusal is already in `pane_feedback`).
    fn begin_split(
        &mut self,
        direction: pane_layout::SplitDirection,
    ) -> Option<(String, u64, Option<String>)> {
        if self
            .active_pane()
            .is_some_and(|pane| pane.endpoint != pane_endpoint::PaneEndpoint::Local)
        {
            self.pane_feedback =
                Some("This pane requires a same-endpoint SSH split, not a local shell.".into());
            return None;
        }
        if self
            .sessions
            .get(self.active)
            .is_some_and(|session| session.panes.len() >= pane_layout::MAX_PANES)
        {
            self.pane_feedback = Some(crate::i18n::format(
                crate::i18n::localizer(),
                "Pane limit reached: at most {} terminal surfaces can be rendered.",
                &[("arg0", &(pane_layout::MAX_PANES))],
            ));
            return None;
        }
        self.pane_feedback = None;
        let requested_cwd = self.active_local_cwd();
        // The resolved label replaces this once the spawn has stat'ed it.
        let initial_cwd = session_spawn::placeholder_cwd(requested_cwd.as_deref());
        let (current_id, session_id) = self.sessions.get(self.active).and_then(|s| {
            s.panes
                .get(s.active_pane)
                .map(|p| (p.id.clone(), s.id.clone()))
        })?;
        let new_id = self.fresh_pane_id();
        let s = self.sessions.get_mut(self.active)?;
        if !s.layout.split_leaf(&current_id, direction, new_id.clone()) {
            return None;
        }
        for member in &mut s.group_members {
            member.split_owned_pane(&current_id, direction, &new_id);
        }
        let pane = scripted_pane(&new_id, "Starting shell…", &initial_cwd);
        s.panes.push(pane);
        s.active_pane = s.panes.len() - 1;
        s.pane_grid.replace_tree(s.layout.pane_grid_tree());
        let ticket = self.expect_pane_spawn(&new_id, &session_id);
        // The new leaf takes width from its siblings, re-wrapping them; drop
        // stale ranges before the reflow.
        self.clear_selection_on_reflow();
        self.resize_active_pty();
        Some((new_id, ticket, requested_cwd))
    }
}
