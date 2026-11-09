//! Emulator rows as app-side `TranscriptLine`s.
//!
//! One function projects both the visible rows and the rows retained in
//! scrollback, so the two have identical shapes: same cell padding, same run
//! coalescing, same trailing-blank trim, same tone tally.

use std::num::NonZeroU64;

use alacritty_terminal::grid::Row;
use alacritty_terminal::index::Column;
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use enzui_core::grapheme_cell::InlineGrapheme;

use crate::escape_policy;
use crate::model::{
    CellInk, LineTone, TranscriptCell, TranscriptHyperlink, TranscriptLine, TranscriptRun,
    cell_attr,
};

/// A cell colour as a colour **source** ([`CellInk`]), not a resolved RGB:
/// the view resolves it against the active scheme on each paint, so a scheme
/// change recolours text already on screen.
pub(crate) fn ink(color: Color) -> CellInk {
    match color {
        Color::Spec(rgb) => CellInk::TrueColor(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => CellInk::Indexed(i),
        Color::Named(named) => match palette_index(named) {
            Some(i) => CellInk::Indexed(i),
            None => CellInk::Default,
        },
    }
}

/// The palette slot of a named colour; `None` for the default colours.
fn palette_index(named: NamedColor) -> Option<u8> {
    let index = named as usize;
    if index < 16 {
        return Some(index as u8);
    }
    match named {
        NamedColor::DimBlack
        | NamedColor::DimRed
        | NamedColor::DimGreen
        | NamedColor::DimYellow
        | NamedColor::DimBlue
        | NamedColor::DimMagenta
        | NamedColor::DimCyan
        | NamedColor::DimWhite => Some((index - NamedColor::DimBlack as usize) as u8),
        _ => None,
    }
}

/// Foreground of a reverse-video cell: its background. A default background
/// reverses to the scheme's canvas, so the glyph reads as a hole in the block.
fn ink_fg_reversed(bg: Color) -> CellInk {
    match ink(bg) {
        CellInk::Default => CellInk::SchemeBackground,
        other => other,
    }
}

/// Background of a reverse-video cell: its foreground. A default foreground
/// reverses to the scheme's ink, so the cell is a filled block.
fn ink_bg_reversed(fg: Color) -> CellInk {
    match ink(fg) {
        CellInk::Default => CellInk::SchemeForeground,
        other => other,
    }
}

/// The semantic tone of a cell's foreground. Faint (SGR 2) is Subtle whatever
/// the colour, as before.
pub(crate) fn tone_for(fg: Color, flags: Flags) -> LineTone {
    if flags.contains(Flags::DIM) {
        return LineTone::Subtle;
    }
    match ink(fg) {
        CellInk::Indexed(i) => tone_from_ansi_index(i),
        CellInk::TrueColor(r, g, b) => tone_from_rgb(r, g, b),
        _ => LineTone::Default,
    }
}

pub(crate) fn tone_from_ansi_index(i: u8) -> LineTone {
    match i % 16 {
        0 => LineTone::Muted,
        1 | 9 => LineTone::Error,
        2 | 10 => LineTone::Success,
        3 | 11 => LineTone::Warning,
        4 | 12 | 6 | 14 | 5 | 13 => LineTone::Accent,
        _ => LineTone::Default,
    }
}

pub(crate) fn tone_from_rgb(r: u8, g: u8, b: u8) -> LineTone {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max.saturating_sub(min) < 32 {
        return if max < 80 {
            LineTone::Muted
        } else {
            LineTone::Default
        };
    }
    if r >= g && r >= b {
        if g >= 150 && b < 100 {
            LineTone::Warning
        } else {
            LineTone::Error
        }
    } else if g >= r && g >= b {
        LineTone::Success
    } else {
        LineTone::Accent
    }
}

fn tone_index(t: LineTone) -> usize {
    match t {
        LineTone::Default => 0,
        LineTone::Muted => 1,
        LineTone::Subtle => 2,
        LineTone::Accent => 3,
        LineTone::Success => 4,
        LineTone::Warning => 5,
        LineTone::Error => 6,
    }
}

fn tone_from_index(i: usize) -> LineTone {
    match i {
        1 => LineTone::Muted,
        2 => LineTone::Subtle,
        3 => LineTone::Accent,
        4 => LineTone::Success,
        5 => LineTone::Warning,
        6 => LineTone::Error,
        _ => LineTone::Default,
    }
}

/// The SGR attributes of a cell, as [`cell_attr`] bits.
pub(crate) fn attrs_of(flags: Flags) -> u8 {
    let mut attrs = 0;
    for (flag, bit) in [
        (Flags::ITALIC, cell_attr::ITALIC),
        (Flags::UNDERLINE, cell_attr::UNDERLINE),
        (Flags::DOUBLE_UNDERLINE, cell_attr::DOUBLE_UNDERLINE),
        (Flags::UNDERCURL, cell_attr::CURLY_UNDERLINE),
        (Flags::DOTTED_UNDERLINE, cell_attr::DOTTED_UNDERLINE),
        (Flags::DASHED_UNDERLINE, cell_attr::DASHED_UNDERLINE),
        (Flags::STRIKEOUT, cell_attr::STRIKETHROUGH),
        (Flags::HIDDEN, cell_attr::HIDDEN),
    ] {
        if flags.contains(flag) {
            attrs |= bit;
        }
    }
    attrs
}

fn blank_cell() -> TranscriptCell {
    TranscriptCell {
        text: InlineGrapheme::new(" "),
        tone: LineTone::Default,
        bold: false,
        blank: true,
        fg: CellInk::Default,
        bg: CellInk::Default,
        attrs: 0,
    }
}

/// A row of `cols` blank cells (a row the emulator has not drawn yet).
#[cfg(test)]
pub(crate) fn blank_line(
    id: String,
    is_prompt: bool,
    cursor_col: usize,
    cols: usize,
) -> TranscriptLine {
    TranscriptLine {
        id,
        text: String::new(),
        hyperlinks: Vec::new(),
        tone: LineTone::Default,
        runs: Vec::new(),
        cells: (0..cols).map(|_| blank_cell()).collect(),
        is_prompt,
        cursor_col: if is_prompt { cursor_col as i32 } else { 0 },
    }
}

/// One cell as the projection reads it, from a visible row or a retained one.
pub(crate) struct CellView<'a> {
    pub c: char,
    pub zerowidth: &'a [char],
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
    pub link: Option<Hyperlink>,
}

impl<'a> CellView<'a> {
    pub(crate) fn of(cell: &'a Cell) -> Self {
        Self {
            c: cell.c,
            zerowidth: cell.zerowidth().unwrap_or_default(),
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags,
            link: if cell.extra.is_some() {
                cell.hyperlink()
            } else {
                None
            },
        }
    }
}

/// One visible emulator row as a `TranscriptLine` of `cols` cells.
pub(crate) fn project_cells(
    cells: &[Cell],
    id: &str,
    is_prompt: bool,
    cursor_col: usize,
    cols: usize,
    link_generation: Option<NonZeroU64>,
) -> TranscriptLine {
    project_row(
        cells.iter().map(CellView::of),
        id,
        is_prompt,
        cursor_col,
        cols,
        link_generation,
    )
}

/// Projects one row, `cells` (at most `cols` long; a retained row drops its
/// trailing default blanks), into a `TranscriptLine` of `cols` cells.
///
/// `link_generation` is the authority the row's OSC 8 ranges are stamped
/// with; `None` projects no ranges at all.
pub(crate) fn project_row<'a>(
    cells: impl Iterator<Item = CellView<'a>>,
    id: &str,
    is_prompt: bool,
    cursor_col: usize,
    cols: usize,
    link_generation: Option<NonZeroU64>,
) -> TranscriptLine {
    let mut text = String::new();
    let mut runs: Vec<TranscriptRun> = Vec::new();
    let mut out: Vec<TranscriptCell> = Vec::with_capacity(cols);
    let mut hyperlinks: Vec<TranscriptHyperlink> = Vec::new();
    let mut pending_text = String::new();
    let mut pending_tone = LineTone::Default;
    let mut pending_bold = false;
    let mut pending_start_col = 0usize;
    let mut have_pending = false;
    let mut hits = [0u32; 7];
    let mut bold_default_hits = 0u32;
    let mut grapheme = String::new();

    let mut cells = cells.take(cols).enumerate().peekable();
    while let Some((col, cell)) = cells.next() {
        let mut flags = cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            // The second column of the wide glyph before it.
            continue;
        }
        // A wide glyph whose second column was edited away is broken: it
        // shows as a blank, as in xterm.
        let broken = flags.contains(Flags::WIDE_CHAR)
            && !cells
                .peek()
                .is_some_and(|(_, next)| next.flags.contains(Flags::WIDE_CHAR_SPACER));
        if broken {
            flags.remove(Flags::WIDE_CHAR);
        }
        grapheme.clear();
        if broken
            || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
            || cell.c == '\0'
            || cell.c == '\t'
        {
            grapheme.push(' ');
        } else {
            grapheme.push(cell.c);
            grapheme.extend(cell.zerowidth);
        }
        let s = grapheme.as_str();
        let width = if flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        if let Some(generation) = link_generation
            && let Some(link) = &cell.link
            && escape_policy::link_is_openable(link.uri())
        {
            let uri = link.uri();
            if let Some(previous) = hyperlinks.last_mut()
                && previous.uri == uri
                && previous.end_col == col
            {
                previous.end_col = col + width;
            } else if hyperlinks.len() < escape_policy::MAX_LINE_HYPERLINKS {
                hyperlinks.push(TranscriptHyperlink {
                    start_col: col,
                    end_col: col + width,
                    uri: uri.to_owned(),
                    generation: generation.get(),
                });
            }
        }
        let bold = flags.contains(Flags::BOLD);
        let tone = tone_for(cell.fg, flags);
        let (fg, bg) = if flags.contains(Flags::INVERSE) {
            (ink_fg_reversed(cell.bg), ink_bg_reversed(cell.fg))
        } else {
            (ink(cell.fg), ink(cell.bg))
        };
        let blank = s.trim().is_empty();
        if !blank {
            if matches!(tone, LineTone::Default) && bold {
                bold_default_hits += 1;
            } else {
                hits[tone_index(tone)] += 1;
            }
        }
        while out.len() < col {
            out.push(blank_cell());
        }
        let attrs = attrs_of(flags);
        out.push(TranscriptCell {
            text: InlineGrapheme::new(s),
            tone,
            bold,
            blank,
            fg,
            bg,
            attrs,
        });
        if width == 2 && out.len() < cols {
            out.push(TranscriptCell {
                text: InlineGrapheme::new(" "),
                tone,
                bold,
                blank: true,
                fg,
                bg,
                attrs,
            });
        }
        if have_pending && (tone != pending_tone || bold != pending_bold) {
            runs.push(TranscriptRun {
                text: std::mem::take(&mut pending_text),
                tone: pending_tone,
                bold: pending_bold,
                start_col: pending_start_col as i32,
            });
            pending_start_col = col;
        }
        if !have_pending {
            pending_start_col = col;
        }
        pending_text.push_str(s);
        pending_tone = tone;
        pending_bold = bold;
        have_pending = true;
        text.push_str(s);
    }
    while out.len() < cols {
        out.push(blank_cell());
    }
    if have_pending && !pending_text.is_empty() {
        runs.push(TranscriptRun {
            text: pending_text,
            tone: pending_tone,
            bold: pending_bold,
            start_col: pending_start_col as i32,
        });
    }
    while runs
        .last()
        .is_some_and(|r| r.text.trim().is_empty() && matches!(r.tone, LineTone::Default) && !r.bold)
    {
        let dropped = runs.pop().unwrap();
        let cut = text.len().saturating_sub(dropped.text.len());
        text.truncate(cut);
    }
    while text.ends_with(' ') {
        text.pop();
        if let Some(last) = runs.last_mut()
            && last.text.ends_with(' ')
        {
            last.text.pop();
        }
    }
    if runs.last().is_some_and(|r| r.text.is_empty()) {
        runs.pop();
    }
    let mut best_idx = 0usize;
    let mut best_hits = 0u32;
    for (i, &h) in hits.iter().enumerate().skip(1) {
        if h > best_hits {
            best_hits = h;
            best_idx = i;
        }
    }
    let tone = if best_hits > 0 {
        tone_from_index(best_idx)
    } else if bold_default_hits > 0 {
        LineTone::Accent
    } else {
        LineTone::Default
    };
    TranscriptLine {
        id: id.to_owned(),
        text,
        hyperlinks,
        tone,
        runs,
        cells: out,
        is_prompt,
        cursor_col: if is_prompt { cursor_col as i32 } else { 0 },
    }
}

/// A row that scrolled off, kept compactly: plain cells with no per-cell
/// allocation, and the rare cells with combining marks or a link in a side
/// table. Copying and dropping a row costs a few allocations, not one per
/// cell.
type RetainedExtras = Box<[(Box<[char]>, Option<Hyperlink>)]>;

pub(crate) struct RetainedRow {
    cells: Box<[RetainedCell]>,
    extras: RetainedExtras,
}

#[derive(Clone, Copy)]
struct RetainedCell {
    c: char,
    fg: Color,
    bg: Color,
    flags: Flags,
    /// 0, or 1 + the index of this cell's entry in `extras`.
    extra: u16,
}

impl RetainedRow {
    /// [`RetainedRow::copy`] of an emulator row. Cells the emulator has not
    /// written since it last reset the row are blanks with the last cell's
    /// background, so when that is the default the search for the last kept
    /// cell starts at the end of the written ones. A 200-column screen of
    /// short lines otherwise looked at every trailing blank of every row.
    pub(crate) fn copy_row(row: &Row<Cell>) -> Self {
        let cells = &row[Column(0)..];
        let written = match cells.last() {
            Some(last) if last.bg == Cell::default().bg => row.occupied(),
            _ => cells.len(),
        };
        Self::copy(&cells[..written])
    }

    /// Copies `cells` up to the last one that is not a default blank. Most
    /// rows end in blanks, so the end is found from the right first and only
    /// the kept cells are copied.
    pub(crate) fn copy(cells: &[Cell]) -> Self {
        let blank = Cell::default();
        let keep = cells
            .iter()
            .rposition(|cell| {
                cell.c != ' '
                    || cell.extra.is_some()
                    || cell.flags != blank.flags
                    || cell.bg != blank.bg
                    || cell.fg != blank.fg
            })
            .map_or(0, |last| last + 1);
        let mut extras = Vec::new();
        let cells = cells[..keep]
            .iter()
            .map(|cell| {
                let mut extra = 0;
                if cell.extra.is_some() && extras.len() < usize::from(u16::MAX) {
                    extras.push((
                        cell.zerowidth().unwrap_or_default().into(),
                        cell.hyperlink(),
                    ));
                    extra = extras.len() as u16;
                }
                RetainedCell {
                    c: cell.c,
                    fg: cell.fg,
                    bg: cell.bg,
                    flags: cell.flags,
                    extra,
                }
            })
            .collect();
        Self {
            cells,
            extras: extras.into_boxed_slice(),
        }
    }

    pub(crate) fn views(&self) -> impl Iterator<Item = CellView<'_>> {
        self.cells.iter().map(|cell| {
            let (zerowidth, link) = match cell.extra {
                0 => (&[][..], None),
                n => {
                    let (zerowidth, link) = &self.extras[usize::from(n) - 1];
                    (&zerowidth[..], link.clone())
                }
            };
            CellView {
                c: cell.c,
                zerowidth,
                fg: cell.fg,
                bg: cell.bg,
                flags: cell.flags,
                link,
            }
        })
    }

    /// The same row with its links dropped.
    pub(crate) fn without_links(&self) -> Self {
        Self {
            cells: self.cells.clone(),
            extras: self
                .extras
                .iter()
                .map(|(zerowidth, _)| (zerowidth.clone(), None))
                .collect(),
        }
    }
}

/// One visible row straight to the view's cells, for a live pane: the same
/// cells as [`project_cells`] followed by `terminal_view::row_cells` with no
/// overlays (a test holds the two equal), without building the transcript
/// line in between.
pub(crate) fn live_row(cells: &[Cell], cols: usize) -> Vec<enzui_terminal::Cell> {
    view_row(cells.iter(), cols)
}

/// What [`view_row`] reads of a cell: an emulator cell on screen, or a
/// scrollback row's retained one.
pub(crate) trait ViewSource {
    fn c(&self) -> char;
    fn zerowidth(&self) -> &[char];
    fn fg(&self) -> Color;
    fn bg(&self) -> Color;
    fn flags(&self) -> Flags;
}

impl ViewSource for &Cell {
    fn c(&self) -> char {
        self.c
    }
    fn zerowidth(&self) -> &[char] {
        Cell::zerowidth(self).unwrap_or_default()
    }
    fn fg(&self) -> Color {
        self.fg
    }
    fn bg(&self) -> Color {
        self.bg
    }
    fn flags(&self) -> Flags {
        self.flags
    }
}

impl ViewSource for CellView<'_> {
    fn c(&self) -> char {
        self.c
    }
    fn zerowidth(&self) -> &[char] {
        self.zerowidth
    }
    fn fg(&self) -> Color {
        self.fg
    }
    fn bg(&self) -> Color {
        self.bg
    }
    fn flags(&self) -> Flags {
        self.flags
    }
}

impl RetainedRow {
    /// This row straight to the view's cells, padded to `cols`, as
    /// [`live_row`] converts a row on screen.
    pub(crate) fn view_row(&self, cols: usize) -> Vec<enzui_terminal::Cell> {
        view_row(self.views(), cols)
    }
}

/// [`live_row`] over any row of cells.
fn view_row<C: ViewSource>(
    cells: impl Iterator<Item = C>,
    cols: usize,
) -> Vec<enzui_terminal::Cell> {
    use crate::terminal_view::{CellParts, view_cell};
    use enzui_terminal::CellFlags as View;
    let blank = || {
        view_cell(
            CellParts {
                ch: ' ',
                combining: "",
                blank: true,
                bold: false,
                tone: LineTone::Default,
                fg: CellInk::Default,
                bg: CellInk::Default,
                attrs: 0,
            },
            || false,
        )
        .0
    };
    let mut out: Vec<enzui_terminal::Cell> = Vec::with_capacity(cols);
    let mut grapheme = String::new();
    // Whether the last cell pushed was a wide glyph (its next blank is its
    // second column).
    let mut spacer = false;
    let mut push =
        |out: &mut Vec<enzui_terminal::Cell>, mut cell: enzui_terminal::Cell, blank: bool| {
            if spacer && blank {
                cell.flags |= View::WIDE_SPACER;
            }
            spacer = cell.flags.contains(View::WIDE);
            out.push(cell);
        };
    let mut cells = cells.take(cols).enumerate().peekable();
    while let Some((col, cell)) = cells.next() {
        let mut flags = cell.flags();
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let broken = flags.contains(Flags::WIDE_CHAR)
            && !cells
                .peek()
                .is_some_and(|(_, next)| next.flags().contains(Flags::WIDE_CHAR_SPACER));
        if broken {
            flags.remove(Flags::WIDE_CHAR);
        }
        grapheme.clear();
        if broken
            || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
            || cell.c() == '\0'
            || cell.c() == '\t'
        {
            grapheme.push(' ');
        } else {
            grapheme.push(cell.c());
            grapheme.extend(cell.zerowidth());
        }
        let width = if flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        let bold = flags.contains(Flags::BOLD);
        let tone = tone_for(cell.fg(), flags);
        let (fg, bg) = if flags.contains(Flags::INVERSE) {
            (ink_fg_reversed(cell.bg()), ink_bg_reversed(cell.fg()))
        } else {
            (ink(cell.fg()), ink(cell.bg()))
        };
        let attrs = attrs_of(flags);
        let is_blank = grapheme.trim().is_empty();
        while out.len() < col {
            push(&mut out, blank(), true);
        }
        let first = grapheme.chars().next().unwrap_or(' ');
        let (view, _) = view_cell(
            CellParts {
                ch: first,
                combining: &grapheme[first.len_utf8()..],
                blank: is_blank,
                bold,
                tone,
                fg,
                bg,
                attrs,
            },
            || termwiz::cell::unicode_column_width(&grapheme, None) > 1,
        );
        push(&mut out, view, is_blank);
        if width == 2 && out.len() < cols {
            let (view, _) = view_cell(
                CellParts {
                    ch: ' ',
                    combining: "",
                    blank: true,
                    bold,
                    tone,
                    fg,
                    bg,
                    attrs,
                },
                || false,
            );
            push(&mut out, view, true);
        }
    }
    while out.len() < cols {
        push(&mut out, blank(), true);
    }
    out
}
