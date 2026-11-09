// No module-level allow.
//! Terminal cell geometry, colours and pointer reports.
//!
//! The panes draw with the framework's GPU terminal view
//! ([`crate::terminal_view`]). This module holds what that view and the
//! reducers share:
//!
//!   * [`CellGeometry`], measured from the terminal font itself, so the rows and
//!     columns given to the PTY are the cells the view paints:
//!     `cols = floor(pane_width / cell_width)`, `rows = floor(pane_height /
//!     cell_height)`;
//!   * colour resolution of a cell's ink against the active scheme, with the
//!     contrast floor;
//!   * the selection and find fills, and the right-edge fill;
//!   * [`GridPointerTarget`], a pointer report in cell coordinates.

use enzui_widgets::text_grid::TextGridEdge;
use iced::widget::{Space, container};
use iced::{Color, Element, Length};

use crate::model::CellInk;
#[cfg(test)]
use crate::model::{Settings, TranscriptLine};

/// The measured geometry of one terminal cell, derived once from the font size
/// and used by BOTH the paint pitch and the PTY resize `cols/rows` computation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellGeometry {
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
}

impl CellGeometry {
    /// Derive cell geometry from the resolved settings font size. Columns are
    /// pinned to `round(col * cell_width)` by the grid widget's own snap-down.
    #[cfg(test)]
    pub(crate) fn from_settings(settings: &Settings) -> Self {
        Self::from_font_size(settings.resolved_font_size_px())
    }

    /// The cell the GPU terminal view draws at this font size, measured from
    /// the font itself at scale 1 (whole logical pixels), so the rows and
    /// columns the app gives the PTY are the ones the view paints. Cached per
    /// size: measuring shapes a glyph.
    /// [`Self::from_font`] with the process's published monospace family.
    #[cfg(test)]
    pub(crate) fn from_font_size(font_size: f32) -> Self {
        Self::from_font(
            font_size,
            crate::terminal_view::terminal_font(font_size, None)
                .resolved()
                .family,
        )
    }

    /// The cell at `font_size` in `family` (the app theme's monospace font).
    pub(crate) fn from_font(font_size: f32, family: Option<String>) -> Self {
        // Keyed by size and family: the desktop monospace font can change at
        // run time, and the view measures the new one.
        type Measured = Vec<((u32, Option<String>), (f32, f32))>;
        thread_local! {
            static MEASURED: std::cell::RefCell<Measured> =
                const { std::cell::RefCell::new(Vec::new()) };
        }
        let font = crate::terminal_view::terminal_font(font_size, family.clone());
        let concrete = crate::terminal_view::concrete_monospace(family);
        let key = (font_size.to_bits(), Some(concrete));
        let (cell_width, cell_height) = MEASURED.with(|measured| {
            if let Some((_, cell)) = measured.borrow().iter().find(|(k, _)| *k == key) {
                return *cell;
            }
            let metrics = enzui_terminal::CellMetrics::measure(&font, 1.0);
            let cell = (
                (metrics.width as f32).max(1.0),
                (metrics.height as f32).max(1.0),
            );
            let mut measured = measured.borrow_mut();
            if measured.len() >= 16 {
                measured.remove(0);
            }
            measured.push((key, cell));
            cell
        });
        Self {
            cell_width,
            cell_height,
            font_size,
        }
    }

    /// Column count for a pane of the given pixel width. `floor` so the last
    /// partial column is never asked of the shell. Clamped to [1, 1000] so a
    /// zero/absurd width can't wedge the PTY.
    pub(crate) fn cols_for(&self, pane_width: f32) -> u16 {
        if !pane_width.is_finite() || pane_width <= 0.0 {
            return crate::pty::DEFAULT_COLS;
        }
        ((pane_width / self.cell_width).floor() as i64).clamp(1, 1000) as u16
    }

    /// Row count for a pane of the given pixel height.
    pub(crate) fn rows_for(&self, pane_height: f32) -> u16 {
        if !pane_height.is_finite() || pane_height <= 0.0 {
            return crate::pty::DEFAULT_ROWS;
        }
        ((pane_height / self.cell_height).floor() as i64).clamp(1, 1000) as u16
    }
}

/// DOCUMENTED DATA EXCEPTION (not a theme literal): the terminal's cell colours
/// are raw 24-bit RGB values that arrive from the PTY / SGR escape sequences and
/// the selected colour-scheme (Dracula, Solarized, …). They are *terminal
/// content*, not app chrome, so no `Palette` token expresses them — the same
/// class as task-manager-app's per-resource identity accents. `Color::from_rgb8`
/// here converts that raw `(u8, u8, u8)` data into an iced colour; it is never a
/// hand-picked UI colour.
pub(crate) fn rgb(c: (u8, u8, u8)) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}

/// Resolve a cell's **background** ink source against the active scheme.
///
/// `None` = transparent (the canvas shows through).
pub(crate) fn resolve_bg_ink(ink: CellInk) -> Option<(u8, u8, u8)> {
    resolve_bg_ink_for(crate::color_scheme::active(), ink)
}

/// [`resolve_bg_ink`] against an explicit scheme, so a test can sweep the whole
/// catalogue without racing the process-global active index.
pub(crate) fn resolve_bg_ink_for(
    scheme: &crate::color_scheme::ColorScheme,
    ink: CellInk,
) -> Option<(u8, u8, u8)> {
    match ink {
        CellInk::Default => None,
        CellInk::Indexed(i) => Some(crate::color_scheme::resolve_ansi_index_for(scheme, i)),
        CellInk::TrueColor(r, g, b) => Some((r, g, b)),
        CellInk::SchemeForeground => Some(scheme.default_fg),
        CellInk::SchemeBackground => Some(scheme.default_bg),
    }
}

/// Resolve a cell's **foreground** ink source against the active scheme, on the
/// background the glyph actually lands on.
///
/// `None` = no explicit colour; the caller falls back to the semantic tone /
/// the scheme's `default_fg`.
///
/// This is where a scheme change becomes visible on text that is ALREADY on
/// screen: the projection stored `CellInk::Indexed(4)`, not an RGB, so this runs
/// against whichever scheme is active on this frame. It is also the only place
/// the contrast floor is applied, and it is applied to
/// [`CellInk::Indexed`]/[`CellInk::Default`]-derived colours only — a
/// [`CellInk::TrueColor`] is returned byte-exact, because a TUI that asked for
/// `#101010` asked for it.
pub(crate) fn resolve_fg_ink(ink: CellInk, effective_bg: (u8, u8, u8)) -> Option<(u8, u8, u8)> {
    resolve_fg_ink_for(crate::color_scheme::active(), ink, effective_bg)
}

/// [`resolve_fg_ink`] against an explicit scheme.
///
/// The public entry point reads the process-global active scheme and delegates
/// here, so a catalogue-wide contrast sweep measures the *same* resolution the
/// paint path performs without mutating (or racing) that global.
pub(crate) fn resolve_fg_ink_for(
    scheme: &crate::color_scheme::ColorScheme,
    ink: CellInk,
    effective_bg: (u8, u8, u8),
) -> Option<(u8, u8, u8)> {
    match ink {
        CellInk::Default => None,
        CellInk::Indexed(i) => Some(crate::palette::readable_on(
            crate::color_scheme::resolve_ansi_index_for(scheme, i),
            effective_bg,
        )),
        // The application's own 24-bit request: painted verbatim.
        CellInk::TrueColor(r, g, b) => Some((r, g, b)),
        CellInk::SchemeForeground => Some(scheme.default_fg),
        // Reverse video's glyph: matching the canvas IS the intent.
        CellInk::SchemeBackground => Some(scheme.default_bg),
    }
}

/// The active colour-scheme cursor tone as an iced colour.
#[cfg(test)]
pub(crate) fn scheme_cursor() -> Color {
    let s = crate::color_scheme::active();
    rgb(s.cursor)
}

/// The terminal canvas colour: **the active scheme's own background**.
///
/// This used to consult a second, parallel background system — the settings
/// dialog's `Dark` / `Darker` / `Black` buttons — whenever the active scheme did
/// not set `override_background`. Since every scheme except the removed
/// low-contrast `Default` mapping DID set it, those three buttons were live in
/// the UI and deliberately inert in six of seven configurations. The presets and
/// the flag are gone: a scheme owns its canvas, full stop, and there is exactly
/// one place a canvas colour comes from.
pub(crate) fn terminal_background() -> Color {
    rgb(crate::color_scheme::active().default_bg)
}

/// The line between split panes, and under the titlebar strip while the
/// content is split: the canvas a step toward the scheme's text colour, so it
/// reads on any scheme without the weight of a black rule.
pub(crate) fn pane_seam() -> Color {
    let scheme = crate::color_scheme::active();
    let (bg, fg) = (rgb(scheme.default_bg), rgb(scheme.default_fg));
    let mix = |a: f32, b: f32| a + (b - a) * 0.16;
    Color::from_rgb(mix(bg.r, fg.r), mix(bg.g, fg.g), mix(bg.b, fg.b))
}

/// Laid over an unfocused split pane: the canvas, part opaque, so its text
/// recedes toward the background and the focused pane stands out.
pub(crate) fn unfocused_pane_shade() -> Color {
    Color {
        a: 0.38,
        ..terminal_background()
    }
}

// ---------------------------------------------------------------------------
// Pointer reports in cell coordinates
// ---------------------------------------------------------------------------

/// Where a pointer event landed on the grid, in **terminal cell coordinates**
///.
///
/// The framework's `text_grid` reports a column and a signed `row_offset`
/// relative to the one row it paints ([`TextGridPointer`]); this row knows its
/// own index in the rendered window, so it adds the two and hands the app a
/// coordinate the app can reason about. Translating the window row into a
/// *transcript* row is `App`'s job — only it knows where the scrollback window
/// starts.
///
/// This is deliberately **coordinates only**: no anchor/focus pair, no
/// inverted paint, no clipboard. Those are the follow-ups this seam makes
/// possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridPointerTarget {
    /// Row index within the **rendered window**, signed: a drag above the first
    /// rendered row is negative, and one below the last is `rows` or more. Not
    /// clamped, because clamping here would silently lose the row a drag left
    /// the viewport toward.
    pub window_row: i32,
    /// Column within the row, always a real column of that row.
    pub column: usize,
    /// Whether the pointer was really over that column, or past one end of the
    /// row (which is how "select through the end of the line" is expressed).
    pub edge: TextGridEdge,
    /// Which pane of the active session this report came from, as an index into
    /// `Session::panes`.
    ///
    /// `None` is an unscoped surface (the single terminal, the detached
    /// window). A split leaf names itself, so a gesture starting in one pane
    /// cannot extend another's selection, and pressing an unfocused leaf can
    /// focus it before the gesture begins.
    pub pane: Option<usize>,
}

/// The three pointer reports the app subscribes to.
///
/// Plain `fn` pointers rather than boxed closures: every row of the grid needs
/// its own copy of the mapping, and a `Copy` struct keeps that free. The pane
/// travels as data on the reported [`GridPointerTarget::pane`] rather than as a
/// captured id, so a split leaf is as cheap to wire as the single surface.
///
/// `Clone`/`Copy` are hand-written: a derive would demand `Message: Copy`,
/// which no app message is, though these fields are only function pointers.
#[derive(Debug)]
pub(crate) struct GridPointerHooks<Message> {
    /// A press that landed on a rendered row.
    pub press: fn(GridPointerTarget) -> Message,
    /// Motion while a press that started on a row is still held.
    pub drag: fn(GridPointerTarget) -> Message,
    /// The release that ends that gesture.
    pub release: fn(GridPointerTarget) -> Message,
    /// The pane every report from this grid is stamped with. `None` for the
    /// unscoped single/detached surface; `Some(index)` for one split leaf.
    pub pane: Option<usize>,
}

impl<Message> Clone for GridPointerHooks<Message> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Message> Copy for GridPointerHooks<Message> {}

/// The sub-cell remainder at the right edge of a pane.
///
/// `cols = floor(pane_width / cell_width)` is exact by design — the shell is
/// never told about a partial column — which leaves `pane_width - cols *
/// cell_width` px between the last cell and the viewport edge. On the primary
/// screen that strip is the canvas and looks right. Under a full-screen TUI
/// that paints an explicit cell background (commonly ANSI black over the whole
/// row) the strip stays canvas-coloured, and the user sees a background stripe
/// down the right-hand side of `btop`.
///
/// This is the width of that strip. It is a **paint** quantity only: no caller
/// derives a column count from it, so cell coordinates are untouched.
#[cfg(test)]
pub(crate) fn edge_remainder_px(
    viewport_width: f32,
    visible_columns: usize,
    cell_width: f32,
) -> f32 {
    if !viewport_width.is_finite() || cell_width <= 0.0 {
        return 0.0;
    }
    (viewport_width - visible_columns as f32 * cell_width).max(0.0)
}

/// The colour the sub-cell remainder of one row must be filled with, or `None`
/// to leave the canvas showing.
///
/// `Some` only when BOTH hold:
///
///  * the row actually reaches the pane's full cell capacity — a short shell
///    line must not smear its last word's highlight to the window edge; and
///  * the rightmost visible cell carries an EXPLICIT background ink source, so
///    the fill is the terminal's own paint extended, never an invented colour.
///
/// Resolution stays at draw time: the stored [`CellInk`] is resolved here, on
/// this frame, against the active scheme; a cached pixel would stop a scheme
/// change from recolouring history.
#[cfg(test)]
pub(crate) fn edge_fill_ink(line: &TranscriptLine, visible_columns: usize) -> Option<(u8, u8, u8)> {
    if visible_columns == 0 || line.cells.len() < visible_columns {
        return None;
    }
    resolve_bg_ink(line.cells[visible_columns - 1].bg)
}

/// The selection highlight colour, derived from the active colour scheme.
///
/// Uses the scheme's foreground at reduced opacity so it reads as a
/// translucent overlay on EVERY terminal background — whether the default
/// canvas, a reverse-video block, or a TUI meter fill. Taking it from the
/// scheme (not a theme literal) means it follows scheme changes and never
/// vanishes against one particular canvas.
pub(crate) fn selection_fill() -> Color {
    let fg = crate::color_scheme::active().default_fg;
    Color::from_rgba8(fg.0, fg.1, fg.2, 0.30)
}

/// A single find-match highlight, in **cell coordinates** already resolved to
/// the visible window's row indices.
///
/// `row` is the index into the `lines` slice passed to [`terminal_grid`], not
/// the transcript row — the caller has already windowed. `start_cell` and
/// `end_cell` are half-open indices into the row's `TranscriptCell` vector,
/// **not** byte offsets in the row's flattened `text` string. This is the
/// grid's native coordinate space; the app owns byte-to-cell resolution before
/// it hands us this vec so a match on multi-byte content still paints the
/// exact cells the shell drew.
///
/// `active` distinguishes the ONE match the user has selected (paints with
/// [`active_match_fill`]) from every other visible match (paints with
/// [`match_fill`]). Two colours because a lone highlight cannot answer
/// "where am I in the match list?" — the tint difference is what says which
/// hit next/previous will jump to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MatchHighlight {
    pub row: usize,
    pub start_cell: usize,
    pub end_cell: usize,
    pub active: bool,
}

/// The find-match highlight colour (inactive matches), derived from the
/// active colour scheme's foreground at low opacity. Same reasoning as
/// [`selection_fill`]: a tint taken from the scheme follows scheme changes
/// and never disappears against one particular canvas. Kept lighter than
/// [`active_match_fill`] so the active match is visually distinct — a lone
/// highlight cannot answer "which match is current?".
pub(crate) fn match_fill() -> Color {
    let fg = crate::color_scheme::active().default_fg;
    Color::from_rgba8(fg.0, fg.1, fg.2, 0.22)
}

/// The active find-match highlight colour — the ONE match Enter/next will
/// move away from. Stronger than [`match_fill`] so the current match reads as
/// selected inside the set. Alpha chosen to remain legible against every
/// scheme's canvas (Solarized Light, Dracula, Gruvbox, Tango, Nord) rather
/// than being tuned to any one.
pub(crate) fn active_match_fill() -> Color {
    let fg = crate::color_scheme::active().default_fg;
    Color::from_rgba8(fg.0, fg.1, fg.2, 0.55)
}

/// An empty placeholder terminal fill (used before a PTY has projected).
pub(crate) fn empty_grid<'a, Message: 'a>() -> Element<'a, Message> {
    let bg = terminal_background();
    container(Space::new().width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(bg.into()),
            ..container::Style::default()
        })
        .into()
}
