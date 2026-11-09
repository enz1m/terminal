//! Terminal surface composition.
//!
//! This module owns the unsplit terminal path; split/group composition remains
//! beside the pane-group model. Keeping the unsplit path isolated also makes
//! its no-extra-chrome invariant explicit.

use super::*;

impl App {
    /// Preserve the original one-pane terminal geometry byte-for-byte. Split
    /// chrome exists only when there is a split, so a normal terminal does not
    /// gain a header, border, inset, or pointer-coordinate offset.
    pub(super) fn single_terminal_pane(&self) -> Element<'_, Message> {
        #[cfg(test)]
        model::perf::note_terminal_surface();
        let Some(pane) = self.active_pane() else {
            return self.empty_terminal();
        };
        let geom = self.geometry();
        let full_lines = &pane.transcript;
        let (win_start, win_end) = self.grid_window(full_lines.len(), geom);
        let pane_width = self.terminal_text_width();
        // Precompute find-match highlights in cell space for this
        // window; the grid paints them as translucent cell backgrounds so a
        // user can see WHERE their matches are, not just "n of m".
        let match_highlights = self.find_matches_for_window(pane, win_start, win_end);
        let text = iced::Size::new(pane_width, self.terminal_text_height());
        let (rows, cols) = self
            .pty_geometry_for(&pane.id, text)
            .unwrap_or((geom.rows_for(text.height), geom.cols_for(pane_width)));
        let screen = self
            .screens
            .get(&(None, pane.id.clone()), Default::default());
        let fit_pane = pane.id.clone();
        let scroll_geom = self.scroll_geometry(full_lines.len(), geom);
        let painted_position = scrollback::ScrollPosition::from_top_row(win_start, scroll_geom);
        let selection = self.selection();
        // The rows before the window, shown under the titlebar.
        let rows_above = self.under_titlebar_rows();
        let above = &full_lines[win_start.saturating_sub(usize::from(rows_above))..win_start];
        let sync = self.live(
            pane,
            &screen,
            rows_above,
            pane.alt_screen || painted_position.resolves_to_end(scroll_geom),
            selection.is_none() && match_highlights.is_empty(),
            true,
        );
        let grid = terminal_view::terminal_grid(
            &screen,
            (above, rows_above),
            &full_lines[win_start..win_end],
            &self.settings,
            self.terminal_family(),
            (cols, rows),
            terminal_view::Overlays {
                focused: true,
                cursor_visible: !pane.cursor_hidden,
                selection,
                window_start_row: win_start,
                matches: &match_highlights,
            },
            terminal_view::Routes {
                pointer: Some(grid_view::GridPointerHooks {
                    press: Message::GridCellPressed,
                    drag: Message::GridCellDragged,
                    release: Message::GridCellReleased,
                    // Unscoped: there is exactly one pane, so a report names no
                    // leaf and the reducer resolves it against the active pane.
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
            // The ordinary shell's inner inset, applied inside
            // the text container so the scrollbar lane below keeps the pane
            // edge. Zero on the alternate screen, so a TUI is full bleed.
            self.terminal_inset(),
            sync,
        );
        // The framework's overlay-scrollbar-over-caller-owned-offset widget
        // (`enz_offset_scrollbar`). The row↔px conversion lives here
        // (`top_row_for_px` / `wheel_rows`) so the framework widget stays
        // row-blind and the reducer still consumes the row-typed messages
        // `ScrollTranscriptToRow` / `TranscriptWheel`.
        let cell_height = geom.cell_height;
        let chrome: Element<'_, Message> = enz_offset_scrollbar(grid)
            .content_len(scroll_geom.content_height())
            .viewport_len(scroll_geom.viewport_height())
            .offset(painted_position.scroll_top_px(scroll_geom))
            .backdrop(grid_view::terminal_background())
            .on_scroll_to(move |px| {
                Message::ScrollTranscriptToRow(scrollback::top_row_for_px(px, scroll_geom))
            })
            .on_wheel(move |delta| {
                Message::TranscriptWheel(scrollback::wheel_rows(delta, cell_height))
            })
            .into();
        // The affordance layer mounts whether or not it carries the button, so
        // the chrome keeps its widget identity (and a live thumb drag) across
        // the frame it appears in (see `App::transcript_surface`).
        let surface = self.transcript_surface(
            chrome,
            (!painted_position.resolves_to_end(scroll_geom))
                .then(|| self.jump_to_latest_button(Message::JumpToLatest)),
        );
        self.input_surface(&self.sessions[self.active], pane, win_start, surface)
    }
}
