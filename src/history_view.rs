//! Bounded virtualized command-history presentation.

use super::*;

/// The text one history row shows for `item`.
///
/// # Why a successful command carries no suffix
///
/// The review: *"when it's the last command a `exit 0` text is added to it for
/// some reason"*. The row used to append `" [exit {code}]"` for **every**
/// finished command, so the newest entry — the one that had just succeeded —
/// grew a `[exit 0]` tail that read as part of the command. "It worked" is the
/// default outcome and needs no annotation; annotating it drowns the case that
/// does, which is a *failure*.
///
/// So exit code `0` is suppressed and every non-zero code is kept, verbatim.
/// A still-running command (`exit_code: None`) is unannotated too, as before —
/// and, importantly, is not confused with a success: both suppress the suffix,
/// but only one of them can later gain a failure suffix.
pub(crate) fn command_label(item: &model::HistoryItem) -> String {
    match item.exit_code {
        // Success: the command, and nothing else.
        Some(0) | None => item.command.clone(),
        Some(code) => format!("{} [exit {}]", item.command, code),
    }
}

impl App {
    /// Build the command-history overlay.
    ///
    /// "Per-tab command history" — each session (tab) has its own
    /// `in_app_history`, not a global list. This is a real behavioural
    /// difference (tested by `two_tabs_have_independent_history`).
    ///
    /// The row list is a viewport-bounded virtual list. See the row
    /// branch below for why, and `history_viewport_height` for the height.
    pub(super) fn history_overlay<'a>(&self, session: &'a Session) -> Element<'a, Message> {
        self.history_overlay_scoped(session, Message::CloseHistory, Message::PasteHistoryCommand)
    }

    /// The command-history overlay with caller-chosen close and paste routing,
    /// so a detached window scopes both to its own window and session rather
    /// than the main window's active pane.
    pub(super) fn history_overlay_scoped<'a>(
        &self,
        session: &'a Session,
        on_close: Message,
        on_paste: impl Fn(String) -> Message + Clone + 'a,
    ) -> Element<'a, Message> {
        use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};
        let sp = self.enz_theme().spacing;

        let mut content = iced::widget::column![
            iced::widget::row![
                enz_text(crate::i18n::label("Per-tab command history"))
                    .size(TextSize::Footnote)
                    .tone(TextTone::Default),
                Space::new().width(Length::Fill).height(0.0),
                // The dismissal of a transient panel is a
                // neutral action, not the emphasised one. `Ghost` paints its
                // label in `palette.accent` (see `button.rs`'s header), which
                // made this close control read as the panel's primary call to
                // action. `Quiet` is Ghost's documented neutral twin: same
                // borderless shape and hover wash, `palette.text` ink. The
                // label is just "Close" — the panel above it already says what
                // is being closed.
                enz_button(crate::i18n::label("Close"), on_close)
                    .kind(ButtonKind::Quiet)
                    // The visible label is the bare verb the reviewer asked
                    // for, but three other controls in this shell are also
                    // named "Close" (the window X, the SSH sheet, the pane
                    // action bar). An accessible name has to stay
                    // unambiguous, so the AT name keeps the object while the
                    // painted text drops it.
                    .aria_label(crate::i18n::label("Close command history"))
                    .into_element(),
            ]
            .spacing(sp.xs())
            .align_y(Alignment::Center)
            .width(Length::Fill),
        ]
        .spacing(sp.xs())
        .width(Length::Fill)
        .padding(iced::Padding::from([sp.xs(), sp.sm()]));

        if session.in_app_history.is_empty() {
            // Two-line empty state: a title line + a subtle hint telling the user how history gets populated.
            content = content.push(
                enz_text(crate::i18n::label("No commands yet"))
                    .size(TextSize::Caption)
                    .tone(TextTone::Muted)
                    .into_element(),
            );
            content = content.push(
                enz_text(crate::i18n::label(
                    "Run a command in the terminal and it appears here.",
                ))
                .size(TextSize::Caption)
                .tone(TextTone::Subtle)
                .into_element(),
            );
        } else {
            // The rows used to be pushed one-per-entry into this
            // column, so an open overlay realised the WHOLE history and took
            // whatever height that came to — out of the terminal's allocation,
            // because this column is a sibling of the pane. Both halves are
            // fixed here: the model is bounded (`CommandHistory`) and the
            // presentation owns a viewport whose realisation is
            // viewport-sized, not history-sized.
            //
            // `virtual_list` (docs/widgets.md `virtual list`, `large list`) is
            // the shipped widget for this; the rows are derived here in `view`
            // from the session, the owned-`Rc` provider case. The key is
            // `HistoryItem::id`, so retained row state follows a command rather
            // than a slot when eviction shifts every index.
            let rows = std::rc::Rc::new(session.in_app_history.newest_first_rows());
            let live = self.history_scroll.get();
            let viewport = history_viewport_height(session.in_app_history.len());
            let list = enzui_widgets::enz_virtual_list(
                rows,
                HISTORY_ROW_PITCH,
                move |_index, item: &model::HistoryItem| {
                    // The command text is no longer
                    // pre-truncated at a fixed character count.
                    //
                    // It used to be `truncate_command(cmd, 40)`: a hard
                    // 40-**byte** cut with an appended ellipsis, applied
                    // before layout. That is the "truncating too soon"
                    // complaint — the overlay is as wide as the window, so a
                    // 1400 px panel showed 40 characters and acres of empty
                    // space, and a wider window changed nothing. It was also
                    // not UTF-8 safe: `&cmd[..max_len]` panics outright when
                    // byte 40 lands inside a multi-byte character, so a
                    // command containing an accent or a CJK path could crash
                    // the overlay (`truncate_command` is now the char-boundary
                    // safe form, kept for its other callers).
                    //
                    // The full command is handed to the text widget and
                    // elided by the SHAPER at the real measured width, so the
                    // amount shown follows the window.
                    let label = command_label(item);
                    let copy_label = crate::i18n::format(
                        crate::i18n::localizer(),
                        "Copy command: {}",
                        &[("arg0", &(item.command))],
                    );
                    let paste_label = crate::i18n::format(
                        crate::i18n::localizer(),
                        "Paste command into terminal: {}",
                        &[("arg0", &(item.command))],
                    );
                    iced::widget::row![
                        enz_toolbar_icon(
                            "copy",
                            Message::CopyHistoryCommand(item.command.clone()),
                        )
                        .aria_label(copy_label)
                        .tooltip(crate::i18n::label("Copy command"))
                        .into_element(),
                        // "a paste button right to the copy".
                        // `clipboard-paste` is the Lucide name for the
                        // clipboard-with-document mark; the neighbouring copy
                        // control already uses the plain `copy` sheets.
                        enz_toolbar_icon("clipboard-paste", on_paste(item.command.clone()))
                            .aria_label(paste_label)
                            .tooltip(crate::i18n::label("Paste into terminal"))
                            .into_element(),
                        enz_text(label)
                            .size(TextSize::Caption)
                            .tone(TextTone::Default)
                            // Elide at the measured width, filling the row:
                            // width-responsive by construction.
                            .single_line_elided()
                            .full_width(),
                    ]
                    .spacing(sp.xs())
                    .align_y(Alignment::Center)
                    .into()
                },
            )
            .id(iced::widget::Id::new(HISTORY_LIST_ID))
            .viewport_height(live.map_or(viewport, |g| g.bounds.height))
            .scroll_offset(live.map_or(0.0, |g| g.translation.y))
            .height(Length::Fixed(viewport))
            .into_element();
            content = content.push(enzui_widgets::scroll_view::publish_geometry_of(
                list,
                self.history_scroll.clone(),
            ));
        }

        // "command text in the history should be selectable".
        //
        // `selection_scope` is the shipped cross-widget read-only selection
        // (docs/widgets.md: `selectable text`, `select text`, `copy text`),
        // so a drag across the rows produces one continuous selection and one
        // clipboard payload rather than N per-row selections. Scoping it to
        // the whole overlay rather than to each row is what makes selecting
        // across two history entries possible at all; the interactive copy /
        // paste controls inside keep their own pointer semantics, which is
        // the behaviour that widget documents.
        let content = enzui_widgets::selection_scope::enz_selection_scope(content)
            // Ctrl+C from a real cross-row selection enters the same production
            // clipboard/toast command route as the row's Copy button.
            .on_copy(Message::CopyHistoryCommand);
        let panel = container(content)
            .width(Length::Fill)
            .style(move |theme: &iced::Theme| {
                let palette = theme.palette();
                container::Style {
                    background: Some(iced::Background::Color(palette.background.base.color)),
                    ..container::Style::default()
                }
            });
        // A disclosure region: the stable id retains the row list's scroll
        // across rebuilds, and the expanded state names it as the open panel
        // the toggle controls.
        use enzui_core::semantics::{Role, Semantics};
        enzui_widgets::semantics_container::SemanticsContainer::new(
            panel,
            Semantics::new(Role::Group)
                .label(crate::i18n::label("Command history"))
                .expanded(true),
        )
        .id(iced::advanced::widget::Id::from("command-history-overlay"))
        .into()
    }
}
