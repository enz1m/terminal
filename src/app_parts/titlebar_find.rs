//! Terminal titlebar find methods.

use super::*;

impl App {
    /// The framework search field owns its inline/trigger presentation and popover.
    /// Match status and navigation live in its results slot while a find is active.
    pub(crate) fn titlebar_find(&self) -> Element<'_, Message> {
        use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
        let mut search = enz_titlebar_search(&self.find.query, Message::FindQueryChanged)
            .placeholder(crate::i18n::text(FIND_PLACEHOLDER))
            // The id `Message::OpenFind` (Ctrl+Shift+F) routes focus to. Not
            // `focus_on_mount`: the field is mounted for the app's whole life,
            // so a mount-time claim would steal the first keystroke of every
            // session from the shell.
            .id(find_field_id())
            .on_submit(Message::FindNext)
            // Clearing the field ends the find: the highlights and the
            // scroll override go with the query.
            .on_clear(Message::CloseFind);
        let on_canvas = self
            .sessions
            .get(self.active)
            .is_none_or(|session| session.panes.len() <= 1);
        if on_canvas {
            search = search.palette(terminal_canvas_palette());
        }

        // A closed find has no result status or navigation controls to expose.
        // Keeping the result row mounted while `results_open(false)` retained
        // five dormant tree nodes in every frame, including compact titlebars
        // where only the collapsed trigger is visible. Build the row only when
        // it can be reached; the open presentation and all messages are
        // unchanged.
        if self.find.open && !self.find.query.is_empty() {
            let sp = self.enz_theme().spacing;
            let status = self.find.status();
            let results: Element<'_, Message> = row![
                enz_text(if status.is_empty() {
                    crate::i18n::label(FIND_EMPTY_STATUS).to_owned()
                } else {
                    status
                })
                .size(TextSize::Caption)
                .tone(if self.find.matches.is_empty() {
                    TextTone::Muted
                } else {
                    TextTone::Default
                })
                .into_element(),
                Space::new().width(Length::Fill).height(0.0),
                enz_button(crate::i18n::label("Previous match"), Message::FindPrev)
                    .kind(ButtonKind::Ghost)
                    .enabled(!self.find.matches.is_empty())
                    .icon("chevron-up")
                    .icon_position(ButtonIconPosition::Only)
                    .aria_label(crate::i18n::label("Previous (Shift+Enter)"))
                    .into_element(),
                enz_button(crate::i18n::label("Next match"), Message::FindNext)
                    .kind(ButtonKind::Ghost)
                    .enabled(!self.find.matches.is_empty())
                    .icon("chevron-down")
                    .icon_position(ButtonIconPosition::Only)
                    .aria_label(crate::i18n::label("Next (Enter)"))
                    .into_element(),
            ]
            .spacing(sp.xs())
            .align_y(Alignment::Center)
            .into();
            // The results popover paints with the canvas palette like the
            // field; its text and buttons must resolve the same palette, or
            // "1 of 241" came out near-black on the dark popover.
            let results: Element<'_, Message> = if on_canvas {
                let mut canvas = self.enz_theme().clone();
                canvas.palette = terminal_canvas_palette();
                iced::widget::themer(Some(canvas.iced_theme()), results).into()
            } else {
                results
            };
            search = search.results(results).results_open(true);
        }
        search.into()
    }

    /// Keep session creation reachable when the sidebar is off screen.
    pub(crate) fn new_terminal_action_item(&self) -> TitlebarActionItem<'_, Message> {
        let grouped = self
            .sessions
            .get(self.active)
            .is_some_and(|session| session.panes.len() > 1);
        TitlebarActionItem::with_inline(
            Descriptor::new("session.new").pinned(),
            Action::new("session.new", Message::NewLocal)
                .with_label(crate::i18n::label("New local terminal"))
                .with_icon(Icon::named("plus")),
            move |_, _| {
                let mut button = enz_toolbar_icon("plus", Message::NewLocal)
                    .label(crate::i18n::label("New local terminal"));
                if !grouped {
                    button = button.tokens(titlebar_icon_tokens());
                }
                button.into()
            },
        )
        .group("session")
    }
}
