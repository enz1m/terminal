use crate::{App, Message, model, session_store};
use iced::Element;
use std::sync::Arc;

impl App {
    pub(crate) fn sync_pinned_capture(&mut self) {
        for session in &self.sessions {
            self.durable_titles
                .entry(session.id.clone())
                .or_insert_with(|| session.title.clone());
            for pane in &session.panes {
                if let Some(pty) = self.ptys.get(&pane.id) {
                    pty.set_pinned_capture(session.pinned);
                }
            }
        }
        self.restored_outputs.retain(|id, _| {
            self.sessions
                .iter()
                .any(|session| session.panes.iter().any(|pane| pane.id == *id))
        });
        let mut identities: std::collections::HashSet<_> = self
            .sessions
            .iter()
            .map(|session| session.id.clone())
            .collect();
        for session in &self.sessions {
            collect_member_titles(
                &session.group_members,
                &mut self.durable_titles,
                &mut identities,
            );
        }
        self.durable_titles.retain(|id, _| identities.contains(id));
        self.restored_grid_sizes
            .retain(|id, _| self.restored_outputs.contains_key(id));
        self.finished_outputs.retain(|id, _| {
            self.sessions
                .iter()
                .any(|session| session.panes.iter().any(|pane| pane.id == *id))
        });
        let cap = self.settings.clamped_scrollback() as usize;
        for output in self.finished_outputs.values_mut() {
            let bounded = output.bounded(cap);
            if **output != bounded {
                *output = Arc::new(bounded);
            }
        }
        let changed: Vec<_> = self
            .restored_outputs
            .iter()
            .filter_map(|(id, output)| {
                let bounded = output.bounded(cap);
                (bounded != **output).then(|| (id.clone(), Arc::new(bounded)))
            })
            .collect();
        for (id, output) in changed {
            self.invalidate_pane_coordinates(&id);
            if let Some(pane) = self
                .sessions
                .iter_mut()
                .flat_map(|session| &mut session.panes)
                .find(|pane| pane.id == id)
            {
                pane.transcript = output.render(&id);
            }
            self.restored_outputs.insert(id, output);
        }
    }

    pub(crate) fn pinned_snapshot(&self) -> session_store::SessionState {
        let mut state = session_store::SessionState::snapshot(
            &self.sessions,
            self.active,
            &self.title_overrides,
        );
        state.pending_deletes = self.secure_pins.pending_deletes.clone();
        for session in &mut state.sessions {
            session.archive = self.secure_pins.references.get(&session.id).cloned();
            if !session.title_overridden
                && let Some(title) = self.durable_titles.get(&session.id)
            {
                session.title = title.clone();
            }
            sanitize_members(
                &mut session.group_members,
                &self.durable_titles,
                &self.title_overrides,
            );
            for pane in &mut session.panes {
                pane.cwd.clear();
            }
        }
        state
    }

    pub(crate) fn exclude_pinned_input(&self, pane: &str) {
        if let Some(pty) = self.ptys.get(pane) {
            pty.exclude_pinned_input();
        }
    }

    pub(crate) fn record_restored_grid_size(&mut self, pane: &str, size: iced::Size) {
        if self.restored_outputs.contains_key(pane)
            && !self.ptys.contains_key(pane)
            && Self::usable_pty_extent(size)
            && self.restored_grid_sizes.get(pane) != Some(&size)
        {
            self.invalidate_pane_coordinates(pane);
            self.restored_grid_sizes.insert(pane.to_owned(), size);
        }
    }

    pub(crate) fn restored_surface_size(&self, pane: &str) -> Option<iced::Size> {
        (self.restored_outputs.contains_key(pane) && !self.ptys.contains_key(pane))
            .then(|| self.restored_grid_sizes.get(pane).copied())
            .flatten()
    }

    /// A pane restored from a pin, measured so its stored rows fit the grid
    /// it is shown in; its notice is in `window_notices`.
    pub(crate) fn measured_restored_pane<'a>(
        &self,
        pane: &model::Pane,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        if self.restored_outputs.contains_key(&pane.id) {
            return iced::widget::Sensor::new(content)
                .on_resize(Message::RestoredGridResized)
                .into();
        }
        content
    }

    pub(crate) fn empty_terminal(&self) -> Element<'_, Message> {
        if self.startup.is_some() {
            return self.startup_notice();
        }
        // Load and storage failures are notices (`window_notices`).
        enzui_widgets::enz_status_page(crate::i18n::label("No live terminal"))
            .icon("terminal")
            .primary_action(
                enzui_core::Action::new("session.new", Message::NewLocal)
                    .with_label(crate::i18n::label("New shell")),
            )
            .into_element()
    }

    /// Reviews one cooperative close request for `window`.
    ///
    /// Nothing is destroyed here. A request that would end the process starts
    /// the secure close over every window that is open right now, so the whole
    /// app stays on screen while its durable state settles; a dependent window
    /// keeps the existing per-window policy and is closed only after this
    /// reducer has recorded that its destruction is the app's own.
    pub(crate) fn request_window_close(&mut self, window: iced::window::Id) -> iced::Task<Message> {
        let request = enzui_core::close_guard::CloseRequest::for_window(window);
        let window = request.window();
        if !self.windows.is_open(window) {
            // A window the ledger never saw carries no session or durable
            // state; honouring it directly cannot trap it open.
            return self.close_dependent_window(window);
        }
        if self.close_terminates(window) {
            self.drain_ptys();
            let windows: Vec<_> = self.windows.open_windows().collect();
            return self.start_close(window, windows, true);
        }
        self.close_dependent_window(window)
    }

    // The ledger's own rule, read without mutating it before the close is
    // committed: the last window, or the primary while its policy closes
    // everything.
    fn close_terminates(&self, window: iced::window::Id) -> bool {
        use enzui_core::window_lifecycle::PrimaryClosePolicy;

        self.windows.open_count() <= 1
            || (self.windows.primary() == Some(window)
                && self.windows.policy() == PrimaryClosePolicy::CloseEverything)
    }

    fn close_dependent_window(&mut self, window: iced::window::Id) -> iced::Task<Message> {
        self.intentional_closes
            .insert(window, crate::CloseIntent::Dependent);
        self.retire_window_input(window);
        iced::window::close(window)
    }

    /// Publishes what a destroyed window's process still owes.
    ///
    /// The surface is already gone, so this close cannot be held for a user
    /// decision.
    pub(crate) fn exit_after_pinned_save(
        &mut self,
        destroyed: iced::window::Id,
        others: Vec<iced::window::Id>,
    ) -> iced::Task<Message> {
        self.drain_ptys();
        self.start_close(destroyed, others, false)
    }

    pub(crate) fn restore_output_history(&self, panes: &[model::Pane]) -> model::CommandHistory {
        model::CommandHistory::from_items(
            panes
                .iter()
                .filter_map(|pane| self.restored_outputs.get(&pane.id))
                .flat_map(|output| output.history()),
        )
    }
}

fn collect_member_titles(
    members: &[crate::pane_group::GroupMember],
    titles: &mut std::collections::HashMap<String, String>,
    identities: &mut std::collections::HashSet<String>,
) {
    for member in members {
        identities.insert(member.session_id.clone());
        titles
            .entry(member.session_id.clone())
            .or_insert_with(|| member.title.clone());
        collect_member_titles(&member.members, titles, identities);
    }
}

pub(crate) fn restore_member_histories(
    members: &mut [crate::pane_group::GroupMember],
    outputs: &std::collections::HashMap<String, Arc<crate::pinned_output::PinnedOutput>>,
) {
    for member in members {
        member.status = model::SessionStatus::Warning;
        member.in_app_history = model::CommandHistory::from_items(
            member
                .pane_ids
                .iter()
                .filter_map(|id| outputs.get(id))
                .flat_map(|output| output.history()),
        );
        restore_member_histories(&mut member.members, outputs);
    }
}

fn sanitize_members(
    members: &mut [session_store::SavedGroupMember],
    titles: &std::collections::HashMap<String, String>,
    overrides: &std::collections::HashSet<String>,
) {
    for member in members {
        if !overrides.contains(&member.session_id)
            && let Some(title) = titles.get(&member.session_id)
        {
            member.title = title.clone();
        }
        sanitize_members(&mut member.members, titles, overrides);
    }
}

#[cfg(test)]
#[path = "pinned_session_tests.rs"]
pub(crate) mod tests;
