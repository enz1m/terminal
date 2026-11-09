//! Terminal detached-pane method.

use super::*;

impl App {
    pub(crate) fn detached_pane<'a>(
        &'a self,
        window: iced::window::Id,
        session: &'a Session,
        _size: iced::Size,
        _top_inset: f32,
        pane_id: &str,
    ) -> Element<'a, Message> {
        let Some(pane) = session.panes.iter().find(|pane| pane.id == pane_id) else {
            return grid_view::empty_grid();
        };
        let geom = self.geometry();
        let inset = if pane.alt_screen {
            0.0
        } else {
            self.enz_theme().spacing.sm()
        };
        let text_size = self.input_text_size(window, session, pane);
        let (text_w, text_h) = (text_size.width, text_size.height);
        let rows = geom.rows_for(text_h) as usize;
        let scroll_geom =
            scrollback::ScrollGeometry::new(pane.transcript.len(), rows, geom.cell_height);
        let retained = self
            .detached_windows
            .get(&window)
            .and_then(|detached| detached.viewports.get(&pane.id))
            .map_or(pane.scroll, |viewport| viewport.scroll);
        // An alternate screen has no transcript viewport. It is always painted
        // at the tail, while reducer-side vetoes keep drag/jump from changing
        // the retained primary-screen position and WheelRoute sends wheel input
        // to this exact pane's PTY.
        let painted_position = if pane.alt_screen {
            scrollback::ScrollPosition::pinned_to_tail()
        } else {
            retained
        };
        let (win_start, win_end) = self
            .input_origin(&session.id, pane_id)
            .and_then(|origin| self.scoped_grid_window(&origin))
            .unwrap_or_else(|| {
                let start = painted_position.top_row(scroll_geom);
                (start, (start + rows).min(pane.transcript.len()))
            });
        let highlights = self
            .window_inputs
            .get(&window)
            .filter(|state| state.find_owner.as_deref() == Some(pane_id))
            .map_or_else(Vec::new, |state| {
                Self::find_matches_for_state(&state.find, pane, win_start, win_end)
            });
        let focused = session
            .panes
            .get(session.active_pane)
            .is_some_and(|active| active.id == pane_id);
        let (grid_rows, grid_cols) = self
            .pty_geometry_for(pane_id, text_size)
            .unwrap_or((rows as u16, geom.cols_for(text_w)));
        let _ = text_h;
        let screen = self
            .screens
            .get(&(Some(window), pane_id.to_owned()), Default::default());
        let fit_pane = pane_id.to_owned();
        // A detached window writes its own screen (its scroll position is its
        // own); the reader writes only main-window panes.
        if let Some(pty) = self.ptys.get(pane_id) {
            pty.set_live(None, 0);
        }
        let grid = terminal_view::terminal_grid(
            &screen,
            (&[], 0),
            &pane.transcript[win_start..win_end],
            &self.settings,
            self.terminal_family(),
            (grid_cols, grid_rows),
            terminal_view::Overlays {
                focused,
                cursor_visible: !pane.cursor_hidden,
                selection: self.selection_for(pane_id),
                window_start_row: win_start,
                matches: &highlights,
            },
            terminal_view::Routes {
                pointer: Some(grid_view::GridPointerHooks {
                    press: Message::GridCellPressed,
                    drag: Message::GridCellDragged,
                    release: Message::GridCellReleased,
                    pane: None,
                }),
                wheel: None,
                fit: Box::new(move |cols, rows| Message::TerminalFit {
                    pane: fit_pane.clone(),
                    cols,
                    rows,
                }),
                ignored: Message::TerminalViewIgnored,
            },
            inset,
            true,
        );
        let pane_for_drag = pane.id.clone();
        let pane_for_wheel = pane.id.clone();
        let cell_height = geom.cell_height;
        let chrome: Element<'a, Message> = enz_offset_scrollbar(grid)
            .content_len(scroll_geom.content_height())
            .viewport_len(scroll_geom.viewport_height())
            .offset(painted_position.scroll_top_px(scroll_geom))
            .backdrop(grid_view::terminal_background())
            .on_scroll_to(move |px| Message::DetachedScrollTranscriptToRow {
                window,
                pane: pane_for_drag.clone(),
                row: scrollback::top_row_for_px(px, scroll_geom),
            })
            .on_wheel(move |delta| Message::DetachedTranscriptWheel {
                window,
                pane: pane_for_wheel.clone(),
                rows: scrollback::wheel_rows(delta, cell_height),
            })
            .into();
        // The detached window shares the docked composition: the jump affordance
        // swaps for an inert layer instead of being mounted and unmounted, so a
        // live scrollbar drag keeps its retained origin as the affordance
        // appears.
        let pane_for_jump = pane.id.clone();
        let jump =
            (!pane.alt_screen && !painted_position.resolves_to_end(scroll_geom)).then(|| {
                enz_button(
                    crate::i18n::label("Jump to latest"),
                    Message::DetachedJumpToLatest {
                        window,
                        pane: pane_for_jump,
                    },
                )
                .kind(ButtonKind::Secondary)
                .icon("arrow-down")
                .aria_label(crate::i18n::label("Jump to latest output"))
                .into_element()
            });
        let surface = iced::widget::mouse_area(self.transcript_surface(chrome, jump))
            .on_press(Message::FocusPane(pane_id.to_owned()))
            .into();
        self.input_surface(session, pane, win_start, surface)
    }
}
