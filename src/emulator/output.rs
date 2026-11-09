use super::*;

impl Screen {
    /// Rows a command may have produced, oldest first, each with a stamp that
    /// stays the same while the row is not rewritten: its place in the whole
    /// output, which a row keeps when it scrolls off.
    pub(crate) fn primary_output_rows(&self) -> Vec<(usize, TranscriptRow)> {
        let (cols, rows) = self.dims();
        let mut out: Vec<_> = self
            .scrollback
            .iter()
            .map(|row| (row.0.line as usize, row.materialize()))
            .collect();
        let stamp = |index: usize| (self.scrollback_counter + 1 + index as u64) as usize;
        let id = |index: usize| format!("retained-output-tw-row-{index}");
        if self.is_alt_screen() {
            // The primary screen is hidden; read the rows kept at the switch.
            if let Some(primary) = &self.primary {
                out.extend(primary.iter().enumerate().map(|(index, cells)| {
                    (
                        stamp(index),
                        TranscriptRow::new(project_row(
                            cells.views(),
                            &id(index),
                            false,
                            0,
                            cols,
                            self.link_generation,
                        )),
                    )
                }));
            }
            return out;
        }
        out.extend((0..rows).map(|index| {
            (
                stamp(index),
                TranscriptRow::new(project_cells(
                    self.row_cells(index),
                    &id(index),
                    false,
                    0,
                    cols,
                    self.link_generation,
                )),
            )
        }));
        out
    }

    pub(super) fn reconcile_pinned_output(&mut self) {
        if !self.pinned_capture.tracks_output() {
            return;
        }
        let mut capture = std::mem::take(&mut self.pinned_capture);
        capture.reconcile(self, self.max_scrollback);
        self.pinned_capture = capture;
    }

    /// A shadow screen that starts blank with this screen's geometry, cursor
    /// and scrollback numbering; a running command's output is replayed into
    /// it, so rows it touched can be told from rows it did not.
    pub(crate) fn output_capture_surface(&self, cap: usize) -> Self {
        let mut output = Self::new(Arc::new(AtomicU64::new(0)));
        let (cols, rows) = self.dims();
        output.resize(rows, cols);
        output.set_max_scrollback(cap);
        output.scrollback_counter = self.scrollback_counter;
        let (x, y) = self.cursor();
        output.apply_capture_bytes(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
        output.take_effects();
        output
    }

    /// Feeds a shadow screen, without effects or shell integration.
    pub(crate) fn apply_capture_bytes(&mut self, bytes: &[u8]) {
        self.parse(bytes);
        self.take_scrolled_rows();
        self.events.take();
    }

    /// Feed one chunk of PTY bytes. Called from the reader thread; returns
    /// after all bytes are consumed.
    ///
    /// A string sequence (OSC, DCS, APC, SOS, PM) whose payload outgrows
    /// [`escape_policy::MAX_STRING_SEQUENCE_BYTES`] is abandoned: the parser
    /// holding its partial buffer is replaced, the rest of that sequence is
    /// dropped, and the pane says so on screen.
    pub(crate) fn process(&mut self, bytes: &[u8]) {
        let segments = self.sequence_guard.split(bytes);
        for segment in segments {
            match segment {
                Segment::Feed(range) => self.feed(&bytes[range]),
                Segment::Discard { started: true } => self.abandon_oversized_sequence(),
                Segment::Discard { started: false } => {}
            }
        }
        self.dirty.fetch_add(1, Ordering::SeqCst);
    }

    pub(super) fn abandon_oversized_sequence(&mut self) {
        self.parser = Processor::new();
        self.scanner.reset();
        #[cfg(test)]
        {
            self.pending_effects.discarded_sequences =
                self.pending_effects.discarded_sequences.saturating_add(1);
        }
        self.feed(OVERSIZED_SEQUENCE_NOTICE);
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        let found = self.scanner.scan(bytes);
        let mut start = 0;
        for (end, scanned) in found {
            self.advance(&bytes[start..end]);
            start = end;
            self.handle_scanned(scanned);
        }
        self.advance(&bytes[start..]);
        self.reconcile_pinned_output();
    }

    /// Parses `bytes` into the emulator (and a running command's shadow), then
    /// settles everything the emulator reported.
    pub(super) fn advance(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if self.pinned_capture.tracks_output() {
            self.pinned_capture.feed(bytes);
        }
        self.parse(bytes);
        self.take_scrolled_rows();
        self.settle_events();
        let alt = self.is_alt_screen();
        if alt != self.alt_screen {
            self.alt_screen = alt;
            // A link is destination authority: it crosses no screen swap.
            self.retire_links();
            self.pending_effects.coordinates_invalidated = true;
            if alt {
                self.pinned_capture.alt_screen_entered();
            } else {
                self.primary = None;
            }
        }
    }

    pub(super) fn settle_events(&mut self) {
        for event in self.events.take() {
            match event {
                Event::Title(title) => {
                    // A refused title leaves the pane on its previous one.
                    if let Some(title) = escape_policy::accepted_title(&title) {
                        self.pending_effects.title = Some(title);
                    }
                }
                Event::Bell => {
                    self.pending_effects.bell_count =
                        self.pending_effects.bell_count.saturating_add(1);
                }
                Event::PtyWrite(reply) => {
                    if let Some(reply) = admitted_reply(&reply) {
                        self.queue_reply(reply);
                    }
                }
                // Clipboard, palette and size queries are refused as policy:
                // terminal output may not read the desktop or the window.
                _ => {}
            }
        }
    }

    pub(super) fn handle_scanned(&mut self, scanned: scan::Scanned) {
        use crate::shell_history::ShellEvent;
        match scanned {
            scan::Scanned::Cwd(cwd) if !self.history_disabled => {
                if let Some(cwd) = escape_policy::accepted_cwd_uri(&cwd) {
                    self.pending_effects.cwd = Some(cwd);
                    self.pending_effects.generated_cwd = None;
                }
            }
            scan::Scanned::Cwd(_) => {}
            scan::Scanned::CommandStatus(status) => {
                let mut capture = std::mem::take(&mut self.pinned_capture);
                capture.command_status(status, self, self.max_scrollback);
                self.pinned_capture = capture;
                if self.history_disabled {
                    return;
                }
                #[cfg(test)]
                {
                    self.pending_effects.command_finished = Some(status);
                }
                let key = self.active_command.take();
                // Idle redraws must not evict an accepted command's completion.
                if key.is_some() || self.pending_effects.shell_events.is_empty() {
                    self.shell_event(ShellEvent::Finished { key, status });
                }
            }
            scan::Scanned::Rxvt(fields) => {
                let mut capture = std::mem::take(&mut self.pinned_capture);
                capture.rxvt(&fields, self, self.max_scrollback);
                self.pinned_capture = capture;
                if crate::shell_history::history_disabled_from_osc(&fields) {
                    self.disable_shell_integration();
                } else if !self.history_disabled
                    && let Some(cwd) = crate::shell_history::cwd::from_osc(&fields)
                {
                    self.pending_effects.generated_cwd = Some(cwd);
                    self.pending_effects.cwd = None;
                } else if !self.history_disabled
                    && let Some(command) = crate::shell_history::command_from_osc(&fields)
                {
                    let Some(key) = crate::shell_history::next_command_key() else {
                        self.disable_shell_integration();
                        return;
                    };
                    self.active_command = Some(key);
                    #[cfg(test)]
                    {
                        let commands = &mut self.pending_effects.commands_started;
                        if commands.len() >= crate::model::IN_APP_HISTORY_MAX {
                            commands.pop_front();
                        }
                        commands.push_back(command.clone());
                        self.pending_effects.command_finished = None;
                    }
                    self.shell_event(ShellEvent::Started { key, command });
                }
            }
            scan::Scanned::ClearScrollback => {
                self.pinned_capture.erase_scrollback();
                self.scrollback.clear();
                self.pending_effects.coordinates_invalidated = true;
            }
            // A reset screen must not keep handing later output, or anything
            // it already printed, an earlier program's destination.
            scan::Scanned::FullReset => self.retire_links(),
        }
    }

    /// Resize the emulator to a new PTY grid. Rows pushed off the top by a
    /// smaller screen enter the scrollback.
    pub(crate) fn resize(&mut self, rows: usize, cols: usize) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if self.dims() == (cols, rows) {
            return;
        }
        self.pinned_capture.resized();
        // A shorter screen keeps the cursor's row: the rows above it that no
        // longer fit scroll off into the scrollback.
        let (_, cursor_row) = self.cursor();
        let off = (cursor_row + 1).saturating_sub(rows);
        if off > 0 && !self.is_alt_screen() {
            for row in 0..off {
                let cells = RetainedRow::copy(self.row_cells(row));
                self.scrolled.push(cells);
            }
        }
        self.term.resize(Size { cols, rows });
        self.region = (0, rows);
        self.take_scrolled_rows();
        self.live.clear();
        self.reconcile_pinned_output();
        self.pending_effects.coordinates_invalidated = true;
        self.dirty.fetch_add(1, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn buffer_dimensions(&self) -> ((usize, usize), Option<(usize, usize)>) {
        let dims = self.dims();
        (dims, self.is_alt_screen().then_some(dims))
    }

    /// Projects retained primary history followed by the visible rows; the
    /// alternate screen exposes only its visible rows. Retained rows share
    /// immutable projections until eviction; a visible row is projected again
    /// only when the emulator changed it or the cursor entered or left it.
    /// Cursor columns are terminal cells, not bytes.
    pub(crate) fn project(&mut self, session_id: &str) -> Vec<TranscriptRow> {
        let (cols, rows) = self.dims();
        let (cursor_col, cursor_row) = self.cursor();
        if self.live.len() != rows || self.live_generation != self.link_generation {
            self.live.clear();
            self.live.resize_with(rows, || None);
            self.live_generation = self.link_generation;
        }
        // The emulator reports the lines it touched (and always the cursor's);
        // a touched row is projected again only if its cells really changed.
        let damaged: Option<Vec<usize>> = match self.term.damage() {
            TermDamage::Full => None,
            TermDamage::Partial(lines) => Some(lines.map(|d| d.line).collect()),
        };
        self.term.reset_damage();
        let check: Vec<usize> = damaged.unwrap_or_else(|| (0..rows).collect());
        for line in check {
            let print = fingerprint(self.row_cells(line.min(rows.saturating_sub(1))));
            if let Some(slot) = self.live.get_mut(line)
                && slot.as_ref().is_some_and(|row| row.cells != print)
            {
                *slot = None;
            }
        }
        let alt = self.is_alt_screen();
        let mut out: Vec<TranscriptRow> =
            Vec::with_capacity(if alt { 0 } else { self.scrollback.len() } + rows);
        if !alt {
            out.extend(self.scrollback.iter().map(ScrollbackRow::materialize));
        }
        for r in 0..rows {
            let cursor = (r == cursor_row).then_some(cursor_col);
            let fresh = matches!(&self.live[r], Some(row) if row.cursor == cursor && row.session == session_id);
            if !fresh {
                let cells = fingerprint(self.row_cells(r));
                let projection = TranscriptRow::new(project_cells(
                    self.row_cells(r),
                    &format!("{session_id}-tw-row-{r}"),
                    cursor.is_some(),
                    cursor.unwrap_or(0),
                    cols,
                    self.link_generation,
                ));
                self.live[r] = Some(LiveRow {
                    session: session_id.to_owned(),
                    cursor,
                    cells,
                    projection,
                });
            }
            out.push(self.live[r].as_ref().expect("projected").projection.clone());
        }
        out
    }

    /// A from-scratch projection, for the equivalence tests.
    #[cfg(test)]
    pub(crate) fn project_deep_reference(&self, session_id: &str) -> Vec<TranscriptLine> {
        let (cols, rows) = self.dims();
        let (cursor_col, cursor_row) = self.cursor();
        let mut out = Vec::new();
        if !self.is_alt_screen() {
            out.extend(
                self.scrollback
                    .iter()
                    .map(ScrollbackRow::project_eager_reference),
            );
        }
        for r in 0..rows {
            let is_prompt = r == cursor_row;
            out.push(project_cells(
                self.row_cells(r),
                &format!("{session_id}-tw-row-{r}"),
                is_prompt,
                if is_prompt { cursor_col } else { 0 },
                cols,
                self.link_generation,
            ));
        }
        out
    }
}
