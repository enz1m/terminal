//! The terminal surface on the framework's GPU terminal view.
//!
//! Each pane (per window) owns a [`TerminalScreen`]. The view writes the pane's
//! visible transcript window into it, with the cursor, find matches and the
//! selection already baked into the cells' colours, and shows it with
//! [`enz_terminal`]. The screen keeps the cells it was given and the renderer
//! redraws only rows that changed, so an unchanged frame uploads nothing.
//!
//! The model stays the app's: scrollback, selection, find, links, keys, IME
//! and focus are handled exactly as before. The view takes no keyboard input
//! (`keyboard(false)`) and no selection of its own (`native_selection(false)`);
//! its pointer reports become the same [`GridPointerTarget`] messages the
//! reducers already consume.

use std::cell::RefCell;
use std::collections::HashMap;

use enzui_terminal::{
    Cell, CellFlags, Cursor, CursorShape, MouseAction, Rgba, TerminalEvent, TerminalFont,
    TerminalScreen, TerminalStyle, enz_terminal,
};
use enzui_widgets::text_grid::TextGridEdge;
use iced::widget::container;
use iced::{Color, Element, Length};
use termwiz::cell::unicode_column_width;

use crate::grid_view::{
    self, GridPointerHooks, GridPointerTarget, MatchHighlight, resolve_bg_ink, resolve_fg_ink,
};
use crate::model::{LineTone, Settings, TranscriptCell, TranscriptLine, TranscriptRow, cell_attr};
use crate::selection::SelectionRange;

/// Which surface a screen belongs to: a pane, in the main window (`None`) or
/// in one detached window.
pub(crate) type ScreenKey = (Option<iced::window::Id>, String);

/// The screens of every painted pane, created on first paint.
#[derive(Default)]
pub(crate) struct Screens {
    screens: RefCell<HashMap<ScreenKey, TerminalScreen>>,
}

impl Screens {
    /// The screen for `key`, created blank on first use.
    pub(crate) fn get(&self, key: &ScreenKey, fg: Rgba) -> TerminalScreen {
        self.screens
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| TerminalScreen::new(80, 24, fg))
            .clone()
    }

    /// Every cell character of the screens painted in `window`, in a stable
    /// order: what the window's terminals show, for tests that compare
    /// frames without a GPU (the view draws nothing in software).
    #[cfg(test)]
    pub(crate) fn contents(&self, window: Option<iced::window::Id>) -> Vec<u8> {
        let screens = self.screens.borrow();
        let mut keys: Vec<_> = screens.keys().filter(|key| key.0 == window).collect();
        keys.sort_by(|a, b| a.1.cmp(&b.1));
        let mut out = Vec::new();
        for key in keys {
            screens[key].read(|state| {
                for y in 0..state.rows() {
                    for cell in state.row(y) {
                        out.extend_from_slice(&(cell.ch as u32).to_le_bytes());
                    }
                }
            });
        }
        out
    }

    /// The foreground and background of every cell of the row whose text is
    /// `text`, in the screens painted in `window`.
    #[cfg(test)]
    pub(crate) fn row_colours(&self, window: Option<iced::window::Id>, text: &str) -> Vec<u8> {
        let screens = self.screens.borrow();
        for (key, screen) in screens.iter() {
            if key.0 != window {
                continue;
            }
            let found = screen.read(|state| {
                (0..state.rows())
                    .find(|&y| state.row_text(y).trim_end() == text)
                    .map(|y| {
                        state
                            .row(y)
                            .iter()
                            .flat_map(|cell| cell.fg.0.into_iter().chain(cell.bg.0))
                            .collect()
                    })
            });
            if let Some(colours) = found {
                return colours;
            }
        }
        panic!("no screen in {window:?} shows the row {text:?}");
    }

    /// Forgets screens whose pane no longer exists.
    #[cfg(test)]
    pub(crate) fn retain(&self, live: impl Fn(&ScreenKey) -> bool) {
        self.screens.borrow_mut().retain(|key, _| live(key));
    }
}

fn rgba((r, g, b): (u8, u8, u8)) -> Rgba {
    Rgba::rgb(r, g, b)
}

fn color_rgba(color: Color) -> Rgba {
    let [r, g, b, a] = color.into_rgba8();
    Rgba([r, g, b, a])
}

/// `fill` (translucent) composited over the opaque `under`.
fn over(fill: Color, under: (u8, u8, u8)) -> Rgba {
    let mix = |f: f32, u: u8| (f * fill.a * 255.0 + f32::from(u) * (1.0 - fill.a)).round() as u8;
    Rgba::rgb(
        mix(fill.r, under.0),
        mix(fill.g, under.1),
        mix(fill.b, under.2),
    )
}

/// The font the terminal draws with: `family` (the app theme's monospace
/// font; `None` follows the published desktop default) at the configured size.
pub(crate) fn terminal_font(font_size: f32, family: Option<String>) -> TerminalFont {
    TerminalFont {
        family,
        size: font_size,
        line_height: 1.0,
    }
}

/// The app theme's monospace family, by name.
pub(crate) fn theme_monospace(theme: &enzui_theme::EnzTheme) -> Option<String> {
    match theme.font_catalog.family(enzui_theme::FontRole::Monospace) {
        enzui_theme::FontFamily::Named(name) => Some(name.clone()),
        enzui_theme::FontFamily::Generic(_) => None,
    }
}

/// `family`, or the concrete family the generic monospace maps to now.
pub(crate) fn concrete_monospace(family: Option<String>) -> String {
    family.unwrap_or_else(|| {
        terminal_font(12.0, None)
            .concrete()
            .family
            .unwrap_or_default()
    })
}

/// How the terminal looks under the active colour scheme, in `family`.
pub(crate) fn terminal_style(settings: &Settings, family: Option<String>) -> TerminalStyle {
    let scheme = crate::color_scheme::active();
    TerminalStyle {
        font: terminal_font(settings.resolved_font_size_px(), family),
        background: rgba(scheme.default_bg),
        foreground: rgba(scheme.default_fg),
        cursor: rgba(scheme.cursor),
        cursor_text: rgba(scheme.default_bg),
        selection_background: color_rgba(grid_view::selection_fill()),
        selection_foreground: None,
        // The app's own inset surrounds the view (see `terminal_grid`).
        padding: 0.0,
        cursor_blink: None,
        // A row's last background reaches the pane edge, so a full-screen
        // TUI's colour leaves no strip beside the grid.
        extend_background: true,
    }
}

/// The foreground of a cell that names no colour: its semantic tone on the
/// scheme, with Muted and Subtle drawn dim.
fn tone_ink(tone: LineTone) -> ((u8, u8, u8), bool) {
    let scheme = crate::color_scheme::active();
    let ansi = |i| crate::color_scheme::resolve_ansi_index_for(scheme, i);
    match tone {
        LineTone::Default => (scheme.default_fg, false),
        LineTone::Muted | LineTone::Subtle => (scheme.default_fg, true),
        LineTone::Accent => (ansi(4), false),
        LineTone::Success => (ansi(2), false),
        LineTone::Warning => (ansi(3), false),
        LineTone::Error => (ansi(1), false),
    }
}

/// One transcript cell as a terminal cell: colours resolved against the
/// active scheme exactly as the old grid resolved them (background first,
/// then the foreground's contrast floor against it).
pub(crate) fn term_cell(cell: &TranscriptCell) -> (Cell, (u8, u8, u8)) {
    let text = cell.text.as_str();
    let mut chars = text.chars();
    let first = chars.next();
    view_cell(
        CellParts {
            ch: first.unwrap_or(' '),
            combining: chars.as_str(),
            blank: cell.blank || first.is_none(),
            bold: cell.bold,
            tone: cell.tone,
            fg: cell.fg,
            bg: cell.bg,
            attrs: cell.attrs,
        },
        || unicode_column_width(text, None) > 1,
    )
}

/// What one cell is made of, from a transcript cell or straight from the
/// emulator; [`view_cell`] turns it into the view's cell either way.
pub(crate) struct CellParts<'a> {
    pub ch: char,
    pub combining: &'a str,
    pub blank: bool,
    pub bold: bool,
    pub tone: LineTone,
    pub fg: crate::model::CellInk,
    pub bg: crate::model::CellInk,
    pub attrs: u8,
}

/// A cell's colours resolved against the active scheme (background first,
/// then the foreground's contrast floor against it), its glyph and flags;
/// also the opaque colour under it. `wide` is asked only for a glyph.
pub(crate) fn view_cell(parts: CellParts<'_>, wide: impl FnOnce() -> bool) -> (Cell, (u8, u8, u8)) {
    let scheme = crate::color_scheme::active();
    let bg = resolve_bg_ink(parts.bg);
    let effective_bg = bg.unwrap_or(scheme.default_bg);
    let (fg, dim) = match resolve_fg_ink(parts.fg, effective_bg) {
        Some(fg) => (fg, matches!(parts.tone, LineTone::Subtle)),
        None => tone_ink(parts.tone),
    };
    let mut flags = CellFlags::NONE;
    if dim {
        flags |= CellFlags::DIM;
    }
    let mut out = Cell::new(' ', rgba(fg));
    if !parts.blank {
        out.ch = parts.ch;
        if !parts.combining.is_empty() {
            out.combining = Some(parts.combining.into());
        }
        if parts.bold {
            flags |= CellFlags::BOLD;
        }
        if wide() {
            flags |= CellFlags::WIDE;
        }
    }
    if let Some(bg) = bg {
        out.bg = rgba(bg);
    }
    out.flags = flags | attr_flags(parts.attrs);
    (out, effective_bg)
}

/// The view's flags for a cell's [`cell_attr`] bits.
pub(crate) fn attr_flags(attrs: u8) -> CellFlags {
    let mut flags = CellFlags::NONE;
    for (bit, flag) in [
        (cell_attr::ITALIC, CellFlags::ITALIC),
        (cell_attr::UNDERLINE, CellFlags::UNDERLINE),
        (cell_attr::DOUBLE_UNDERLINE, CellFlags::DOUBLE_UNDERLINE),
        (cell_attr::CURLY_UNDERLINE, CellFlags::CURLY_UNDERLINE),
        (cell_attr::DOTTED_UNDERLINE, CellFlags::DOTTED_UNDERLINE),
        (cell_attr::DASHED_UNDERLINE, CellFlags::DASHED_UNDERLINE),
        (cell_attr::STRIKETHROUGH, CellFlags::STRIKETHROUGH),
        (cell_attr::HIDDEN, CellFlags::HIDDEN),
    ] {
        if attrs & bit != 0 {
            flags |= flag;
        }
    }
    flags
}

/// What one painted window looks like, beyond its transcript rows.
pub(crate) struct Overlays<'s> {
    pub focused: bool,
    /// Whether the program shows its cursor (it is drawn only when focused).
    pub cursor_visible: bool,
    pub selection: Option<SelectionRange>,
    pub window_start_row: usize,
    pub matches: &'s [MatchHighlight],
}

/// The cells of one window row: transcript cells, wide-character spacers,
/// then the find fills and the selection on top.
pub(crate) fn row_cells(
    line: &TranscriptLine,
    row_index: usize,
    overlays: &Overlays<'_>,
    fills: &Fills,
) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(line.cells.len());
    let mut under = Vec::with_capacity(line.cells.len());
    let mut spacer = false;
    for cell in &line.cells {
        let (mut out, bg) = term_cell(cell);
        if spacer && cell.blank {
            // The continuation cell behind a double-width glyph.
            out.flags |= CellFlags::WIDE_SPACER;
        }
        spacer = out.flags.contains(CellFlags::WIDE);
        cells.push(out);
        under.push(bg);
    }
    for hit in overlays.matches.iter().filter(|hit| hit.row == row_index) {
        let fill = if hit.active {
            fills.active_match
        } else {
            fills.find
        };
        let end = hit.end_cell.min(cells.len());
        for col in hit.start_cell.min(end)..end {
            cells[col].bg = over(fill, under[col]);
            under[col] = rgb_of(cells[col].bg);
        }
    }
    if let Some(selection) = &overlays.selection {
        let row = (overlays.window_start_row + row_index) as i64;
        for (col, cell) in cells.iter_mut().enumerate() {
            if selection.contains(row, col) {
                cell.bg = over(fills.selection, under[col]);
            }
        }
    }
    cells
}

fn rgb_of(c: Rgba) -> (u8, u8, u8) {
    (c.0[0], c.0[1], c.0[2])
}

/// The translucent fills, looked up once per paint.
pub(crate) struct Fills {
    selection: Color,
    find: Color,
    active_match: Color,
}

impl Fills {
    pub(crate) fn current() -> Self {
        Self {
            selection: grid_view::selection_fill(),
            find: grid_view::match_fill(),
            active_match: grid_view::active_match_fill(),
        }
    }
}

pub(crate) fn cursor_shape(style: &str) -> CursorShape {
    match style {
        "underline" => CursorShape::Underline,
        "bar" => CursorShape::Beam,
        _ => CursorShape::Block,
    }
}

/// Writes one window of transcript rows into `screen`, sized `cols` by `rows`,
/// below the `rows_above` rows before the window (`above`, the newest last),
/// which the view draws above its bounds.
/// Rows that did not change are compared and left alone, so the renderer
/// redraws only what moved.
pub(crate) fn sync_screen(
    screen: &TerminalScreen,
    (above, rows_above): (&[TranscriptRow], u16),
    lines: &[TranscriptRow],
    (cols, rows): (u16, u16),
    cursor_style: &str,
    overlays: &Overlays<'_>,
) {
    let scheme = crate::color_scheme::active();
    let fills = Fills::current();
    // The rows above show only output: no selection, matches or cursor.
    let plain = Overlays {
        focused: false,
        cursor_visible: false,
        selection: None,
        window_start_row: 0,
        matches: &[],
    };
    let above = &above[above.len().saturating_sub(usize::from(rows_above))..];
    let blank_above = usize::from(rows_above) - above.len();
    let built_above: Vec<Vec<Cell>> = above
        .iter()
        .enumerate()
        .map(|(y, line)| row_cells(line.as_ref(), y, &plain, &fills))
        .collect();
    let built: Vec<Vec<Cell>> = lines
        .iter()
        .take(usize::from(rows))
        .enumerate()
        .map(|(y, line)| row_cells(line.as_ref(), y, overlays, &fills))
        .collect();
    // The prompt row carries the shell's cursor; it shows only in the focused
    // pane, as before.
    let cursor = lines
        .iter()
        .take(usize::from(rows))
        .position(|line| line.is_prompt)
        .map(|y| (y, lines[y].cursor_col.max(0)));
    screen.update(|w| {
        w.set_default_fg(rgba(scheme.default_fg));
        w.resize(cols, rows_above + rows);
        for y in 0..rows_above {
            match usize::from(y)
                .checked_sub(blank_above)
                .and_then(|index| built_above.get(index))
            {
                Some(cells) => w.set_row(y, cells.iter().cloned()),
                None => w.set_row(y, std::iter::empty()),
            }
        }
        for y in 0..rows {
            match built.get(usize::from(y)) {
                Some(cells) => w.set_row(rows_above + y, cells.iter().cloned()),
                None => w.set_row(rows_above + y, std::iter::empty()),
            }
        }
        w.set_cursor(match cursor {
            Some((row, col)) => Cursor {
                col: u16::try_from(col)
                    .unwrap_or(u16::MAX)
                    .min(cols.saturating_sub(1)),
                row: rows_above + row as u16,
                shape: cursor_shape(cursor_style),
                visible: overlays.focused && overlays.cursor_visible,
            },
            None => Cursor {
                col: 0,
                row: 0,
                shape: cursor_shape(cursor_style),
                visible: false,
            },
        });
    });
}

/// A pointer report on the view as the grid's [`GridPointerTarget`]: the
/// row is signed (a drag above or below the grid keeps selecting into
/// scrollback), and past the end of a row's content is `AfterLast` on its
/// last real column, as the old grid reported it.
fn pointer_target(
    lines: &[TranscriptRow],
    col: u16,
    row: i32,
    pane: Option<usize>,
) -> GridPointerTarget {
    let len = usize::try_from(row)
        .ok()
        .and_then(|row| lines.get(row))
        .map_or(0, |line| line.cells.len());
    let col = usize::from(col);
    let (column, edge) = if len == 0 {
        (0, TextGridEdge::AfterLast)
    } else if col >= len {
        (len - 1, TextGridEdge::AfterLast)
    } else {
        (col, TextGridEdge::Inside)
    };
    GridPointerTarget {
        window_row: row,
        column,
        edge,
        pane,
    }
}

/// How strongly rows drawn above a view fade into the canvas at their top.
const UNDER_TITLEBAR_FADE: f32 = 0.95;

/// Where the view's events go.
pub(crate) struct Routes<'a, Message> {
    /// Pointer gestures, as grid cell reports.
    pub pointer: Option<GridPointerHooks<Message>>,
    /// Wheel movement, in rows (positive towards newer output); `None` leaves
    /// an unfocused leaf inert.
    pub wheel: Option<Box<dyn Fn(f32) -> Message + 'a>>,
    /// The grid the view fits, in cells.
    pub fit: Box<dyn Fn(u16, u16) -> Message + 'a>,
    /// Events the app does not act on.
    pub ignored: Message,
}

/// The terminal view of one pane window: writes `lines` into `screen` and
/// shows it, inset by `inset` on the scheme's canvas, with the rows before
/// the window in `above` drawn over the space above it, `rows_above` of
/// them (see [`sync_screen`]).
#[allow(
    clippy::too_many_arguments,
    reason = "A terminal frame combines independent screen, grid, settings, overlay and route inputs."
)]
pub(crate) fn terminal_grid<'a, Message: Clone + 'a>(
    screen: &TerminalScreen,
    (above, rows_above): (&[TranscriptRow], u16),
    lines: &'a [TranscriptRow],
    settings: &Settings,
    family: Option<String>,
    grid: (u16, u16),
    overlays: Overlays<'_>,
    routes: Routes<'a, Message>,
    inset: f32,
    sync: bool,
) -> Element<'a, Message> {
    // A live pane's screen is written by its PTY reader (see `App::live`).
    if sync {
        sync_screen(
            screen,
            (above, rows_above),
            lines,
            grid,
            settings.resolved_cursor_style(),
            &overlays,
        );
    }
    let Routes {
        pointer,
        wheel,
        fit,
        ignored,
    } = routes;
    let pane = pointer.and_then(|hooks| hooks.pane);
    let view = enz_terminal(screen)
        .style(terminal_style(settings, family))
        .keyboard(false)
        .focused(overlays.focused)
        .native_selection(false)
        .rows_above(rows_above)
        // Under a floating titlebar: faded so the bar stays legible.
        .fade_rows_above(UNDER_TITLEBAR_FADE)
        .on_event(move |event| match event {
            // The rows above are the view's too; the window is the rest.
            TerminalEvent::Resize { cols, rows } => fit(cols, rows.saturating_sub(rows_above)),
            TerminalEvent::Mouse {
                action,
                col,
                unclamped_row,
                ..
            } => {
                let Some(hooks) = pointer else {
                    return ignored.clone();
                };
                let row = unclamped_row - i32::from(rows_above);
                let target = pointer_target(lines, col, row, pane);
                match action {
                    MouseAction::Press(iced::mouse::Button::Left) => (hooks.press)(target),
                    MouseAction::Drag(iced::mouse::Button::Left) => (hooks.drag)(target),
                    MouseAction::Release(iced::mouse::Button::Left) => (hooks.release)(target),
                    _ => ignored.clone(),
                }
            }
            TerminalEvent::Scroll { lines: delta, .. } => match &wheel {
                Some(wheel) => wheel(-(delta as f32)),
                None => ignored.clone(),
            },
            _ => ignored.clone(),
        });
    let background = grid_view::terminal_background();
    let inset = if inset.is_finite() && inset > 0.0 {
        inset
    } else {
        0.0
    };
    let content: Element<'a, Message> = view.into();
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(inset)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(background.into()),
            ..container::Style::default()
        })
        .into()
}
