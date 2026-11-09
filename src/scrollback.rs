//! Per-pane scrollback position and its wheel routing.
//!
//! # The defect this exists to remove
//!
//! Before this module the terminal had no scrollback *at all*: the grid was not
//! inside a scroll owner, no pane carried a scroll offset, and no wheel event
//! was mapped anywhere in the app. `App::grid_window` only ever chose a
//! bottom-pinned slice (or a find-target slice), so output that scrolled off the
//! top was in the backend ring buffer and permanently unreachable from the UI.
//!
//! # Why the offset is CALLER state and there is no `enz_scroll_view`
//!
//! The grid realises only the rows it is handed: a 5 000-row transcript is
//! sliced to the ~40 rows that fit before a single `enz_text_grid` is built.
//! That slicing decision needs the offset *before* layout, so the offset is
//! per-pane app state ([`ScrollPosition`] on [`crate::model::Pane`]) and the
//! grid stays outside any scroll owner. Wrapping the grid in `enz_scroll_view`
//! would put the position in the scroll view while the realisation window stayed
//! here — two owners of one number — and it would have to realise every row to
//! measure the content, which is exactly the cost the slice avoids.
//!
//! # The overlay chrome comes from the framework
//!
//! The overlay scrollbar widget itself is the framework's
//! [`enzui_widgets::enz_offset_scrollbar`] (layout-transparent, offset
//! owned by the caller, wheel reported not applied, thumb ink resolved against
//! a caller-supplied backdrop). The app composes it directly in [`crate::main`]
//! and this module keeps only the state (position, geometry, wheel routing)
//! that is terminal-specific.
//!
//! What is consumed rather than re-picked:
//!
//! * [`enzui_widgets::enz_offset_scrollbar`] — the overlay thumb, drag
//!   capture, lane hover reveal, offset conversion and backdrop-resolved ink.
//! * [`enzui_theme::ScrollbarTokens`] — the reveal/grab lane, painted
//!   thumb, minimum thumb length, capsule radius and both alphas.
//! * [`enzui_widgets::tail_follow::is_at_end`] /
//!   [`enzui_widgets::tail_follow::distance_from_end`] — the framework's
//!   "is the reader at the bottom" judgement, so tail-follow releases on the
//!   same rule chat surfaces use.
//!
//! # The tail-follow rule
//!
//! [`ScrollPosition`] stores the **first visible row**, not a distance from the
//! bottom, and `None` means "pinned to the tail". Both halves of the rule fall
//! out of that representation:
//!
//! * pinned → the first visible row is recomputed from the *new* length every
//!   frame, so appended output follows the tail;
//! * released → the stored row index is unchanged by an append, so the reader's
//!   viewport is preserved and [`ScrollPosition::is_at_bottom`] is `false`,
//!   which is what raises the jump-to-latest affordance.
//!
//! # Alternate screen
//!
//! A full-screen TUI (`htop`, `btop`, `vim`) owns the wheel: the primary
//! scrollback is not its timeline, and the backend deliberately records no
//! scrollback while alt-screen is engaged. [`WheelRoute::resolve`] therefore
//! routes a wheel notch on an alt-screen pane to the **PTY** (xterm's
//! alternate-scroll arrows, built from [`crate::keymap`] so the bytes are not a
//! second copy) and leaves the transcript offset untouched. This is exactly
//! what [`enz_offset_scrollbar`]'s "wheel reported not applied" property lets
//! the caller do: the framework widget publishes the raw `ScrollDelta`, the app
//! banks the fractional residue and asks `WheelRoute` where it goes.
//!
//! [`enz_offset_scrollbar`]: enzui_widgets::enz_offset_scrollbar

use enzui_theme::ScrollbarTokens;
use iced::mouse;

/// Rows one wheel notch moves, matching the three-line notch xterm and
/// gnome-terminal use for scrollback.
pub(crate) const WHEEL_ROWS_PER_NOTCH: f32 = 3.0;

/// Invisible reveal/grab lane width — the framework token, not a re-typed 12.
///
/// Retained here so `main.rs` and the scrollback visual gates can address the
/// same lane geometry the framework's [`enz_offset_scrollbar`] paints.
/// Delegates to [`ScrollbarTokens`] so a token retune moves both together.
///
/// [`enz_offset_scrollbar`]: enzui_widgets::enz_offset_scrollbar
pub(crate) fn hit_width_px() -> f32 {
    ScrollbarTokens::DEFAULT.hit_width
}

// ---------------------------------------------------------------------------
// Geometry: the three numbers a scrollbar range is made of
// ---------------------------------------------------------------------------

/// The transcript geometry a scroll position is resolved against: how many rows
/// exist **in total**, how many fit, and how tall one row is.
///
/// `total_rows` is the full transcript length (backend scrollback ring +
/// on-screen rows), never the visible slice: a scrollbar whose range is the
/// visible slice always looks full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollGeometry {
    pub total_rows: usize,
    pub visible_rows: usize,
    pub cell_height: f32,
}

impl ScrollGeometry {
    pub(crate) fn new(total_rows: usize, visible_rows: usize, cell_height: f32) -> Self {
        Self {
            total_rows,
            visible_rows,
            cell_height,
        }
    }

    /// The largest first-visible row: the bottom-pinned window's start.
    pub(crate) fn max_top_row(&self) -> usize {
        self.total_rows.saturating_sub(self.visible_rows)
    }

    /// Full content height in px — `total_rows * cell_height`.
    pub(crate) fn content_height(&self) -> f32 {
        self.total_rows as f32 * self.cell_height
    }

    /// Viewport height in px — `visible_rows * cell_height`.
    pub(crate) fn viewport_height(&self) -> f32 {
        self.visible_rows as f32 * self.cell_height
    }

    /// Is there anything to scroll?
    #[cfg(test)]
    pub(crate) fn overflowing(&self) -> bool {
        self.total_rows > self.visible_rows && self.visible_rows > 0
    }

    /// One page, in rows: a viewport minus one row of overlap so the reader
    /// keeps a line of context across a PageUp.
    pub(crate) fn page_rows(&self) -> usize {
        self.visible_rows.saturating_sub(1).max(1)
    }
}

// ---------------------------------------------------------------------------
// ScrollPosition — one offset, three gestures
// ---------------------------------------------------------------------------

/// A pane's scrollback position: the first visible transcript row, or `None`
/// while pinned to the tail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScrollPosition {
    top: Option<usize>,
}

impl ScrollPosition {
    /// Pinned to the newest output (the state a fresh pane opens in).
    pub(crate) fn pinned_to_tail() -> Self {
        Self { top: None }
    }

    /// The position that renders `row` as the first visible row.
    ///
    /// Used by the view to hand the chrome the window it is ACTUALLY painting,
    /// so the thumb cannot disagree with the rows on screen (a find target is a
    /// transient override of the same window).
    pub(crate) fn from_top_row(row: usize, geom: ScrollGeometry) -> Self {
        let mut p = Self::pinned_to_tail();
        p.set_top_row(row, geom);
        p
    }

    /// `true` while the tail is being followed — the stored pin, not the pixel
    /// judgement [`Self::resolves_to_end`] that raises the jump-to-latest
    /// affordance.
    ///
    /// Read only by the `scrollback_app_tests` gates that distinguish viewport
    /// preservation from tail follow; [`Self::top_row`] resolves a pinned
    /// position to the tail for production. Kept as the model's statement of
    /// the pin with a scoped `#[allow]` rather than a module blanket.
    #[allow(
        dead_code,
        reason = "Scrollback diagnostics expose retained row information beyond the active viewport."
    )]
    pub(crate) fn is_at_bottom(&self) -> bool {
        self.top.is_none()
    }

    /// The first row the grid should render.
    ///
    /// Pinned resolves against the CURRENT length, which is tail-follow; a
    /// released position is clamped but otherwise untouched by an append, which
    /// is viewport preservation.
    pub(crate) fn top_row(&self, geom: ScrollGeometry) -> usize {
        let max = geom.max_top_row();
        match self.top {
            None => max,
            Some(t) => t.min(max),
        }
    }

    /// Move to an absolute first-visible row. Landing at (or past) the bottom
    /// re-acquires the tail pin, so scrolling back down resumes following —
    /// the framework's `rejoin` rule, applied to rows.
    pub(crate) fn set_top_row(&mut self, row: usize, geom: ScrollGeometry) {
        let max = geom.max_top_row();
        if row >= max {
            self.top = None;
        } else {
            self.top = Some(row);
        }
    }

    /// Move by `delta` rows; positive is toward the newest output. The ONE
    /// mutation the wheel, PageUp/PageDown and a scrollbar drag all go through.
    pub(crate) fn scroll_by_rows(&mut self, delta: i64, geom: ScrollGeometry) {
        let current = self.top_row(geom) as i64;
        let max = geom.max_top_row() as i64;
        let next = (current + delta).clamp(0, max);
        self.set_top_row(next as usize, geom);
    }

    /// Re-pin to the newest output (the jump-to-latest affordance).
    pub(crate) fn jump_to_tail(&mut self) {
        self.top = None;
    }

    /// The scroll offset in px, for the scrollbar chrome.
    pub(crate) fn scroll_top_px(&self, geom: ScrollGeometry) -> f32 {
        self.top_row(geom) as f32 * geom.cell_height
    }

    /// Distance from the tail in px, through the framework's own judgement
    /// (`tail_follow::distance_from_end`), so "at the bottom" is not defined
    /// twice in this workspace.
    pub(crate) fn distance_from_end_px(&self, geom: ScrollGeometry) -> f32 {
        enzui_widgets::tail_follow::distance_from_end(
            self.scroll_top_px(geom),
            geom.viewport_height(),
            geom.content_height(),
        )
    }

    /// Whether the *resolved pixel* position counts as the end, by
    /// `tail_follow::is_at_end`. Distinct from [`Self::is_at_bottom`], which is
    /// the stored pin: a released position one row off the tail is not pinned
    /// but is within the framework's release distance.
    pub(crate) fn resolves_to_end(&self, geom: ScrollGeometry) -> bool {
        enzui_widgets::tail_follow::is_at_end(self.distance_from_end_px(geom))
    }
}

/// Convert a pixel offset into the first visible row (round to nearest, so a
/// drag of exactly N rows' worth of travel lands on N rows).
///
/// This is the adapter between the framework's pixel-typed offset
/// (`enz_offset_scrollbar` publishes `f32`) and the row-typed offset the
/// terminal reducer works with. Rounding rather than truncating means a drag
/// of exactly N rows' worth of thumb travel lands on N rows, which is the
/// property `wheel_page_and_drag_converge_on_one_offset` reads.
pub(crate) fn top_row_for_px(px: f32, geom: ScrollGeometry) -> usize {
    if !px.is_finite() || geom.cell_height <= 0.0 {
        return 0;
    }
    let rows = (px / geom.cell_height).round();
    rows.clamp(0.0, geom.max_top_row() as f32) as usize
}

/// Rows a wheel event moves, signed toward the newest output.
///
/// The sign convention is upstream's: `iced` applies `-y` to a scrollable's
/// offset for both delta kinds (`vendor/iced/widget/src/scrollable.rs`),
/// so wheel-up (positive `y`) moves toward the top.
///
/// [`enz_offset_scrollbar`] publishes the raw `ScrollDelta`; this converts it
/// to the signed rows the reducer, residue banking and [`WheelRoute::resolve`]
/// all consume.
///
/// [`enz_offset_scrollbar`]: enzui_widgets::enz_offset_scrollbar
pub(crate) fn wheel_rows(delta: mouse::ScrollDelta, cell_height: f32) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => -y * WHEEL_ROWS_PER_NOTCH,
        mouse::ScrollDelta::Pixels { y, .. } => {
            if cell_height > 0.0 {
                -y / cell_height
            } else {
                0.0
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Wheel routing: transcript vs PTY
// ---------------------------------------------------------------------------

/// The pane state a wheel routing decision depends on: which screen the shell is
/// on, and which cursor-key encoding it is listening for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WheelTarget {
    /// The alternate screen is engaged: the running program owns the wheel.
    pub alt_screen: bool,
    /// DECCKM is engaged, so a synthesised cursor key must be `ESC O X`.
    pub application_cursor_keys: bool,
    /// The pane's live mouse-tracking pair. When it tracks, the wheel is
    /// the APPLICATION's, ahead of BOTH alternate-scroll and scrollback.
    pub mouse_protocol: crate::mouse::MouseProtocol,
    /// Shift is the escape hatch — a Shift+wheel reaches scrollback even
    /// while an application tracks the mouse.
    pub mouse_shift: bool,
    /// The 1-based cell a wheel report carries (the last pointer cell).
    pub mouse_cell: (usize, usize),
}

/// Where a wheel gesture on a pane belongs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WheelRoute {
    /// Primary screen: move this pane's scrollback offset by N rows.
    Transcript(i64),
    /// Alternate screen: the TUI owns the wheel. These bytes go to the PTY and
    /// the transcript offset is not touched.
    Pty(Vec<u8>),
    /// Nothing to do (a sub-row trackpad delta that has not accumulated yet).
    Nothing,
}

impl WheelRoute {
    /// The routing decision, from the pane's live backend state.
    pub(crate) fn resolve(target: WheelTarget, rows: i64) -> Self {
        if rows == 0 {
            return Self::Nothing;
        }
        // Precedence, highest first:
        //   1. an application tracking the mouse owns the wheel (it is a mouse
        //      report), unless Shift asks for scrollback instead;
        //   2. the alternate screen's xterm alternate-scroll cursor keys;
        //   3. the primary screen's own scrollback.
        // The order matters: a full-screen TUI can both be on the alt screen
        // and track the mouse (`htop`), and then the report must win over the
        // alternate-scroll fallback, or the wheel does nothing inside it.
        if target.mouse_protocol.tracks() && !target.mouse_shift {
            let (col, row) = target.mouse_cell;
            return match target.mouse_protocol.wheel_bytes(rows, col, row) {
                Some(bytes) => Self::Pty(bytes),
                None => Self::Nothing,
            };
        }
        if target.alt_screen {
            match alt_screen_wheel_bytes(rows, target.application_cursor_keys) {
                Some(bytes) => Self::Pty(bytes),
                None => Self::Nothing,
            }
        } else {
            Self::Transcript(rows)
        }
    }
}

/// xterm's `alternateScroll`: on the alternate screen a wheel notch becomes
/// cursor-key presses, so `less`/`man`/`btop` page as the user expects.
///
/// `application_cursor_keys` is the pane's live DECCKM state: `less`, `vim` and
/// `htop` all set it (terminfo `smkx`) and then listen for `ESC O B`, so sending
/// the normal-mode `ESC [ B` is a wheel that silently does nothing. The bytes
/// come from [`crate::keymap::cursor_key_bytes`] rather than being spelled again
/// here, so the wheel and the arrow keys cannot drift apart.
pub(crate) fn alt_screen_wheel_bytes(rows: i64, application_cursor_keys: bool) -> Option<Vec<u8>> {
    use iced::keyboard::key::Named;
    let named = if rows > 0 {
        Named::ArrowDown
    } else {
        Named::ArrowUp
    };
    let one = crate::keymap::cursor_key_bytes(named, application_cursor_keys)?;
    let count = rows.unsigned_abs() as usize;
    let mut out = Vec::with_capacity(one.len() * count);
    for _ in 0..count {
        out.extend_from_slice(&one);
    }
    Some(out)
}
