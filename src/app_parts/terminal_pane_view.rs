//! Terminal pane view methods.

use super::*;

impl App {
    /// One independently painted terminal surface in the framework pane tree.
    ///
    /// A leaf has no per-pane frame and no header band: the pane grid's seam
    /// separates two leaves, and the row the header cost is returned to the PTY
    /// by [`Self::pane_text_size`]. Every leaf wires the pointer hooks stamped
    /// with its own pane index, so both panes support the same selection
    /// gestures and neither can extend the other's range.
    pub(crate) fn terminal_pane<'a>(&'a self, pane_id: &'a str) -> Element<'a, Message> {
        #[cfg(test)]
        model::perf::note_terminal_surface();
        use enzui_core::semantics::{Role, Semantics};
        use enzui_widgets::semantics_container::SemanticsContainer;

        let Some(session) = self.sessions.get(self.active) else {
            return grid_view::empty_grid();
        };
        let Some(pane_index) = session.panes.iter().position(|pane| pane.id == pane_id) else {
            return grid_view::empty_grid();
        };
        let pane = &session.panes[pane_index];
        let focused = pane_index == session.active_pane;
        let geom = self.geometry();
        // The leaf's rows, columns, scroll window and pointer-to-cell mapping
        // all derive from this one rect.
        let text = self.pane_text_size(pane_id);
        let visible_rows = geom.rows_for(text.height) as usize;
        let scroll_geom =
            scrollback::ScrollGeometry::new(pane.transcript.len(), visible_rows, geom.cell_height);
        let (win_start, win_end) = self.pane_transcript_window(pane, visible_rows, focused);
        let lines = &pane.transcript[win_start..win_end];
        // Each leaf reports its pointer gestures stamped with its own index;
        // the reducer resolves focus and keeps a drag inside its origin pane.
        let pointer = Some(grid_view::GridPointerHooks {
            press: Message::GridCellPressed,
            drag: Message::GridCellDragged,
            release: Message::GridCellReleased,
            pane: Some(pane_index),
        });
        // One find bar per active session, so only the focused leaf highlights
        // matches.
        let match_highlights: Vec<grid_view::MatchHighlight> = if focused {
            self.find_matches_for_window(pane, win_start, win_end)
        } else {
            Vec::new()
        };
        let (rows, cols) = self
            .pty_geometry_for(&pane.id, text)
            .unwrap_or((visible_rows as u16, geom.cols_for(text.width)));
        let screen = self
            .screens
            .get(&(None, pane.id.clone()), Default::default());
        let fit_pane = pane.id.clone();
        let painted_position = scrollback::ScrollPosition::from_top_row(win_start, scroll_geom);
        let selection = self.selection_for(&pane.id);
        let sync = self.live(
            pane,
            &screen,
            0,
            pane.alt_screen || painted_position.resolves_to_end(scroll_geom),
            selection.is_none() && match_highlights.is_empty(),
            focused,
        );
        let grid = terminal_view::terminal_grid(
            &screen,
            (&[], 0),
            lines,
            &self.settings,
            self.terminal_family(),
            (cols, rows),
            terminal_view::Overlays {
                focused,
                cursor_visible: !pane.cursor_hidden,
                // This leaf's own selection; a pane never receives a sibling's
                // range.
                selection,
                window_start_row: win_start,
                matches: &match_highlights,
            },
            terminal_view::Routes {
                pointer,
                wheel: None,
                fit: Box::new(move |cols, rows| Message::TerminalFit {
                    pane: fit_pane.clone(),
                    cols,
                    rows,
                }),
                ignored: Message::TerminalViewIgnored,
            },
            self.pane_inset(pane.alt_screen),
            sync,
        );
        // Wheel/drag callbacks are wired only on the focused leaf, so an
        // unfocused leaf's scrollbar chrome is inert.
        let cell_height = geom.cell_height;
        let mut chrome = enz_offset_scrollbar(grid)
            .content_len(scroll_geom.content_height())
            .viewport_len(scroll_geom.viewport_height())
            .offset(painted_position.scroll_top_px(scroll_geom))
            .backdrop(grid_view::terminal_background());
        if focused {
            chrome = chrome
                .on_scroll_to(move |px| {
                    Message::ScrollTranscriptToRow(scrollback::top_row_for_px(px, scroll_geom))
                })
                .on_wheel(move |delta| {
                    Message::TranscriptWheel(scrollback::wheel_rows(delta, cell_height))
                });
        }
        let surface = self.transcript_surface(
            chrome.into(),
            (focused && !painted_position.resolves_to_end(scroll_geom))
                .then(|| self.jump_to_latest_button(Message::JumpToLatest)),
        );
        let canvas = grid_view::terminal_background();
        let painted = container(surface)
            .width(Length::Fill)
            .height(Length::Fill)
            // Clip the leaf to its own rect. Terminal geometry is
            // frozen to the committed split while a pointer resize is live
            // (`pane_surface_size`), so a shrinking leaf's frozen text is wider
            // or taller than the moving region; the clip hides the sweep
            // instead of bleeding it across the seam into the neighbour.
            .clip(true)
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(canvas.into()),
                ..container::Style::default()
            });
        // The panes the keyboard is not in recede. The shade is always the
        // stack's second child, clear on the focused pane, so a focus change
        // repaints it without re-parenting the terminal beneath.
        let shade = (!focused).then(grid_view::unfocused_pane_shade);
        let painted = iced::widget::stack![
            painted,
            container(iced::widget::Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_theme: &iced::Theme| container::Style {
                    background: shade.map(Into::into),
                    ..container::Style::default()
                }),
        ];
        let id = pane.id.clone();
        let labelled = SemanticsContainer::new(
            painted,
            Semantics::new(Role::Group)
                .label(crate::i18n::format(
                    crate::i18n::localizer(),
                    "{} terminal pane, {}",
                    &[("arg0", &(pane.title)), ("arg1", &(pane.cwd))],
                ))
                .focused(focused),
        )
        .id(iced::advanced::widget::Id::from(format!(
            "terminal-pane-{}",
            pane.id
        )));
        // The grid captures presses on text; the `mouse_area` catches presses
        // it declines (the inset margin, the scrollbar lane) so an edge click
        // still focuses the leaf.
        let leaf = iced::widget::mouse_area(labelled)
            .on_press(Message::FocusPane(id))
            .into();
        self.input_surface(session, pane, win_start, leaf)
    }

    /// The terminal pane(s) alone: one surface, a single narrow-rung leaf, or
    /// the framework pane grid. Wrapped in `responsive` by [`Self::content_body`]
    /// so its allocation is recorded for PTY geometry.
    pub(crate) fn panes_element(&self) -> Element<'_, Message> {
        let Some(session) = self.sessions.get(self.active) else {
            return grid_view::empty_grid();
        };
        if session.panes.len() == 1 {
            self.single_terminal_pane()
        } else if self.panes_stack_narrow(session) {
            session
                .panes
                .get(session.active_pane)
                .map_or_else(grid_view::empty_grid, |pane| self.terminal_pane(&pane.id))
        } else {
            enzui_widgets::enz_pane_grid(
                &session.pane_grid,
                |pane_id, _maximized| self.terminal_pane(pane_id),
                Message::PaneGridEvent,
            )
            .id_base(format!("terminal-pane-grid-{}", session.id))
            .seam_color(grid_view::pane_seam())
            .into_element()
        }
    }

    pub(crate) fn content_body(&self) -> Element<'_, Message> {
        if self.startup.is_some() {
            return self.startup_notice();
        }
        // No session to show (none at all, or the active one is detached
        // into its own window, which draws it): the empty-terminal page.
        let Some(session) = self
            .sessions
            .get(self.active)
            .filter(|_| !self.in_own_window(self.active))
        else {
            return self.empty_terminal();
        };
        let enz = self.enz_theme();
        let sp = enz.spacing;
        let mut parts: Vec<Element<'_, Message>> = Vec::new();

        // The body find row is GONE. Find is a real field in
        // the titlebar band (`titlebar_find`), so the transcript column no
        // longer loses a measured row to it and nothing reflows the PTY when
        // a user searches.

        // Multi-pane controls live in the shell titlebar. Keeping them out of
        // the content column leaves every terminal leaf as a peer surface and
        // prevents a group-only toolbar from stealing terminal rows. The
        // transient rename editor remains in content because it is the result
        // of the titlebar command, not a persistent group control.
        if session.panes.len() > 1
            && let Some(draft) = &self.pane_rename_draft
        {
            parts.push(
                container(
                    enz_text_field(
                        "",
                        crate::i18n::text("Pane name"),
                        draft,
                        Message::PaneRenameEdited,
                    )
                    .on_submit(Message::CommitPaneRename)
                    .aria_label(crate::i18n::text("Pane name"))
                    .id_prefix("term-pane-rename")
                    .into_element(),
                )
                .padding(iced::Padding::from([sp.xs(), sp.sm()]))
                .into(),
            );
        }

        // Failures and pin storage are notices over the grid
        // (`window_notices`), so they never take terminal rows.
        // The shell frame also consumes height when no banner is present.
        // PTY sizing and scroll windows use the allocation that actually paints.
        // A split's seams meet a line under the titlebar strip rather than
        // ending in the strip's open canvas.
        if session.panes.len() > 1 && !self.panes_stack_narrow(session) {
            parts.push(
                container(iced::widget::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(enzui_widgets::pane_grid::SEAM_THICKNESS))
                    .style(|_theme: &iced::Theme| container::Style {
                        background: Some(grid_view::pane_seam().into()),
                        ..container::Style::default()
                    })
                    .into(),
            );
        }
        let grid_area = self.grid_area.clone();
        let panes: Element<'_, Message> = iced::widget::responsive(move |size| {
            grid_area.set(Some(size));
            self.panes_element()
        })
        .into();
        parts.push(panes);

        // ---- History overlay (cluster C) ---------------------------------
        if self.history_open {
            parts.push(self.history_overlay(session));
        }

        // Reserve the top strip the floating icon bar draws over.
        //
        // The grid is inset below the floating bar: the icon cluster spans the
        // same y-band as terminal rows 1-2, so without the inset any line long
        // enough to reach the icons renders underneath them, unreadable, and a
        // right-aligned program paints into the obstructed zone because
        // `cols_for` hands the shell the full width.
        //
        // Fix (issue option 2, "reserve the height"): inset the content column's
        // top by the floating bar's own footprint so the terminal — and any
        // find/pane bar, which carry their own trailing controls into the same
        // obstructed corner — start BELOW the bar. The inset is the framework's
        // reported `content_inset[0]` for this exact layout (`grid_top_inset`),
        // never a hardcoded 40/52, so it tracks the band height and shell gap.
        // The full pane width below the bar is unobstructed, so `cols_for` stays
        // full-width and honest; `rows_for` loses the inset rows (subtracted in
        // `resize_active_pty`/`grid_window`) so the PTY row count matches what
        // is actually visible.
        //
        // The outer container still paints the terminal's own canvas, so the
        // reserved strip reads as the same dark surface the bar floats over;
        // only the text and cursor are inset out from under the icons.
        let top_inset = self.grid_top_inset();
        // The ordinary terminal's inner inset. Applied on the one container
        // that owns the content column, so the find row and the pane tree share
        // it, and taken from the same `terminal_inset()` every geometry helper
        // subtracts, so the paint and the PTY's rows/cols cannot disagree. On
        // the alternate screen it is 0.0, so a TUI reaches every edge.
        // The split content area rounds its top corners like the unsplit
        // terminal. Both fill the whole content pane with the terminal
        // background so the shell's rounded-pane clip shows at the top; a
        // palette-coloured split fill would leave the strip above the leaves
        // invisible against the backdrop. The inter-leaf seam stays
        // `palette.background` through the pane grid's own seam paint.
        let content_background = grid_view::terminal_background();
        container(column(parts).width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding::ZERO.top(top_inset))
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(content_background.into()),
                ..container::Style::default()
            })
            .into()
    }

    /// Whether `session`'s panes are too many for the content width to tile,
    /// so only the active one shows.
    pub(crate) fn panes_stack_narrow(&self, session: &model::Session) -> bool {
        pane_layout::is_narrow_layout_with(
            self.content_pane_width(),
            session.panes.len(),
            &self.enz_theme().sizing,
        )
    }
}
