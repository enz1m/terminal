//! Terminal text selection model over transcript coordinates.
//!
//! ## What is selected between two points?
//!
//! **Character-stream** (everything from anchor to focus, wrapping at line
//! ends, like xterm / gnome-terminal). This is the default every user expects.
//!
//! ## What happens when the grid reflows or new output arrives?
//!
//! * **New output below the selection** does NOT invalidate it. Coordinates
//!   are transcript-absolute `i64` (not viewport-relative), which is the whole
//!   reason `App::transcript_cell` is unclamped — an absolute row keeps its
//!   meaning after new lines push the viewport down.
//! * **A reflow** (terminal resize that changes the column count) **clears**
//!   the selection. Column positions lose their meaning when the PTY re-wraps,
//!   and attempting to adjust them would produce a selection that looks correct
//!   and copies wrong text. This matches xterm, gnome-terminal and kitty.
//!
//! ## Word and line selection
//!
//! [`word_selection_at`] and [`line_selection_at`] are pure over transcript
//! coordinates. `App` feeds presses through the framework's shared
//! [`enzui_core::click::ClickDetector`]: double-click selects a word and
//! triple-click selects a line.

use crate::model::TranscriptRow;

pub(crate) fn grapheme_columns(
    cells: &[crate::model::TranscriptCell],
) -> impl Iterator<Item = (std::ops::Range<usize>, &str)> {
    let mut col = 0;
    std::iter::from_fn(move || {
        let text = cells.get(col)?.text.as_str();
        let end = col
            .saturating_add(termwiz::cell::unicode_column_width(text, None).max(1))
            .min(cells.len());
        let span = col..end;
        col = end;
        Some((span, text))
    })
}

/// A normalised selection range: `start <= end` in reading order.
///
/// Coordinates are transcript-absolute `(row, column)` in `i64` / `usize`.
/// Rows may be negative (a drag above the viewport into scrollback that has
/// not yet been materialised); clamping to `0..len` is the caller's job when
/// extracting text, not the model's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectionRange {
    /// First selected cell (inclusive), reading order.
    pub start_row: i64,
    pub start_col: usize,
    /// Last selected cell (inclusive), reading order.
    pub end_row: i64,
    pub end_col: usize,
    /// Whether the end point was past the line's last column, meaning
    /// "select through the end of the line" (the trailing newline).
    pub end_past_line: bool,
}

impl SelectionRange {
    /// Build a normalised range from an anchor and a focus, in either direction.
    ///
    /// The normalisation handles **both** forward and backward drags: a drag
    /// from bottom-right to top-left produces the same range as top-left to
    /// bottom-right. Sabotage: normalising only one direction would make
    /// backwards drags select nothing.
    pub(crate) fn from_anchor_focus(
        anchor: (i64, usize),
        focus: (i64, usize),
        focus_past_line: bool,
    ) -> Self {
        let (start, end, end_past) =
            if anchor.0 < focus.0 || (anchor.0 == focus.0 && anchor.1 <= focus.1) {
                (anchor, focus, focus_past_line)
            } else {
                // Backwards drag: anchor becomes end, focus becomes start.
                // The "past line" flag applies to the original focus (now start),
                // but for the reversed end we don't have that info — anchor was
                // always a press *on* a column, never past it.
                (focus, anchor, false)
            };
        Self {
            start_row: start.0,
            start_col: start.1,
            end_row: end.0,
            end_col: end.1,
            end_past_line: end_past,
        }
    }

    /// Whether a specific cell is inside this selection (character-stream mode).
    pub(crate) fn contains(&self, row: i64, col: usize) -> bool {
        if row < self.start_row || row > self.end_row {
            return false;
        }
        if row == self.start_row && row == self.end_row {
            return col >= self.start_col && col <= self.end_col;
        }
        if row == self.start_row {
            return col >= self.start_col;
        }
        if row == self.end_row {
            return col <= self.end_col;
        }
        // Rows strictly between start and end are fully selected.
        true
    }

    /// Whether this range is empty (anchor == focus on the same cell).
    pub(crate) fn is_empty(&self) -> bool {
        self.start_row == self.end_row && self.start_col == self.end_col
    }

    /// Extract the selected text from a transcript.
    ///
    /// `transcript` is the full pane transcript (all rows, not just visible).
    /// The range is clamped to `[0, transcript.len())` so scrollback
    /// coordinates that have not been materialised are silently skipped.
    ///
    /// This extracts from the **full transcript**, not just the visible window.
    /// Sabotage: copying only visible rows would lose text when the selection
    /// spans a scrolled-off row.
    /// Intersecting a wide grapheme copies it once, without continuation-column spaces.
    pub(crate) fn selected_text(&self, transcript: &[TranscriptRow]) -> String {
        let total = transcript.len() as i64;
        let first = self.start_row.max(0) as usize;
        let last = (self.end_row.max(0) as usize).min(transcript.len().saturating_sub(1));
        if first >= transcript.len() || self.start_row >= total {
            return String::new();
        }

        let mut result = String::new();
        for (row_idx, row) in transcript.iter().enumerate().take(last + 1).skip(first) {
            let line = row.as_ref();
            let row_i64 = row_idx as i64;
            let cells = &line.cells;
            let col_start = if row_i64 == self.start_row {
                self.start_col
            } else {
                0
            };
            let col_end = if row_i64 == self.end_row {
                self.end_col.min(cells.len().saturating_sub(1))
            } else {
                cells.len().saturating_sub(1)
            };

            for (cols, text) in grapheme_columns(cells) {
                if cols.start > col_end {
                    break;
                }
                if cols.end > col_start {
                    result.push_str(text);
                }
            }

            // Append a newline between rows (not after the last).
            if row_idx < last {
                // Trim trailing spaces from the row before adding newline,
                // matching xterm behaviour: selecting a short line does not
                // paste a line full of trailing spaces.
                let trimmed = result.trim_end_matches(' ');
                result.truncate(trimmed.len());
                result.push('\n');
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Word and line selection — pure functions, gesture-agnostic.
// ---------------------------------------------------------------------------

/// Characters that delimit words in a terminal. Matches the set xterm and
/// gnome-terminal use: whitespace plus common shell punctuation.
fn is_word_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '(' | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '<'
                | '>'
                | '"'
                | '\''
                | '`'
                | '|'
                | '&'
                | ';'
                | ','
                | '.'
                | ':'
                | '/'
                | '\\'
                | '='
                | '+'
                | '-'
                | '*'
                | '?'
                | '!'
                | '@'
                | '#'
                | '$'
                | '%'
                | '^'
                | '~'
        )
}

/// Select the word under `(row, col)` from the transcript.
///
/// A "word" is a maximal run of non-boundary characters. If the cell under the
/// cursor IS a boundary character, select just that one cell.
///
/// This is the pure logic for double-click word selection. The gesture (timing
/// the double-click) is NOT wired here — a shared framework detector is being
/// built separately.
pub(crate) fn word_selection_at(
    transcript: &[TranscriptRow],
    row: i64,
    col: usize,
) -> Option<SelectionRange> {
    let row_usize = usize::try_from(row).ok()?;
    let line = transcript.get(row_usize)?;
    let cells = &line.cells;
    if col >= cells.len() {
        return None;
    }

    let ch = cells[col].text.as_str().chars().next().unwrap_or(' ');
    if is_word_boundary(ch) {
        // Select just this one boundary character.
        return Some(SelectionRange {
            start_row: row,
            start_col: col,
            end_row: row,
            end_col: col,
            end_past_line: false,
        });
    }

    // Expand left.
    let mut start = col;
    while start > 0 {
        let prev_ch = cells[start - 1].text.as_str().chars().next().unwrap_or(' ');
        if is_word_boundary(prev_ch) {
            break;
        }
        start -= 1;
    }

    // Expand right.
    let mut end = col;
    while end + 1 < cells.len() {
        let next_ch = cells[end + 1].text.as_str().chars().next().unwrap_or(' ');
        if is_word_boundary(next_ch) {
            break;
        }
        end += 1;
    }

    Some(SelectionRange {
        start_row: row,
        start_col: start,
        end_row: row,
        end_col: end,
        end_past_line: false,
    })
}

/// Select the entire line at `row`. This is the pure logic for triple-click
/// line selection.
pub(crate) fn line_selection_at(transcript: &[TranscriptRow], row: i64) -> Option<SelectionRange> {
    let row_usize = usize::try_from(row).ok()?;
    let line = transcript.get(row_usize)?;
    let last_col = line.cells.len().saturating_sub(1);
    Some(SelectionRange {
        start_row: row,
        start_col: 0,
        end_row: row,
        end_col: last_col,
        end_past_line: true,
    })
}
