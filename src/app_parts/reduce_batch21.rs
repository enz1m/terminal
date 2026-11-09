//! Terminal reducer batch 21: appearance reset and settings page arms.

use super::*;

impl App {
    pub(crate) fn reduce21_reset_appearance_0(&mut self) -> Task<Message> {
        let defaults = Settings::default();
        self.settings.font_size_px = defaults.font_size_px;
        self.settings.cursor_style = defaults.cursor_style;
        self.settings.color_scheme = defaults.color_scheme.clone();
        self.settings.scrollback_lines = defaults.scrollback_lines;
        // Keep the cached subtitle in step with the value it renders.
        // The reset writes to the ring too. Without this the
        // reset would restore the field but leave every live PTY at
        // its pre-reset cap.
        let cap = self.settings.clamped_scrollback() as usize;
        for session in self.ptys.values() {
            session.set_max_scrollback(cap);
        }
        self.drain_ptys();
        color_scheme::set_active_by_id(self.settings.resolved_color_scheme());
        self.resize_active_pty();
        self.save_preferences()
    }
    pub(crate) fn reduce21_select_settings_page_1(&mut self, page: String) -> Task<Message> {
        if matches!(page.as_str(), "appearance" | "shortcuts") {
            self.settings_page = page;
            self.settings_drilled_in = settings_surface::settings_dialog_nav(self)
                == enzui_widgets::settings_dialog::SettingsNav::Stacked;
        }

        Task::none()
    }
}
