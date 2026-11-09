//! Terminal titlebar-inner method.

use super::*;

impl App {
    pub(crate) fn titlebar_inner(
        &self,
        carries_sidebar_destinations: bool,
    ) -> Element<'_, Message> {
        // Commands are declared directly on the titlebar. Its layout
        // reserves Close before handing the residual to the shared action bar,
        // so no generic pushed child or app-owned compact fold can cover it.
        let grouped = self
            .sessions
            .get(self.active)
            .is_some_and(|session| session.panes.len() > 1);
        let mut commands = self.titlebar_commands();
        if carries_sidebar_destinations {
            // The sidebar's own `server` icon is off screen, so this is
            // the only route to the SSH manager. While the sidebar IS inline it
            // publishes that icon itself and this must not be a second copy.
            commands.push((
                "app.ssh",
                "server",
                crate::i18n::label(SIDEBAR_SSH_LABEL).to_owned(),
                Message::OpenSshManager,
            ));
            // Settings lives in the sidebar's band too; off screen, here.
            commands.push((
                "app.settings",
                "settings",
                crate::i18n::label("Settings").to_owned(),
                Message::OpenSettings,
            ));
        }

        let mut items: Vec<TitlebarActionItem<'_, Message>> = Vec::new();
        if carries_sidebar_destinations {
            // The sidebar header's `plus` is off screen, so the band
            // owns "New local terminal" — and owns it on the WIDE rung too
            // (a user who collapsed navigation at 1400 px used to be left with
            // no route to a new session at all). Pinned, so it survives the
            // fold as one affordance-sized box beside the kebab.
            items.push(self.new_terminal_action_item());
        }
        items.extend(commands.into_iter().map(|(id, icon, label, msg)| {
            // The folded menu row carries the SAME lucide glyph the
            // inline icon button paints — one binding, `icon`, read by both, so
            // the row and the button cannot disagree. The group is the
            // command's category, authored contiguously with the pinned
            // `session` commands above: `session` (close, history) then `app`
            // (settings, and the compact-rung SSH manager), so exactly one
            // separator is painted, between the two runs.
            let group = match id {
                "app.settings" | "app.ssh" => "app",
                id if id.starts_with("pane.") => "pane",
                _ => "session",
            };
            let action = Action::new(id, msg.clone())
                .with_label(label.clone())
                .with_icon(Icon::named(icon));
            TitlebarActionItem::with_inline(Descriptor::new(id), action, move |_, _| {
                let mut button = enz_toolbar_icon(icon, msg.clone()).label(label.clone());
                if !grouped {
                    button = button.tokens(titlebar_icon_tokens());
                }
                if id == "app.settings" {
                    button = button.id(settings_button_id());
                }
                button.into()
            })
            .group(group)
        }));

        enz_title_bar()
            .center(self.titlebar_find())
            // Transparent chrome reveals the surface selected below: the ANSI
            // canvas for an ordinary terminal, and shell background for a
            // grouped workspace.
            .tone(enzui_widgets::surface::SurfaceTone::Transparent)
            .id("terminal-emulator-window")
            .actions(items, Message::ToggleTitlebarOverflow)
            // Framework dismissal closes the shared menu for every command route.
            .action_dismiss(Message::CloseTitlebarOverflow)
            // The menu rows are stable automation and accessibility
            // targets. This caller-owned namespace prevents another action
            // menu carrying the same descriptor ids from aliasing them.
            .action_menu_id_prefix(TITLEBAR_OVERFLOW_MENU_ID_PREFIX)
            // The fold collector is a framework-built control on this
            // transparent band, so it takes the same canvas-resolved tokens the
            // app's own icons take. The app theme's ink would be dark grey on a
            // dark terminal canvas in light mode, about 2.71:1 against the
            // 4.5:1 icon-only floor.
            .action_collector(enzui_widgets::action_bar::Collector::Kebab {
                icon: "ellipsis-vertical".to_owned(),
                tokens: (!grouped).then(titlebar_icon_tokens),
            })
            .overflow_open(self.titlebar_overflow_open)
            // Stable target for keyboard and accessibility navigation.
            .action_overflow_trigger_id(titlebar_overflow_button_id())
            .into_element()
    }
}
