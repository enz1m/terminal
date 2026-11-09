//! Terminal shell-composition method.

use super::*;

impl App {
    pub(crate) fn shell_composition(&self) -> Element<'_, Message> {
        let enz = self.enz_theme();
        let mut tokens = shell_tokens(enz);
        if self
            .sessions
            .get(self.active)
            .is_none_or(|session| session.panes.len() <= 1)
            && self.effective_presentation()
                != enzui_widgets::nav_split::SidebarPresentation::Inline
        {
            tokens = tokens.toggle_icon_tokens(titlebar_icon_tokens());
        }
        let mut builder = enz_app_shell(|| self.content())
            // The terminal canvas continues behind its floating titlebar.
            .layout(ShellLayout::FloatingTitlebarSidebar)
            .tokens(tokens)
            .on_window_gesture(Message::Window)
            .maximized(
                self.main_window
                    .and_then(|id| self.window_chrome.get(&id))
                    .is_some_and(enzui_widgets::ShellWindowState::maximized),
            )
            .fullscreen(
                self.main_window
                    .and_then(|id| self.window_chrome.get(&id))
                    .is_some_and(enzui_widgets::ShellWindowState::fullscreen),
            )
            .sidebar_label(crate::i18n::label("Sessions"))
            .breakpoints(enz.breakpoints)
            .state(self.nav_split)
            .titlebar_with_geometry(|geometry| self.titlebar(geometry))
            .sidebar(|| self.sidebar_body_cached())
            .sidebar_titlebar(|| self.sidebar_header())
            .on_sidebar_state(Message::SidebarState)
            .on_dismiss_overlay(Message::DismissSidebar);
        if let Some(p) = self.sidebar_force() {
            builder = builder.forced(p);
        }
        // One drag session spans the source rail and destination canvas.
        enzui_widgets::drag_drop_host::enz_drag_drop_host(
            &self.drag_channel,
            builder.into_element(),
        )
        .on_item_drop(Message::SessionDropped)
        .into()
    }
}
