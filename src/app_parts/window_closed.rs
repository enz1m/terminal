//! Terminal window-closed method.

use super::*;

impl App {
    /// A window was destroyed: re-dock what it carried, then decide whether the
    /// PROCESS should end.
    ///
    /// The exit decision is the shared [`WindowLedger`]'s, not this app's: a
    /// daemon keeps running with zero windows, so "the user closed the last
    /// window" has to become a real `iced::exit()` or the process survives with
    /// nothing on screen.
    ///
    /// [`WindowLedger`]: enzui_core::window_lifecycle::WindowLedger
    pub(crate) fn window_closed(&mut self, id: iced::window::Id) -> Task<Message> {
        use enzui_core::window_lifecycle::LifecycleAction;

        let intent = self.intentional_closes.remove(&id);
        if self.focused_window == Some(id) {
            self.focused_window = None;
            self.invalidate_sidebar();
        }
        self.retire_window_input(id);
        if intent == Some(CloseIntent::Teardown) {
            // The committed close already published everything it owed and
            // emitted its own exit. This event retires window and ledger state
            // and must not re-enter the session or durable-state policy.
            self.detached_windows.remove(&id);
            let _ = self.windows.closed(id);
            self.reconcile_operation_origins();
            return Task::none();
        }
        if let Some(detached) = self.detached_windows.remove(&id)
            && let Some(index) = self
                .sessions
                .iter()
                .position(|session| session.id == detached.session)
        {
            if self
                .main_window
                .is_none_or(|main| self.windows.is_open(main))
            {
                if detached.closing {
                    // Re-docked: the tab comes back into the main window.
                    self.sessions[index].detached = false;
                    self.activate_session(index);
                    self.resize_active_pty();
                } else {
                    self.remove_session_at(index, true);
                }
            }
            self.invalidate_sidebar();
        }
        let action = self.windows.closed(id);
        if self.main_window == Some(id) && !action.exits() {
            self.close_main_window_tabs();
        }
        self.reconcile_operation_origins();
        // This surface is already destroyed, so whatever it still owes is
        // published on a best-effort teardown rather than held for a decision.
        match action {
            LifecycleAction::Continue => Task::none(),
            LifecycleAction::Exit => self.exit_after_pinned_save(id, Vec::new()),
            LifecycleAction::CloseThenExit(others) => self.exit_after_pinned_save(id, others),
        }
    }

    /// The main window closed while detached windows stay open: its own tabs
    /// close with it. A pinned tab is kept, unseen, so the session it is
    /// pinned for is saved when the last window closes, as it would have been
    /// with the main window open.
    fn close_main_window_tabs(&mut self) {
        for index in (0..self.sessions.len()).rev() {
            let session = &self.sessions[index];
            if !session.detached && !session.pinned {
                let _ = self.remove_session_at(index, true);
            }
        }
        self.invalidate_sidebar();
    }
}
