use enzui_core::form::SecretText;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::emulator::Screen;
use crate::model::{self, TranscriptRow};

pub(crate) const RESTORED_LABEL: &str = "Restored output — process not live";
pub(crate) const MAX_ROWS: usize = 20_000;
pub(crate) const MAX_BYTES: usize = 256 * 1024;
pub(crate) const MAX_COMMANDS: usize = 128;
pub(crate) const MAX_ROW_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputRow {
    pub(crate) text: SecretText,
    pub(crate) columns: usize,
}

impl Hash for OutputRow {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.expose_secret().hash(state);
        self.columns.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcceptedCommand {
    pub(crate) text: SecretText,
    pub(crate) exit: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PinnedOutput {
    rows: VecDeque<OutputRow>,
    commands: VecDeque<AcceptedCommand>,
}

pub(crate) fn plain_text(text: &str) -> bool {
    !text.chars().any(|ch| {
        ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    })
}

impl PinnedOutput {
    pub(crate) fn rows(&self) -> impl Iterator<Item = &OutputRow> {
        self.rows.iter()
    }
    pub(crate) fn commands(&self) -> impl Iterator<Item = &AcceptedCommand> {
        self.commands.iter()
    }
    fn bytes(&self) -> usize {
        self.rows
            .iter()
            .map(|row| row.text.expose_secret().len() + 16)
            .sum::<usize>()
            + self
                .commands
                .iter()
                .map(|command| command.text.expose_secret().len() + 16)
                .sum::<usize>()
    }
    pub(crate) fn bounded(&self, cap: usize) -> Self {
        let mut out = self.clone();
        out.trim(cap);
        out
    }
    fn trim(&mut self, cap: usize) {
        while self.rows.len() > cap.min(MAX_ROWS) {
            self.rows.pop_front();
        }
        while self.commands.len() > cap.min(MAX_COMMANDS) {
            self.commands.pop_front();
        }
        while self.bytes() > MAX_BYTES {
            if self.rows.pop_front().is_none() {
                self.commands.pop_front();
            }
        }
    }
    pub(crate) fn read_row(&mut self, columns: usize, text: String) -> Result<(), String> {
        let owned = SecretText::new(text);
        let text = owned.expose_secret();
        if columns == 0
            || columns > 1000
            || text.len() > MAX_ROW_BYTES
            || !plain_text(text)
            || termwiz::cell::unicode_column_width(text, None) > columns
            || self.rows.len() >= MAX_ROWS
        {
            return Err("invalid retained terminal row".into());
        }
        if self.bytes() + text.len() + 16 > MAX_BYTES {
            return Err("retained output exceeds byte budget".into());
        }
        self.rows.push_back(OutputRow {
            text: owned,
            columns,
        });
        Ok(())
    }
    pub(crate) fn read_command(&mut self, text: String, exit: i32) -> Result<(), String> {
        let owned = SecretText::new(text);
        let text = owned.expose_secret();
        if text.trim().is_empty()
            || text.len() > crate::shell_history::MAX_COMMAND_BYTES
            || !plain_text(text)
            || self.commands.len() >= MAX_COMMANDS
        {
            return Err("invalid retained command".into());
        }
        if self.bytes() + text.len() + 16 > MAX_BYTES {
            return Err("retained commands exceed byte budget".into());
        }
        self.commands
            .push_back(AcceptedCommand { text: owned, exit });
        Ok(())
    }
    pub(crate) fn render(&self, pane: &str) -> Vec<TranscriptRow> {
        self.rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let mut line = model::TranscriptLine::monochrome(
                    format!("restored-{pane}-{i}"),
                    row.text.expose_secret().to_owned(),
                    model::LineTone::Default,
                    false,
                );
                while line.cells.len() < row.columns {
                    line.cells.push(model::TranscriptCell {
                        text: enzui_core::grapheme_cell::InlineGrapheme::new(" "),
                        tone: model::LineTone::Default,
                        bold: false,
                        blank: true,
                        fg: model::CellInk::Default,
                        bg: model::CellInk::Default,
                        attrs: 0,
                    });
                }
                TranscriptRow::new(line)
            })
            .collect()
    }
    pub(crate) fn history(&self) -> impl Iterator<Item = model::HistoryItem> + '_ {
        self.commands.iter().map(|command| model::HistoryItem {
            id: String::new(),
            command: command.text.expose_secret().to_owned(),
            when: "Restored".into(),
            exit_code: Some(command.exit),
        })
    }
}

#[derive(Default)]
enum Phase {
    #[default]
    Unknown,
    Prompt,
    Submitted,
    EmptySubmitted,
    Running,
}

#[derive(Default)]
pub(crate) struct OutputCapture {
    supported: bool,
    enabled: bool,
    blocked: bool,
    phase: Phase,
    prompt_touched: bool,
    candidate: Option<(SecretText, Box<Screen>)>,
    settled: Arc<PinnedOutput>,
    approved: HashSet<(usize, OutputRow)>,
}

impl OutputCapture {
    pub(crate) fn tracks_output(&self) -> bool {
        self.supported
    }

    pub(crate) fn supported(&mut self, supported: bool) {
        self.supported = supported;
    }
    pub(crate) fn enable(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.candidate = None;
            self.settled = Arc::default();
            self.approved.clear();
        }
    }
    pub(crate) fn exclude_input(&mut self) {
        self.blocked = true;
        self.candidate = None;
    }
    pub(crate) fn disable_history(&mut self) {
        self.supported(false);
        self.exclude_input();
        self.clear_retained();
    }
    pub(crate) fn input(&mut self, bytes: &[u8], shell_owns_input: bool) {
        if bytes.is_empty() {
            return;
        }
        // An editor may consume cancellation without publishing another prompt boundary.
        if bytes.iter().any(|byte| matches!(*byte, 3 | 4)) {
            self.exclude_input();
        }
        // Shell line editors can disable echo while they still own prompt input.
        if !shell_owns_input || !matches!(self.phase, Phase::Prompt) {
            self.exclude_input();
            return;
        }
        if let Some(index) = bytes
            .iter()
            .position(|byte| matches!(*byte, b'\r' | b'\n' | 3 | 4))
        {
            self.phase = if self.prompt_touched || bytes[..index].iter().any(|byte| *byte != b' ') {
                Phase::Submitted
            } else {
                Phase::EmptySubmitted
            };
            if index + 1 != bytes.len() {
                self.exclude_input();
            }
        } else if bytes.iter().any(|byte| *byte != b' ') {
            self.prompt_touched = true;
        }
    }
    pub(crate) fn snapshot(&self) -> Arc<PinnedOutput> {
        self.settled.clone()
    }
    pub(crate) fn trim(&mut self, cap: usize) {
        Arc::make_mut(&mut self.settled).trim(cap);
        if let Some((_, surface)) = self.candidate.as_mut() {
            surface.set_max_scrollback(cap);
        }
    }
    pub(crate) fn clear_retained(&mut self) {
        self.settled = Arc::default();
        self.approved.clear();
        self.candidate = None;
    }

    pub(crate) fn resized(&mut self) {
        self.candidate = None;
    }

    pub(crate) fn reconcile(&mut self, live: &Screen, cap: usize) {
        if self.approved.is_empty() {
            return;
        }
        // Text equality cannot preserve provenance after a row is edited or its coordinate is reused.
        let retained: VecDeque<_> = live
            .primary_output_rows()
            .into_iter()
            .filter_map(|(stamp, row)| {
                let output = OutputRow {
                    text: SecretText::new(row.text.clone()),
                    columns: row.cells.len(),
                };
                self.approved
                    .contains(&(stamp, output.clone()))
                    .then_some((stamp, output))
            })
            .collect();
        let mut next = (*self.settled).clone();
        next.rows = retained.iter().map(|(_, row)| row.clone()).collect();
        next.trim(cap);
        let keep_from = retained.len().saturating_sub(next.rows.len());
        self.approved = retained.into_iter().skip(keep_from).collect();
        if next != *self.settled {
            self.settled = Arc::new(next);
        }
    }

    #[cfg(test)]
    pub(crate) fn paused(&self) -> bool {
        !self.supported || self.blocked
    }
    /// The scrollback was erased (`CSI 3 J`): nothing retained survives.
    pub(crate) fn erase_scrollback(&mut self) {
        self.settled = Arc::default();
        self.approved.clear();
        self.candidate = None;
    }

    /// A program entered the alternate screen.
    pub(crate) fn alt_screen_entered(&mut self) {
        // Pre-prompt capability probes have no eligible command output.
        if !matches!(self.phase, Phase::Unknown) {
            self.exclude_input();
        }
    }

    /// Output bytes, replayed into a running command's shadow screen.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        if let Some((_, surface)) = self.candidate.as_mut() {
            surface.apply_capture_bytes(bytes);
        }
    }

    /// OSC 777 from the shell: a command was accepted, or history is off.
    pub(crate) fn rxvt(&mut self, fields: &[String], live: &Screen, cap: usize) {
        if crate::shell_history::history_disabled_from_osc(fields) {
            self.disable_history();
        } else if let Some(command) = crate::shell_history::command_from_osc(fields) {
            let command = SecretText::new(command);
            if self.supported
                && !self.blocked
                && matches!(self.phase, Phase::Submitted)
                && plain_text(command.expose_secret())
                && !live.is_alt_screen()
            {
                self.phase = Phase::Running;
                if self.enabled {
                    self.candidate = Some((command, Box::new(live.output_capture_surface(cap))));
                }
            } else {
                self.exclude_input();
            }
        }
    }

    /// OSC 133 D: the running command finished with `status`.
    pub(crate) fn command_status(&mut self, status: i32, live: &Screen, cap: usize) {
        if let Some((command, mut surface)) = self.candidate.take()
            && self.supported
            && self.enabled
            && !self.blocked
            && matches!(self.phase, Phase::Running)
            && !live.is_alt_screen()
        {
            let mut output = (*self.settled).clone();
            // The blank shadow excludes input echoes and untouched cells from before command acceptance.
            let projected = surface.project("retained-output");
            let candidate: HashMap<_, _> =
                projected.iter().map(|row| (row.id.as_str(), row)).collect();
            for (stamp, row) in live.primary_output_rows() {
                if row.text.len() <= MAX_ROW_BYTES
                    && plain_text(&row.text)
                    && row.cells.len() <= 1000
                    && candidate.get(row.id.as_str()).is_some_and(|other| {
                        other.text == row.text && other.cells.len() == row.cells.len()
                    })
                {
                    self.approved.insert((
                        stamp,
                        OutputRow {
                            text: SecretText::new(row.text.clone()),
                            columns: row.cells.len(),
                        },
                    ));
                }
            }
            output.commands.push_back(AcceptedCommand {
                text: command,
                exit: status,
            });
            output.trim(cap);
            self.settled = Arc::new(output);
        }
        if matches!(self.phase, Phase::Submitted) {
            self.exclude_input();
        }
        self.phase = Phase::Prompt;
        self.prompt_touched = false;
    }
}
