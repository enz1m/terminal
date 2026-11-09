//! Split panes and transcript rows.

use super::*;

/// One split pane inside a session, backed by its own live PTY.
#[derive(Clone)]
pub(crate) struct Pane {
    pub endpoint: crate::pane_endpoint::PaneEndpoint,
    pub id: String,
    pub title: String,
    /// WRITTEN AND NEVER READ - see `preserved_transcript_rows` below.
    #[allow(
        dead_code,
        reason = "The pane model retains optional chrome metadata alongside terminal output."
    )]
    pub subtitle: String,
    pub status: SessionStatus,
    pub transcript: Vec<TranscriptRow>,
    /// This pane's own scrollback position.
    ///
    /// Per-pane, not per-app: two panes of one split scroll independently, and
    /// switching tabs must not move anyone's viewport. Defaults to
    /// `ScrollPosition::pinned_to_tail`, so a fresh pane follows output.
    pub scroll: crate::scrollback::ScrollPosition,
    /// Whether this pane's shell is in DECCKM (application cursor keys) mode
    ///, refreshed from the backend on every drain.
    ///
    /// Read only to choose the encoding of a synthesised cursor key (the
    /// alternate-scroll wheel); the grid does not care.
    pub application_cursor_keys: bool,
    /// Whether this pane's shell currently has DECSET 2004 (bracketed paste)
    /// mode engaged, refreshed from the backend on every drain.
    ///
    /// Read only by the paste path to decide whether to wrap the clipboard
    /// payload in the `ESC [ 200 ~` / `ESC [ 201 ~` guards; the grid does not
    /// care.
    pub bracketed_paste: bool,
    /// This pane's live mouse-tracking `(mode, encoding)` pair,
    /// refreshed from the backend on every drain.
    ///
    /// Read by the pointer/wheel handlers to decide whether a gesture is
    /// reported to the running application (and how) or belongs to local
    /// selection / scrollback; the grid does not care.
    pub mouse_protocol: crate::mouse::MouseProtocol,
    /// Whether this pane's shell currently has the ALTERNATE SCREEN engaged
    ///, refreshed from the backend on every drain.
    ///
    /// The wheel and PageUp/PageDown belong to a full-screen TUI while this is
    /// `true`; the primary scrollback is not its timeline and the backend keeps
    /// no scrollback for it.
    pub alt_screen: bool,
    /// Whether the program hid the cursor (DECTCEM `ESC [ ? 25 l`), refreshed
    /// on every drain; a hidden cursor is not drawn.
    pub cursor_hidden: bool,
    /// Number of leading transcript rows restored from a pinned snapshot.
    ///
    /// `preserved_transcript_rows`, `subtitle`, `prompt` and `footer`
    /// remain model-only chrome. `cwd` is live state: OSC 7 updates it, the
    /// session row renders it, and new local panes inherit it.
    #[allow(
        dead_code,
        reason = "Pinned transcript restoration retains the count alongside the output rows."
    )]
    pub preserved_transcript_rows: usize,
    #[allow(
        dead_code,
        reason = "The pane model retains optional prompt metadata without rendering extra chrome."
    )]
    pub prompt: String,
    #[allow(
        dead_code,
        reason = "The pane model retains optional footer metadata without rendering extra chrome."
    )]
    pub footer: String,
    pub cwd: String,
}

impl Pane {
    /// Record the pane's alternate-screen state, resetting the scrollback
    /// position whenever it CHANGES.
    ///
    /// Both directions matter. Entering the alternate screen with a scrolled-up
    /// offset would otherwise paint the tail of the primary scrollback above the
    /// TUI's own canvas — the two screens interleaved in one viewport, which is
    /// what a stale offset produced when `btop` started. Leaving it lands on the
    /// newest output, as every terminal does when a full-screen program exits.
    ///
    /// Returns `true` when the state changed, so a caller can drop any
    /// accumulated sub-row wheel residue with it.
    pub(crate) fn set_alt_screen(&mut self, alt: bool) -> bool {
        if self.alt_screen == alt {
            return false;
        }
        self.alt_screen = alt;
        self.scroll.jump_to_tail();
        true
    }
}

// ---------------------------------------------------------------------------
// Transcript / cell model — the render path for the terminal grid
// ---------------------------------------------------------------------------

/// Shared immutable row; deferred contents are projected on first access.
#[derive(Clone)]
pub(crate) struct TranscriptRow {
    revision: u64,
    contents: RowContents,
}

#[derive(Clone)]
enum RowContents {
    Ready(std::sync::Arc<TranscriptLine>),
    Deferred {
        source: std::sync::Arc<dyn DeferredLine>,
        /// This handle's view of the source's full projection, filled on the
        /// first `Deref`. [`TranscriptRow::release_expansion`] empties it.
        expanded: std::sync::OnceLock<std::sync::Arc<TranscriptLine>>,
    },
}

/// A deferred projection must remain immutable and retain its unique revision for its lifetime.
///
/// The full per-cell projection is the expensive part (one `TranscriptCell`
/// per column, about 8 KB for a 200-column row), so a source shares one copy
/// among its handles and may drop it again when no handle near a viewport needs
/// it: re-expanding projects the same frozen source and yields the same value.
pub(crate) trait DeferredLine: Send + Sync {
    /// The full projection, shared until [`Self::release`].
    fn expand(&self) -> std::sync::Arc<TranscriptLine>;
    /// Drop the shared projection, if held.
    fn release(&self);
    fn content_revision(&self) -> u64;
    /// The projection's `text`, answered without retaining its cells.
    fn text(&self) -> &str;
    /// The projection's `id`, likewise.
    fn id(&self) -> &str;
}

/// Deferred sources currently holding a full projection, process-wide.
static EXPANDED_DEFERRED_ROWS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Record that a deferred source created (`true`) or dropped (`false`) its
/// full projection.
pub(crate) fn note_deferred_expansion(expanded: bool) {
    use std::sync::atomic::Ordering;
    if expanded {
        EXPANDED_DEFERRED_ROWS.fetch_add(1, Ordering::Relaxed);
    } else {
        EXPANDED_DEFERRED_ROWS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// How many deferred rows currently hold a full projection.
pub(crate) fn expanded_deferred_rows() -> usize {
    EXPANDED_DEFERRED_ROWS.load(std::sync::atomic::Ordering::Relaxed)
}

impl TranscriptRow {
    pub(crate) fn allocate_revision() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("transcript revision space exhausted")
    }

    pub(crate) fn new(line: TranscriptLine) -> Self {
        Self {
            revision: Self::allocate_revision(),
            contents: RowContents::Ready(std::sync::Arc::new(line)),
        }
    }

    pub(crate) fn deferred(line: std::sync::Arc<dyn DeferredLine>) -> Self {
        Self {
            revision: line.content_revision(),
            contents: RowContents::Deferred {
                source: line,
                expanded: std::sync::OnceLock::new(),
            },
        }
    }

    /// Drop this handle's full projection and ask its source to drop the
    /// shared copy. Returns whether the handle held one. A later read
    /// re-expands the identical row from its frozen source.
    pub(crate) fn release_expansion(&mut self) -> bool {
        match &mut self.contents {
            RowContents::Deferred { source, expanded } => {
                // The source is released even when this handle never read it:
                // an earlier projection's handle may have expanded it.
                source.release();
                expanded.take().is_some()
            }
            RowContents::Ready(_) => false,
        }
    }

    /// Whether a deferred handle currently holds its full projection.
    #[cfg(test)]
    pub(crate) fn is_expanded(&self) -> bool {
        matches!(&self.contents, RowContents::Deferred { expanded, .. } if expanded.get().is_some())
    }

    /// Clones and repeated deferred handles retain the same process-unique payload revision.
    pub(crate) fn content_revision(&self) -> u64 {
        self.revision
    }

    /// The row's text, without materialising a deferred row's cells.
    pub(crate) fn text(&self) -> &str {
        match &self.contents {
            RowContents::Ready(line) => &line.text,
            RowContents::Deferred { source, .. } => source.text(),
        }
    }

    /// The row's id, without materialising a deferred row's cells.
    pub(crate) fn id(&self) -> &str {
        match &self.contents {
            RowContents::Ready(line) => &line.id,
            RowContents::Deferred { source, .. } => source.id(),
        }
    }

    /// Whether two handles share one row, without projecting either.
    pub(crate) fn ptr_eq(a: &Self, b: &Self) -> bool {
        match (&a.contents, &b.contents) {
            (RowContents::Ready(a), RowContents::Ready(b)) => std::sync::Arc::ptr_eq(a, b),
            (RowContents::Deferred { source: a, .. }, RowContents::Deferred { source: b, .. }) => {
                std::ptr::addr_eq(std::sync::Arc::as_ptr(a), std::sync::Arc::as_ptr(b))
            }
            _ => false,
        }
    }
}

impl std::ops::Deref for TranscriptRow {
    type Target = TranscriptLine;

    fn deref(&self) -> &TranscriptLine {
        match &self.contents {
            RowContents::Ready(line) => line,
            RowContents::Deferred { source, expanded } => expanded.get_or_init(|| source.expand()),
        }
    }
}

impl AsRef<TranscriptLine> for TranscriptRow {
    fn as_ref(&self) -> &TranscriptLine {
        self
    }
}

impl std::fmt::Debug for TranscriptRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        TranscriptLine::fmt(self, f)
    }
}

impl PartialEq for TranscriptRow {
    fn eq(&self, other: &Self) -> bool {
        Self::ptr_eq(self, other) || **self == **other
    }
}

impl From<TranscriptLine> for TranscriptRow {
    fn from(line: TranscriptLine) -> Self {
        Self::new(line)
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct TranscriptLine {
    pub id: String,
    /// Full row text — fallback / headless-assert text.
    pub text: String,
    /// OSC 8 hyperlink ranges retained in terminal-column coordinates.
    ///
    /// One range per contiguous link, rather than one `String` per cell: a
    /// normal 120-column row with no links pays no extra allocation.
    pub hyperlinks: Vec<TranscriptHyperlink>,
    /// Dominant tone for the row (coarse fallback).
    pub tone: LineTone,
    /// Coloured segments (contiguous same-tone clusters). Legacy path kept for
    /// headless assertions.
    pub runs: Vec<TranscriptRun>,
    /// Per-cell breakdown, one entry per screen column. This is what the
    /// `EnzTextGrid` widget iterates: each cell pinned at `x = round(col *
    /// cell_w)`.
    pub cells: Vec<TranscriptCell>,
    pub is_prompt: bool,
    /// Column (0-based) of the cursor when this row is the cursor row.
    pub cursor_col: i32,
}

/// One OSC 8 hyperlink painted across `[start_col, end_col)`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TranscriptHyperlink {
    pub start_col: usize,
    pub end_col: usize,
    pub uri: String,
    /// The screen's link authority when this range was projected.
    ///
    /// Process-unique and reminted on every screen reset, alternate-screen
    /// swap and new session, so a range only opens while the pane's current
    /// authority still matches. Retained scrollback keeps the generation it
    /// was captured under, which is what stops a pre-reset link from staying
    /// actionable.
    pub generation: u64,
}

/// One screen cell projected from the terminal parser. Maps 1:1 to the
/// framework's `TextGridCell`.
#[derive(Clone, PartialEq)]
pub(crate) struct TranscriptCell {
    /// This cell's grapheme cluster, stored **inline**.
    ///
    /// Inline storage rather than a `String`: the cell count tracks the
    /// terminal's width, not its content, so a ten-character line on an
    /// 80-column terminal would otherwise mint 80 heap buffers, 70 of them
    /// holding one space; a `String` here measured 23.3% of process CPU during
    /// a `cat` burst. The raw termwiz `Line` is frozen at scroll-off and these
    /// cells are materialised lazily on first read, and the inline storage
    /// keeps that materialisation allocation-free per cell.
    ///
    /// [`InlineGrapheme`] is the same 24 bytes as the `String` it replaces and
    /// is byte-exact for every cluster: clusters up to 22 bytes are inline,
    /// longer ones (4-person ZWJ families, tag-sequence flags) spill to the heap
    /// rather than being truncated. See
    /// `multi_byte_content_round_trips_byte_identically_through_projection`.
    ///
    /// [`InlineGrapheme`]: enzui_core::grapheme_cell::InlineGrapheme
    pub text: InlineGrapheme,
    pub tone: LineTone,
    pub bold: bool,
    /// `true` for pure whitespace cells; the renderer paints no glyph but keeps
    /// the column slot so column N is always the same pixel across rows.
    pub blank: bool,
    /// The **source** of this cell's foreground colour — not a resolved RGB.
    /// Resolved against the active colour scheme at draw time by
    /// `grid_view::map_cell`.
    pub fg: CellInk,
    /// The **source** of this cell's background fill (reverse video, meter
    /// fills), resolved at draw time exactly like [`Self::fg`].
    pub bg: CellInk,
    /// Text attributes beyond bold: [`cell_attr`] bits.
    pub attrs: u8,
}

/// [`TranscriptCell::attrs`] bits: the SGR attributes the view draws.
pub(crate) mod cell_attr {
    pub(crate) const ITALIC: u8 = 1;
    pub(crate) const UNDERLINE: u8 = 1 << 1;
    pub(crate) const DOUBLE_UNDERLINE: u8 = 1 << 2;
    pub(crate) const CURLY_UNDERLINE: u8 = 1 << 3;
    pub(crate) const DOTTED_UNDERLINE: u8 = 1 << 4;
    pub(crate) const DASHED_UNDERLINE: u8 = 1 << 5;
    pub(crate) const STRIKETHROUGH: u8 = 1 << 6;
    pub(crate) const HIDDEN: u8 = 1 << 7;
}

/// Where a cell's colour came from, preserved so the draw path can resolve it
/// against whichever colour scheme is active *now*.
///
/// This field used to be an `Option<(u8, u8, u8)>` holding an already-resolved
/// RGB. That made every scheme change lossy: `Message::SetColorScheme` updated
/// the resolver and the canvas, but each projected cell kept the RGB the *old*
/// scheme had resolved it to, so the reported symptom was "the background
/// changes but the text colours do not". A cell that remembers only an RGB has
/// no way back to the palette slot the shell actually asked for.
///
/// It also makes the readability safeguard expressible. A `Default` or `Indexed`
/// colour is the *framework's* to choose, so `palette::readable_on` may lift it
/// to the contrast floor; a [`Self::TrueColor`] value is the *application's*
/// request (btop's `#101010` meter track) and is painted exactly as asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellInk {
    /// SGR default. As a foreground: fall back to the row's semantic
    /// [`LineTone`] / the scheme's `default_fg`. As a background: transparent,
    /// i.e. the terminal canvas shows through.
    Default,
    /// An xterm-256 palette index (`SGR 30-37`, `90-97`, `38;5;n`). Resolved
    /// through the active scheme for `0..=15` and through the universal xterm
    /// cube/greyscale table above that.
    Indexed(u8),
    /// A 24-bit colour the application asked for by value (`SGR 38;2;r;g;b`).
    /// Painted verbatim — never scheme-mapped, never contrast-nudged.
    TrueColor(u8, u8, u8),
    /// The active scheme's `default_fg` as a concrete colour. This is what a
    /// REVERSE-VIDEO cell's *fill* is when the cell had no explicit background:
    /// the block must paint the default ink colour, and it must follow the
    /// scheme like everything else.
    SchemeForeground,
    /// The active scheme's `default_bg` as a concrete colour: a reverse-video
    /// cell's *glyph*, so the character reads as a hole punched in the block.
    /// Deliberately exempt from the contrast nudge — matching the canvas is the
    /// entire point.
    SchemeBackground,
}

/// One contiguous same-colour segment inside a `TranscriptLine`.
#[derive(Clone, PartialEq)]
pub(crate) struct TranscriptRun {
    pub text: String,
    pub tone: LineTone,
    pub bold: bool,
    pub start_col: i32,
}

impl TranscriptLine {
    /// Construct a monochrome line — the common shape for scripted transcripts
    /// and fallback rendering.
    pub(crate) fn monochrome(
        id: impl Into<String>,
        text: impl Into<String>,
        tone: LineTone,
        is_prompt: bool,
    ) -> Self {
        let text = text.into();
        // Segment on GRAPHEME CLUSTERS, not on `char`s.
        //
        // This used to be `text.chars().map(..)`, which tore every cluster apart:
        // a US flag is ONE cluster of TWO `char`s, so it became two cells, and a
        // lone regional indicator renders as its *fallback glyph* — a cyan letter
        // in a box. That is literally the "paints as the letters US" defect, on a
        // path with ten call sites (the shell-exited banner, the failed-pane
        // reason, the demo transcript), entirely independent of the terminal
        // parser.
        //
        // # Why this stays COLUMN-INDEXED
        //
        // `cells` is indexed by screen column, not by cluster: `grid_view::
        // row_cells` matches `cursor_col` against the cell's `.enumerate()`
        // index, and `project_line` (the PTY path) pads a blank cell for every
        // continuation column termwiz's `visible_cells()` skips. Emitting one
        // cell per cluster would silently make index != column for any row
        // containing a wide glyph, and the block cursor would land left of where
        // it paints — the drift this issue explicitly warns about. So a
        // double-width cluster contributes its glyph cell PLUS one blank
        // continuation cell, exactly as the PTY path already does.
        //
        // Widths come from termwiz's own `unicode_column_width`, which is the
        // very function `Surface::print_text` advances the cursor by, so this
        // row's column arithmetic agrees with the parser's by construction
        // rather than by a second table that could drift.
        let mut cells: Vec<TranscriptCell> = Vec::new();
        for g in text.graphemes(true) {
            let width = unicode_column_width(g, None).max(1);
            cells.push(TranscriptCell {
                text: InlineGrapheme::new(g),
                tone,
                bold: false,
                blank: g.trim().is_empty(),
                fg: CellInk::Default,
                bg: CellInk::Default,
                attrs: 0,
            });
            // Continuation columns of a wide cluster: blank, so column N is the
            // same pixel on every row and the cursor index stays a column index.
            for _ in 1..width {
                cells.push(TranscriptCell {
                    text: InlineGrapheme::new(" "),
                    tone,
                    bold: false,
                    blank: true,
                    fg: CellInk::Default,
                    bg: CellInk::Default,
                    attrs: 0,
                });
            }
        }
        // The prompt cursor sits one past the last column, which is the row's
        // COLUMN count. `text.chars().count()` was wrong in both directions at
        // once: too large by one for every combining mark or ZWJ joiner, too
        // small by one for every double-width glyph.
        let cursor_col = if is_prompt { cells.len() as i32 } else { 0 };
        Self {
            id: id.into(),
            hyperlinks: Vec::new(),
            runs: vec![TranscriptRun {
                text: text.clone(),
                tone,
                bold: false,
                start_col: 0,
            }],
            cells,
            text,
            tone,
            is_prompt,
            cursor_col,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineTone {
    Default,
    Muted,
    /// SGR 2 (faint / dim) intensity. Sits between Default and Muted.
    Subtle,
    Accent,
    Success,
    Warning,
    Error,
}
