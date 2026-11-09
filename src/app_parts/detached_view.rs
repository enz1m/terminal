//! Terminal detached-view method.

use super::*;

impl App {
    pub(crate) fn detached_view<'a>(
        &'a self,
        window: iced::window::Id,
        session_id: &'a str,
    ) -> Element<'a, Message> {
        let chrome = self.window_chrome.get(&window);
        let tokens = crate::shell_tokens::detached_shell_tokens();
        enz_app_shell(move || self.detached_content(window, session_id))
            .layout(ShellLayout::FloatingTitlebar)
            .tokens(tokens)
            .breakpoints(self.enz_theme().breakpoints)
            .titlebar(move || self.detached_titlebar(window, session_id))
            .maximized(chrome.is_some_and(enzui_widgets::app_shell::ShellWindowState::maximized))
            .fullscreen(chrome.is_some_and(enzui_widgets::app_shell::ShellWindowState::fullscreen))
            .on_window_gesture(move |command| Message::WindowFor(window, command))
            .into_element()
    }

    fn detached_titlebar(
        &self,
        window: iced::window::Id,
        session_id: &str,
    ) -> Element<'_, Message> {
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
        else {
            return grid_view::empty_grid();
        };
        let history_open = self
            .detached_windows
            .get(&window)
            .is_some_and(|window| window.history_open);
        let mut actions = Vec::new();
        for (id, icon, label, direction) in [
            (
                "pane.split-right",
                "columns-2",
                "Split pane right on the same endpoint",
                pane_layout::SplitDirection::Horizontal,
            ),
            (
                "pane.split-down",
                "rows-2",
                "Split pane down on the same endpoint",
                pane_layout::SplitDirection::Vertical,
            ),
        ] {
            let pane = session
                .panes
                .get(session.active_pane)
                .map(|pane| pane.id.clone())
                .unwrap_or_default();
            actions.push(TitlebarActionItem::icon(
                id,
                icon,
                crate::i18n::label(label),
                Message::DetachedSplitPane {
                    window,
                    pane,
                    direction,
                },
            ));
        }
        actions.push(TitlebarActionItem::with_inline(
            Descriptor::new("session.history"),
            Action::new("session.history", Message::DetachedToggleHistory(window))
                .with_label(crate::i18n::label("Command history"))
                .with_icon(Icon::named("history")),
            move |action, _| {
                enz_toolbar_icon("history", action.message().clone())
                    .label(action.label())
                    .expanded(history_open)
                    .into()
            },
        ));
        actions.push(TitlebarActionItem::icon(
            "session.redock",
            "corner-down-left",
            crate::i18n::format(
                crate::i18n::localizer(),
                "Re-dock {} into the main window",
                &[("arg0", &session.title)],
            ),
            Message::RedockSession(session.id.clone()),
        ));
        let overflow_open = self
            .detached_windows
            .get(&window)
            .is_some_and(|d| d.titlebar_overflow_open);
        enz_title_bar()
            .tone(enzui_widgets::surface::SurfaceTone::Transparent)
            .id(format!("terminal-detached-{window:?}"))
            .actions(
                actions,
                Message::DetachedTitlebarOverflow(window, !overflow_open),
            )
            .overflow_open(overflow_open)
            .action_dismiss(Message::DetachedTitlebarOverflow(window, false))
            .action_menu_id_prefix(format!("terminal-detached-actions-{window:?}"))
            .into_element()
    }

    fn detached_content(&self, window: iced::window::Id, session_id: &str) -> Element<'_, Message> {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return grid_view::empty_grid();
        };
        let strip = self.detached_top_strip();
        let detached_size = self
            .detached_windows
            .get(&window)
            .map_or(iced::Size::ZERO, |d| d.size);
        let history_open = self
            .detached_windows
            .get(&window)
            .is_some_and(|d| d.history_open);

        let canvas = grid_view::terminal_background();
        let find_height = self.input_find_height(window);
        let body = container(self.detached_panels(window, session, detached_size, strip))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::default().top(strip + find_height))
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(canvas.into()),
                ..container::Style::default()
            });

        let mut layers = iced::widget::stack![body];
        if find_height > 0.0 {
            layers = layers.push(
                container(self.detached_find_bar(window, session))
                    .width(Length::Fill)
                    .padding(iced::Padding::default().top(strip)),
            );
        }
        if history_open {
            layers = layers.push(
                container(self.scope_element(
                    &session.id,
                    &session.panes[session.active_pane].id,
                    self.history_overlay_scoped(
                        session,
                        Message::DetachedCloseHistory(window),
                        move |command| Message::DetachedPasteHistoryCommand { window, command },
                    ),
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(Alignment::Start)
                .padding(iced::Padding::default().top(strip)),
            );
        }

        let presentation = self.notice_layer(Some(window), layers.into(), None);
        self.input_menu_view(Some(window), presentation)
    }
}
