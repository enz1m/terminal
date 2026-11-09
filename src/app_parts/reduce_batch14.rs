//! Terminal reducer batch 14: context-menu arms.

use super::*;

impl App {
    pub(crate) fn reduce14_open_context_menu_0(
        &mut self,
        id: String,
        at: iced::Point,
    ) -> Task<Message> {
        // Guarded on the session still existing: a right-click that
        // raced a close must not open a menu against nothing, which
        // would then render as an empty popover.
        //
        // Build the rows, then let `OpenContextMenu::new` refuse
        // a model with no focusable row — the framework host's own
        // `is_available` rule, so the app never hands it a model it
        // would immediately close.
        // Built into a local first: `row_context(self, ..)` borrows
        // the whole `App`, and assigning into `self.context_menu` in
        // the same expression would overlap that borrow with the
        // place being written.
        let menu = self
            .sessions
            .iter()
            .position(|s| s.id == id)
            .map(|index| {
                (
                    index,
                    self.sessions[index].pinned,
                    context_menu::row_context(self, index),
                )
            })
            .and_then(|(_, pinned, ctx)| {
                let rows = context_menu::menu_model_in(pinned, ctx);
                context_menu::OpenContextMenu::new(id.clone(), at, rows)
            });
        self.context_menu = menu;

        Task::none()
    }
    pub(crate) fn reduce14_context_menu_event_1(
        &mut self,
        event: enzui_widgets::context_menu_host::ContextMenuEvent,
    ) -> Task<Message> {
        use enzui_widgets::context_menu_host::apply_event_restoring;
        let Some(open) = self.context_menu.as_mut() else {
            // A dismissal arriving after the menu already closed is
            // benign and must stay a no-op, not a panic.
            return Task::none();
        };
        let id = open.session.clone();
        let update = apply_event_restoring::<Message>(&mut open.model, &mut open.state, &event);
        let fired = update.activated().cloned();
        // A closed state means the showing is over: drop the whole
        // `OpenContextMenu`, so a closed menu cannot keep a stale model
        // (and its highlight) alive into the next opening.
        if !self
            .context_menu
            .as_ref()
            .is_some_and(|m| m.state.is_open())
        {
            self.context_menu = None;
        }
        // Resolve the fired id against the session's LIVE position,
        // then re-dispatch through the existing typed arm, so a
        // context-menu `Close tab` and the toolbar's still run one
        // reducer body.
        let resolved = fired.as_ref().and_then(|entry| {
            let index = self.sessions.iter().position(|s| s.id == id)?;
            context_menu::row_for_id(context_menu::row_context(self, index), entry)
        });
        if let Some(row) = resolved {
            let task = self.update(Message::ContextRowActivated(id, row));
            return update.finish(task);
        }
        update.finish(Task::none())
    }
    pub(crate) fn reduce14_context_row_activated_2(
        &mut self,
        id: String,
        row: context_menu::RowKind,
    ) -> Task<Message> {
        // The menu closes FIRST, whatever the row does: a menu left up
        // over the result of its own action is the classic half-wired
        // context menu.
        self.context_menu = None;
        // `Close tab` from the context menu focuses the
        // successor session row; every other row returns `None` and
        // leaves focus alone.
        // `Detach tab` produces a real window, so this
        // row's consequence is a Task, not a focus id. Handled before
        // the state-only rows so there is ONE detach path (the same
        // `toggle_session_detached` helper) instead of a second bool
        // flip here.
        if row == context_menu::RowKind::ToggleDetach {
            let detached = self
                .sessions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.detached);
            if detached.is_some() {
                return self.toggle_session_detached(&id);
            }
            return Task::none();
        }
        if let Some(target) = self.apply_context_row(&id, row) {
            return restore_session_row_focus_task(target);
        }

        Task::none()
    }
}
