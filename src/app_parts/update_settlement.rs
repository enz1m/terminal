//! Terminal modal, persistence and close settlement funnel.

use super::*;

impl App {
    pub(crate) fn update_settled(&mut self, message: Message) -> Task<Message> {
        if !self.accepts_operation_completion(&message) {
            return Task::none();
        }
        // Modal hosts own traversal, but scoped AT actions must survive refusal
        // of the app-global focus route without triggering focus reconciliation.
        if (self.settings_open || self.ssh_manager_open)
            && matches!(message, Message::FocusNext | Message::FocusPrevious)
        {
            return Task::none();
        }
        let takes_terminal_focus = matches!(message, Message::GridCellPressed(_));
        let before_focus = self.sync_standard_modal_focus();
        let task = Task::batch([before_focus, self.update_inner(message)]);
        self.reconcile_operation_origins();
        let task = Task::batch([task, self.sync_standard_modal_focus()]);
        self.reconcile_ssh_host_save();
        // After every reducer settles, mirror the retained settings
        // presentation against the logical `settings_open` state. Also retires
        // an outgoing settings generation if another overlay just took the
        // modal host (e.g. the SSH manager opened during an exit), so the
        // settling exit cannot resurrect a dialog whose retention was already
        // superseded (calendar's `sync_settings_presentation` pattern).
        settings_surface::sync(self);
        self.reconcile_open_context_menu();
        // A press on the terminal surface TAKES keyboard
        // ownership away from the chrome.
        //
        // Closing Settings restores focus to its invoker, the titlebar gear, so
        // focus does not vanish for assistive tech. The transcript is a
        // `text_grid`, which registers no focusable of its own, so without this
        // a click on it would leave the gear focused and the next Space or
        // Enter would re-open the dialog instead of reaching the shell.
        //
        // The fix runs the framework's own `release_all_focus` operation over
        // the real tree, so it is the pointer press — not an app-global key
        // exception in the `KeyPressed` arm — that clears chrome focus. Any
        // future focusable in the chrome is covered without being enumerated
        // here. See `terminal_press_releases_chrome_focus`.
        let task = if takes_terminal_focus {
            Task::batch([self.release_input_focus(), task])
        } else {
            task
        };
        // A tab closed while it was detached leaves a window with
        // nothing to show. Draining here — ONE funnel, after every arm — rather
        // than in each closing arm means a future "close tab" route cannot ship
        // with the orphan forgotten only for that one.
        let task = match self.take_orphaned_windows() {
            Some(closes) => Task::batch([task, closes]),
            None => task,
        };
        // Snapshot after every reducer instead of guessing which messages may
        // mutate restartable state. The typed comparison is the dirty bit: it
        // sees nested reducers and every snapshot-relevant field, while
        // transcript/status/chrome-only messages still schedule no write.
        // `queue_session_save` keeps the existing single-writer coalescing, so
        // repeated keyboard focus changes replace the queued latest snapshot
        // instead of starting one write per repeat.
        let queued = self.queue_session_save();
        // A close holds until its last owed write lands, so the turn that
        // settles one reviews it here, after the pass above has taken anything
        // newly owed. Reviewing in the funnel rather than in the completion arm
        // keeps the commit reachable however that arm returned.
        let settled = self.settle_close();
        Task::batch([task, queued, settled])
    }
}
