//! Terminal reducer batch 20: focus and appearance arms.

use super::*;

impl App {
    pub(crate) fn reduce20_focus_next_0(&mut self) -> Task<Message> {
        // Advance keyboard focus so the titlebar's close X is reachable without
        // a mouse, with the framework's wrapping operation (the stock one drops
        // the ring at the last control; the key is Ctrl+F6, `classify_event`).
        // Suppressed while the settings modal is open: there the modal host
        // owns Tab and wraps focus inside the dialog, and the global traversal
        // would escape it.
        if !self.settings_open && !self.ssh_manager_open {
            return operate(traverse::<Message>(Traversal::Next));
        }

        Task::none()
    }
    pub(crate) fn reduce20_focus_previous_1(&mut self) -> Task<Message> {
        if !self.settings_open && !self.ssh_manager_open {
            return operate(traverse::<Message>(Traversal::Previous));
        }

        Task::none()
    }
    pub(crate) fn reduce20_set_color_scheme_2(&mut self, id: String) -> Task<Message> {
        self.settings.color_scheme = id;
        color_scheme::set_active_by_id(self.settings.resolved_color_scheme());
        self.save_preferences()
    }
    pub(crate) fn reduce20_set_font_size_px_3(&mut self, px: i32) -> Task<Message> {
        self.settings.font_size_px = px.clamp(model::FONT_SIZE_MIN_PX, model::FONT_SIZE_MAX_PX);
        // Recompute cell pitch and immediately tell the shell its new
        // rows/columns; storing the number alone leaves TUIs stale.
        self.resize_active_pty();
        self.save_preferences()
    }
    pub(crate) fn reduce20_set_cursor_style_4(&mut self, style: String) -> Task<Message> {
        self.settings.cursor_style = style;
        self.save_preferences()
    }
    pub(crate) fn reduce20_set_scrollback_5(&mut self, n: i32) -> Task<Message> {
        // Clamp through the same helper the label advertises
        // the range with, then push the resolved bound into every
        // live PTY so the ring shrinks/grows immediately.
        self.settings.scrollback_lines = n;
        let resolved = self.settings.clamped_scrollback();
        self.settings.scrollback_lines = resolved;
        let cap = resolved as usize;
        for session in self.ptys.values() {
            session.set_max_scrollback(cap);
        }
        self.drain_ptys();
        self.save_preferences()
    }
}
