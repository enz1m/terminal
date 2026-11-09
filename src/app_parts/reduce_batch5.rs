//! Terminal reducer batch 5: grid-cell arms.

use super::*;

impl App {
    pub(crate) fn reduce5_grid_cell_pressed_0(
        &mut self,
        target: grid_view::GridPointerTarget,
    ) -> Task<Message> {
        // A press focuses the leaf it landed in before anything reads
        // the active pane. `text_grid` captures the press it published,
        // so the surrounding `mouse_area`'s `FocusPane` never fires for
        // a press on text.
        let Some((pane_index, pane_id)) = self.pointer_pane(target) else {
            return Task::none();
        };
        self.focus_pane_index(pane_index);
        let cell = self.transcript_cell(target);

        // Modern terminals reserve Ctrl+click for opening the
        // detected link under the pointer. Do this before starting a
        // selection gesture, so releasing the same click cannot turn
        // the URI into selected text. Plain click/drag remains the
        // selection path below and therefore still reaches the PTY's
        // familiar text-selection model.
        if self.mods.control()
            && let Some(target) = self.link_target_at(cell)
        {
            self.grid_pointer = GridPointerProbe::default();
            let _ = self.selections.remove(&pane_id);
            self.pane_feedback = None;
            return self.begin_owned_link(target);
        }

        // An application tracking the mouse OWNS a plain (non-
        // Shift) press. It is REPORTED to the child and starts no local
        // selection. Shift is the escape hatch that always selects
        // locally, so a user in `vim` never loses copy-to-clipboard.
        let proto = self.active_mouse_protocol();
        if proto.tracks() && !self.mods.shift() {
            let (col, row) = Self::report_cell(target);
            self.mouse_last_cell = (col, row);
            if let Some(bytes) =
                proto.encode(mouse::PointerEvent::Press(mouse::Button::Left), col, row)
            {
                self.grid_pointer = GridPointerProbe::default();
                let _ = self.selections.remove(&pane_id);
                self.type_active_pty(&bytes);
                return Task::none();
            }
        }

        // Feed the click detector for word/line selection.
        // Use transcript cell coordinates as a proxy position —
        // clicks on the same cell are at distance 0, always within
        // the deadband.
        let proxy_pos = iced::Point::new(cell.1 as f32, cell.0 as f32);
        let report = self
            .click_detector
            .press(std::time::Instant::now(), proxy_pos);

        // Every press begins a drag gesture (`holding: true`).
        // Double/triple clicks pre-fill the selection with a word
        // or line, but the gesture stays live so a subsequent drag
        // can extend it.
        self.grid_pointer = GridPointerProbe {
            anchor: Some(cell),
            focus: Some(cell),
            holding: true,
            expanded_at: (report.count != ClickCount::Single).then_some(cell),
            focus_past_line_end: target.edge == enzui_widgets::text_grid::TextGridEdge::AfterLast,
            // The gesture belongs to the pressed pane for its whole
            // life, so a drag that crosses the seam keeps extending the
            // pressed pane's range and never the neighbour's.
            pane: Some(pane_id.clone()),
        };
        let _ = self.selections.remove(&pane_id);

        match report.count {
            ClickCount::Double => {
                // Double-click: word selection.
                if let Some(pane) = self.active_pane()
                    && let Some(sel) =
                        selection::word_selection_at(&pane.transcript, cell.0, cell.1)
                {
                    self.grid_pointer.anchor = Some((sel.start_row, sel.start_col));
                    self.grid_pointer.focus = Some((sel.end_row, sel.end_col));
                    self.grid_pointer.focus_past_line_end = sel.end_past_line;
                    self.set_selection(Some(sel));
                }
            }
            ClickCount::Triple => {
                // Triple-click: line selection.
                if let Some(sel) = self.active_pane().and_then(|_| {
                    selection::line_selection_at(
                        &self.sessions[self.active].panes[self.sessions[self.active].active_pane]
                            .transcript,
                        cell.0,
                    )
                }) {
                    self.grid_pointer.anchor = Some((sel.start_row, sel.start_col));
                    self.grid_pointer.focus = Some((sel.end_row, sel.end_col));
                    self.grid_pointer.focus_past_line_end = sel.end_past_line;
                    self.set_selection(Some(sel));
                }
            }
            ClickCount::Single => {}
        }

        Task::none()
    }
    pub(crate) fn reduce5_grid_cell_dragged_1(
        &mut self,
        target: grid_view::GridPointerTarget,
    ) -> Task<Message> {
        // Drop reports from a pane that does not own the live gesture.
        if !self.pointer_report_owns_gesture(target) {
            return Task::none();
        }
        // Under mouse tracking (mode 1002/1003) a plain drag is
        // reported to the child, coalesced to one report per cell so an
        // uncoalesced move stream cannot flood the PTY. Shift still
        // selects locally.
        let proto = self.active_mouse_protocol();
        if proto.tracks() && !self.mods.shift() {
            let (col, row) = Self::report_cell(target);
            if (col, row) != self.mouse_last_cell
                && let Some(bytes) =
                    proto.encode(mouse::PointerEvent::Drag(mouse::Button::Left), col, row)
            {
                self.mouse_last_cell = (col, row);
                self.type_active_pty(&bytes);
            }
            return Task::none();
        }
        // Only a live gesture moves the focus: a stray move must not
        // rewrite a finished gesture's coordinates.
        let cell = self.transcript_cell(target);
        if self.grid_pointer.holding && self.grid_pointer.expanded_at != Some(cell) {
            self.grid_pointer.expanded_at = None;
            self.grid_pointer.focus = Some(cell);
            self.grid_pointer.focus_past_line_end =
                target.edge == enzui_widgets::text_grid::TextGridEdge::AfterLast;
            // Build the selection as the drag progresses.
            self.rebuild_selection();
        }

        Task::none()
    }
    pub(crate) fn reduce5_grid_cell_released_2(
        &mut self,
        target: grid_view::GridPointerTarget,
    ) -> Task<Message> {
        if !self.pointer_report_owns_gesture(target) {
            return Task::none();
        }
        // Mirror the press — a tracking application is told the
        // button came up, and no local selection is finalised.
        let proto = self.active_mouse_protocol();
        if proto.tracks() && !self.mods.shift() {
            let (col, row) = Self::report_cell(target);
            self.mouse_last_cell = (col, row);
            if let Some(bytes) =
                proto.encode(mouse::PointerEvent::Release(mouse::Button::Left), col, row)
            {
                self.type_active_pty(&bytes);
            }
            return Task::none();
        }
        let cell = self.transcript_cell(target);
        if self.grid_pointer.holding && self.grid_pointer.expanded_at != Some(cell) {
            self.grid_pointer.expanded_at = None;
            self.grid_pointer.focus = Some(cell);
            self.grid_pointer.focus_past_line_end =
                target.edge == enzui_widgets::text_grid::TextGridEdge::AfterLast;
        }
        self.grid_pointer.holding = false;
        // Finalise the selection.
        self.rebuild_selection();

        Task::none()
    }
}
