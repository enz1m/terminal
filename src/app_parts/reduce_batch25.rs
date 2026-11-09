//! Terminal reducer batch 25: pinned-clear and find arms.

use super::*;

impl App {
    pub(crate) fn reduce25_clear_pinned_output_0(&mut self) -> Task<Message> {
        if let Some(session) = self.sessions.get(self.active)
            && let Some(reference) = self.secure_pins.references.get(&session.id).cloned()
        {
            self.request_pinned_clear(reference);
        }

        Task::none()
    }
    pub(crate) fn reduce25_open_find_1(&mut self) -> Task<Message> {
        self.find_owner = self.active_pane_id();
        self.find.open();
        // The field lives in the titlebar and is always mounted, so the
        // accelerator routes focus to it rather than mounting a row.
        // `focus(id)` reaches the field at both rungs: when the widget has
        // collapsed to its icon, the framework lays the field out hidden, sees
        // the routed focus land on it and opens the popover the same frame.
        //
        // No `resize_active_pty()`: opening find no longer takes
        // height from the transcript, so there is no new geometry to
        // publish. A resize here would be a lie the PTY acts on.
        focus_find_task()
    }
    pub(crate) fn reduce25_close_find_2(&mut self) -> Task<Message> {
        self.find.close();
        // The field is chrome. Leaving focus on it, or restoring focus to a
        // titlebar invoker, would send the next Space to the chrome instead of
        // the shell, so dismissing find hands the keyboard back to the
        // terminal.
        self.release_input_focus()
    }
    pub(crate) fn reduce25_find_query_changed_3(&mut self, query: String) -> Task<Message> {
        self.find_owner = self.active_pane_id();
        // The field is always present, so TYPING is the
        // opening gesture — a user who clicks the titlebar field and
        // types never sends `OpenFind`, and without this the matches
        // would be computed and then dropped by the `find.open` guard
        // in `find_matches_for_window`/`grid_window`.
        if !query.is_empty() {
            self.find.open();
        }
        self.find.set_query(query);
        // Search the active pane's transcript.
        if let Some(s) = self.sessions.get(self.active)
            && let Some(p) = s.panes.get(s.active_pane)
        {
            self.find.search(&p.transcript);
        }

        Task::none()
    }
    pub(crate) fn reduce25_find_next_4(&mut self) -> Task<Message> {
        self.find.next_match();

        Task::none()
    }
    pub(crate) fn reduce25_find_prev_5(&mut self) -> Task<Message> {
        self.find.prev_match();

        Task::none()
    }
}
