//! The terminal emulator: `alacritty_terminal` behind the app's screen API.
//!
//! The PTY reader feeds bytes with [`Screen::process`]; the UI thread reads
//! the transcript with [`Screen::project`]. Parsing and the VT state machine
//! are Alacritty's. Around it this module keeps what the app owns:
//!
//! - **Scrollback.** The emulator's own history is only a hand-off buffer:
//!   after each fed segment the rows that scrolled off the screen are copied
//!   into this screen's bounded, deferred [`ScrollbackRow`]s and the buffer is
//!   cleared. That captures every scroll-off, line feeds and autowrap alike,
//!   with stable `scrollback-N` ids, without the emulator's history growing.
//! - **Policy.** Titles, links and replies pass the same `escape_policy`
//!   checks as before; clipboard (OSC 52), palette and size queries stay
//!   unanswered, and only the replies the app always gave are sent.
//! - **Shell integration.** OSC 7, 133 D and 777, which the emulator drops,
//!   come from [`scan::Scanner`] at their exact place in the stream.
//! - **Pinned output.** A running command's output is replayed into a shadow
//!   screen ([`Screen::output_capture_surface`]) exactly as before.

use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config, Osc52, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::escape_policy::{self, Segment};
use crate::model::{TranscriptLine, TranscriptRow};

mod handler;
mod project;
pub(crate) mod scan;

pub(crate) use project::project_cells;
use project::{RetainedRow, project_row};

/// Screen dimensions before the first resize.
pub(crate) const DEFAULT_ROWS: usize = 30;
pub(crate) const DEFAULT_COLS: usize = 120;

/// Default retained-row limit. Applications configure each reader before parsing output.
pub(crate) const MAX_SCROLLBACK: usize = 5000;

/// How often the reader writes a live pane's screen during output.
pub(crate) const LIVE_PERIOD: std::time::Duration = std::time::Duration::from_millis(8);

/// What a pane prints when it abandons an oversized control sequence.
const OVERSIZED_SEQUENCE_NOTICE: &[u8] = b"\r\n[oversized control sequence discarded]\r\n";

/// The process's link authority.
///
/// Every screen starts on its own generation and remints on each authority
/// reset, so a retained row's link can never be revived by a later screen or a
/// replacement session reaching the same value. Once it is exhausted, screens
/// go without hyperlink authority rather than share a live generation.
static LINK_GENERATIONS: escape_policy::AuthorityAllocator =
    escape_policy::AuthorityAllocator::new(escape_policy::AuthorityKind::Link);

/// App-visible side effects produced while parsing terminal control sequences.
///
/// These are deliberately separate from the cell grid: OSC 0/2 changes a tab
/// title and BEL raises an alert, neither of which should manufacture a glyph.
/// A title is coalesced to the latest value in a PTY burst and the bell count is
/// saturating, so hostile output cannot grow an unbounded event queue before
/// the UI thread drains it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalEffects {
    pub(crate) shell_events: VecDeque<crate::shell_history::ShellEvent>,
    #[cfg(test)]
    pub commands_started: VecDeque<String>,
    /// Retire old coordinate-derived state before publishing the paired cell snapshot.
    pub coordinates_invalidated: bool,
    /// Sequences whose payload outgrew [`escape_policy::MAX_STRING_SEQUENCE_BYTES`]
    /// and were discarded with the parser holding them.
    #[cfg(test)]
    pub discarded_sequences: u32,
    pub title: Option<String>,
    // OSC 7 host qualification belongs to the pane; this field retains the
    // admitted URI text unqualified.
    pub cwd: Option<String>,
    pub(crate) generated_cwd: Option<crate::shell_history::cwd::ReportedCwd>,
    #[cfg(test)]
    pub command_finished: Option<i32>,
    pub bell_count: u32,
}

/// The emulator's events, collected while it parses and handled after.
#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<Event>>>);

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        let mut events = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Bounded: a hostile burst of queries cannot grow this.
        if events.len() < 256 {
            events.push(event);
        }
    }
}

impl Events {
    fn take(&self) -> Vec<Event> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// A grid size for the emulator.
#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Where the reader writes the visible rows itself while the app shows this
/// pane following its live output (see `terminal_view`): the GPU view then
/// redraws without an app message, a transcript projection or a view rebuild
/// per frame. Rows are converted exactly as the app converts them.
pub(crate) struct LiveTarget {
    pub(crate) screen: enzui_terminal::TerminalScreen,
    pub(crate) focused: bool,
    pub(crate) cursor_style: String,
    /// Fingerprints of the rows last written, by row.
    printed: Vec<u64>,
    /// The colour scheme they were written in: a scheme change rewrites all.
    scheme: &'static str,
    last: Option<std::time::Instant>,
    /// How many of the newest scrollback rows the screen holds above the
    /// emulator's rows (the view draws them above its bounds).
    rows_above: u16,
    /// Which scrollback row each of those last showed (see [`above_print`]).
    above_printed: Vec<u64>,
}

impl LiveTarget {
    pub(crate) fn new(
        screen: enzui_terminal::TerminalScreen,
        focused: bool,
        cursor_style: &str,
    ) -> Self {
        Self {
            screen,
            focused,
            cursor_style: cursor_style.to_owned(),
            printed: Vec::new(),
            scheme: "",
            last: None,
            rows_above: 0,
            above_printed: Vec::new(),
        }
    }

    /// Puts the newest `rows` scrollback rows at the top of the screen, above
    /// the emulator's own rows.
    pub(crate) fn rows_above(mut self, rows: u16) -> Self {
        self.rows_above = rows;
        self
    }
}

/// What a row above the emulator's shows: a scrollback row is immutable, so
/// its place in the output and the width it is padded to identify it; `0`
/// is a blank row.
fn above_print(row: Option<&ScrollbackRow>, cols: usize) -> u64 {
    row.map_or(0, |row| (row.0.line.wrapping_add(1) << 16) ^ cols as u64)
}

/// A write prepared under the screen lock, applied to the view's screen
/// after the lock is released (never hold both).
pub(crate) struct LiveWrite {
    screen: enzui_terminal::TerminalScreen,
    grid: (u16, u16),
    rows: Vec<(u16, Vec<enzui_terminal::Cell>)>,
    cursor: enzui_terminal::Cursor,
    fg: enzui_terminal::Rgba,
}

impl LiveWrite {
    pub(crate) fn apply(self) {
        let Self {
            screen,
            grid,
            rows,
            cursor,
            fg,
        } = self;
        screen.update(|w| {
            w.set_default_fg(fg);
            w.resize(grid.0, grid.1);
            for (y, cells) in rows {
                w.set_row(y, cells);
            }
            w.set_cursor(cursor);
        });
    }
}

/// One visible row's projection, reused while the row is undamaged.
struct LiveRow {
    session: String,
    cursor: Option<usize>,
    /// The cells it was projected from, fingerprinted.
    cells: u64,
    projection: TranscriptRow,
}

/// Owns the emulator and the app's scrollback. Fed bytes via
/// `process(&[u8])` and read via `project(session_id)`.
pub(crate) struct Screen {
    term: Term<Events>,
    events: Events,
    parser: Processor,
    sequence_guard: escape_policy::StringSequenceGuard,
    scanner: scan::Scanner,
    /// The link authority every range projected now is stamped with.
    ///
    /// `None` once the allocator refused this screen a generation, which is
    /// terminal: no range is projected, the ranges already retained are
    /// dropped, and no reset or screen swap may mint a replacement.
    link_generation: Option<NonZeroU64>,
    pub(crate) pinned_capture: crate::pinned_output::OutputCapture,
    /// Whether the alternate screen was engaged after the last segment.
    alt_screen: bool,
    live: Vec<Option<LiveRow>>,
    live_generation: Option<NonZeroU64>,
    scrollback: VecDeque<ScrollbackRow>,
    max_scrollback: usize,
    /// Rows that ever scrolled off: the last one's id is `scrollback-N`.
    scrollback_counter: u64,
    /// The scroll region, tracked as the emulator sets it.
    region: handler::Region,
    /// Rows that scrolled off during the current parse, oldest first.
    scrolled: Vec<RetainedRow>,
    /// The primary screen's rows while the alternate screen hides them.
    primary: Option<Vec<RetainedRow>>,
    /// Backing `dirty` counter shared with the caller so the UI poll can
    /// detect changes without holding the Screen mutex.
    dirty: Arc<AtomicU64>,
    pending_effects: TerminalEffects,
    history_disabled: bool,
    active_command: Option<u64>,
    // Query-time positions and parser order survive the reader/UI handoff.
    pending_replies: VecDeque<Vec<u8>>,
    live_target: Option<LiveTarget>,
    /// Set while a live write is owed (output arrived within [`LIVE_PERIOD`]
    /// of the last one); the reader flushes it when output pauses.
    pub(crate) live_pending: Arc<std::sync::atomic::AtomicBool>,
}

/// One row that scrolled off the primary screen, with its projection deferred.
///
/// The cells are copied as they scrolled off (trailing default blanks
/// dropped), so the lazy projection is a pure function of that snapshot.
struct ScrollbackRow(Arc<ScrollbackSource>);

struct ScrollbackSource {
    cells: RetainedRow,
    /// Stable id (`scrollback-N`), formatted on first read.
    id: OnceLock<String>,
    /// The row's place in the whole output: its `N`.
    line: u64,
    /// Screen width at capture time; the projection pads to it.
    cols: usize,
    /// Link authority in effect when this row scrolled off. A later reset
    /// remints the screen's generation and leaves this one stale, which is what
    /// retires the row's links without rewriting it. `None` once the screen
    /// lost hyperlink authority altogether, which drops the ranges.
    link_generation: Option<NonZeroU64>,
    /// The full projection, shared by every handle until released. Computed
    /// on first read, never on the reader thread.
    projected: Mutex<Option<Arc<TranscriptLine>>>,
    /// The projected row text alone, for whole-transcript readers (find).
    text: OnceLock<String>,
    revision: u64,
}

impl crate::model::DeferredLine for ScrollbackSource {
    fn content_revision(&self) -> u64 {
        self.revision
    }

    /// Scrollback rows are never the cursor row.
    fn expand(&self) -> Arc<TranscriptLine> {
        let mut projected = self
            .projected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(line) = projected.as_ref() {
            return line.clone();
        }
        let line = Arc::new(self.project());
        let _ = self.text.set(line.text.clone());
        *projected = Some(line.clone());
        crate::model::note_deferred_expansion(true);
        line
    }

    fn release(&self) {
        let mut projected = self
            .projected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if projected.take().is_some() {
            crate::model::note_deferred_expansion(false);
        }
    }

    fn text(&self) -> &str {
        self.text.get_or_init(|| self.project().text)
    }

    fn id(&self) -> &str {
        self.id()
    }
}

impl Drop for ScrollbackSource {
    fn drop(&mut self) {
        let held = self
            .projected
            .get_mut()
            .map_or_else(|e| e.into_inner().is_some(), |p| p.is_some());
        if held {
            crate::model::note_deferred_expansion(false);
        }
    }
}

impl ScrollbackSource {
    fn id(&self) -> &str {
        self.id.get_or_init(|| format!("scrollback-{}", self.line))
    }

    fn project(&self) -> TranscriptLine {
        project_row(
            self.cells.views(),
            self.id(),
            false,
            0,
            self.cols,
            self.link_generation,
        )
    }
}

impl ScrollbackRow {
    fn new(
        cells: RetainedRow,
        line: u64,
        cols: usize,
        link_generation: Option<NonZeroU64>,
    ) -> Self {
        Self(Arc::new(ScrollbackSource {
            cells,
            id: OnceLock::new(),
            line,
            cols,
            link_generation,
            projected: Mutex::new(None),
            text: OnceLock::new(),
            revision: TranscriptRow::allocate_revision(),
        }))
    }

    /// Drop this row's hyperlink authority. Handles already given out keep
    /// their old ranges; a fresh source makes later projections link-free.
    fn disable_links(&mut self) {
        let source = &self.0;
        *self = Self::new(source.cells.without_links(), source.line, source.cols, None);
    }

    /// A handle that projects this row when first read.
    fn materialize(&self) -> TranscriptRow {
        TranscriptRow::deferred(self.0.clone())
    }

    /// Eager, un-cached projection, for the equivalence tests.
    #[cfg(test)]
    fn project_eager_reference(&self) -> TranscriptLine {
        self.0.project()
    }
}

fn color_word(color: alacritty_terminal::vte::ansi::Color) -> u64 {
    use alacritty_terminal::vte::ansi::Color;
    match color {
        Color::Named(named) => named as u64,
        Color::Spec(rgb) => {
            1 << 32 | u64::from(rgb.r) << 16 | u64::from(rgb.g) << 8 | u64::from(rgb.b)
        }
        Color::Indexed(i) => 2 << 32 | u64::from(i),
    }
}

/// A fingerprint of a row's cells: what a projection depends on. A cheap
/// multiply-xor mix over each cell's words; it decides only whether to
/// project a row again.
fn fingerprint(cells: &[Cell]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut hash = cells.len() as u64;
    let mut mix = |word: u64| hash = (hash ^ word).wrapping_mul(K).rotate_left(29);
    for cell in cells {
        mix(u64::from(cell.c as u32) | u64::from(cell.flags.bits()) << 32);
        mix(color_word(cell.fg) ^ color_word(cell.bg).rotate_left(17));
        if cell.extra.is_some() {
            for &c in cell.zerowidth().unwrap_or_default() {
                mix(u64::from(c as u32));
            }
            if let Some(link) = cell.hyperlink() {
                for b in link.uri().bytes() {
                    mix(u64::from(b));
                }
            }
        }
    }
    hash
}

/// Only replies that say nothing about the desktop: the primary device
/// attributes (as `ESC [ ? 1 ; 0 c`), the fixed "ready" status report
/// (`ESC [ 0 n`, which programs use to know their output was parsed) and
/// cursor position reports. Everything else the emulator would answer
/// (secondary attributes, mode and keyboard reports) stays unanswered.
fn admitted_reply(reply: &str) -> Option<Vec<u8>> {
    if reply == "\x1b[?6c" {
        return Some(b"\x1b[?1;0c".to_vec());
    }
    if reply == "\x1b[0n" {
        return Some(reply.as_bytes().to_vec());
    }
    let body = reply.strip_prefix("\x1b[")?.strip_suffix('R')?;
    let (row, col) = body.split_once(';')?;
    (row.bytes().all(|b| b.is_ascii_digit()) && col.bytes().all(|b| b.is_ascii_digit()))
        .then(|| reply.as_bytes().to_vec())
}

impl Screen {
    pub(crate) fn new(dirty: Arc<AtomicU64>) -> Self {
        let events = Events::default();
        let config = Config {
            // Rows that scroll off are copied by `handler::Hooked` into this
            // screen's own scrollback; the emulator keeps none.
            scrolling_history: 0,
            kitty_keyboard: false,
            osc52: Osc52::Disabled,
            ..Config::default()
        };
        let term = Term::new(
            config,
            &Size {
                cols: DEFAULT_COLS,
                rows: DEFAULT_ROWS,
            },
            events.clone(),
        );
        Self {
            term,
            events,
            parser: Processor::new(),
            sequence_guard: escape_policy::StringSequenceGuard::default(),
            scanner: scan::Scanner::default(),
            link_generation: LINK_GENERATIONS.allocate(),
            pinned_capture: crate::pinned_output::OutputCapture::default(),
            alt_screen: false,
            live: Vec::new(),
            live_generation: None,
            scrollback: VecDeque::new(),
            max_scrollback: MAX_SCROLLBACK,
            scrollback_counter: 0,
            region: (0, DEFAULT_ROWS),
            scrolled: Vec::new(),
            primary: None,
            dirty,
            pending_effects: TerminalEffects::default(),
            history_disabled: false,
            active_command: None,
            pending_replies: VecDeque::new(),
            live_target: None,
            live_pending: Arc::default(),
        }
    }

    /// Starts or stops writing the visible rows straight into a view's screen.
    /// Starting writes the whole screen at once, so it is never left stale.
    pub(crate) fn set_live(&mut self, target: Option<LiveTarget>) -> Option<LiveWrite> {
        self.live_target = target;
        self.live_pending.store(false, Ordering::Release);
        self.live_write(true)
    }

    /// The rows that changed since the last live write, when one is due
    /// (`force`, or [`LIVE_PERIOD`] since the last); otherwise marks one owed.
    pub(crate) fn live_write(&mut self, force: bool) -> Option<LiveWrite> {
        let live = self.live_target.as_mut()?;
        if !force && live.last.is_some_and(|last| last.elapsed() < LIVE_PERIOD) {
            self.live_pending.store(true, Ordering::Release);
            return None;
        }
        self.live_pending.store(false, Ordering::Release);
        let (cols, rows) = (self.term.columns(), self.term.screen_lines());
        let live = self.live_target.as_mut()?;
        let scheme = crate::color_scheme::active().id;
        if live.printed.len() != rows || live.scheme != scheme {
            live.printed = vec![u64::MAX; rows];
            live.scheme = scheme;
            live.above_printed.clear();
        }
        let _overlays = crate::terminal_view::Overlays {
            focused: live.focused,
            cursor_visible: true,
            selection: None,
            window_start_row: 0,
            matches: &[],
        };
        let _fills = crate::terminal_view::Fills::current();
        // The newest scrollback rows first, oldest at the top, then the
        // emulator's rows below them.
        let above = live.rows_above;
        let count = usize::from(above);
        if live.above_printed.len() != count {
            live.above_printed = vec![u64::MAX; count];
        }
        let first = self.scrollback.len() as isize - count as isize;
        let mut changed = Vec::new();
        for (y, printed) in live.above_printed.iter_mut().enumerate() {
            let row = usize::try_from(first + y as isize)
                .ok()
                .and_then(|index| self.scrollback.get(index));
            let print = above_print(row, cols);
            if *printed == print {
                continue;
            }
            *printed = print;
            let cells = row.map_or_else(Vec::new, |row| row.0.cells.view_row(cols));
            changed.push((y as u16, cells));
        }
        for r in 0..rows {
            let cells = &self.term.grid()[Line(r as i32)][Column(0)..];
            let print = fingerprint(cells);
            if live.printed[r] == print {
                continue;
            }
            live.printed[r] = print;
            changed.push((above + r as u16, project::live_row(cells, cols)));
        }
        let point = self.term.grid().cursor.point;
        let cursor = enzui_terminal::Cursor {
            col: point.column.0.min(cols.saturating_sub(1)) as u16,
            row: above + point.line.0.max(0) as u16,
            shape: crate::terminal_view::cursor_shape(&live.cursor_style),
            visible: live.focused && self.term.mode().contains(TermMode::SHOW_CURSOR),
        };
        let live = self.live_target.as_mut()?;
        live.last = Some(std::time::Instant::now());
        let scheme = crate::color_scheme::active();
        Some(LiveWrite {
            screen: live.screen.clone(),
            grid: (cols as u16, above + rows as u16),
            rows: changed,
            cursor,
            fg: enzui_terminal::Rgba::rgb(
                scheme.default_fg.0,
                scheme.default_fg.1,
                scheme.default_fg.2,
            ),
        })
    }

    pub(crate) fn coordinates_invalidated(&self) -> bool {
        self.pending_effects.coordinates_invalidated
    }

    pub(crate) fn has_terminal_replies(&self) -> bool {
        !self.pending_replies.is_empty()
    }

    pub(crate) fn take_terminal_replies(&mut self) -> VecDeque<Vec<u8>> {
        std::mem::take(&mut self.pending_replies)
    }

    fn queue_reply(&mut self, reply: Vec<u8>) {
        if self.pending_replies.len() < 16 {
            self.pending_replies.push_back(reply);
        }
    }

    fn shell_event(&mut self, event: crate::shell_history::ShellEvent) {
        let events = &mut self.pending_effects.shell_events;
        if events.len() >= crate::model::IN_APP_HISTORY_MAX * 2 {
            events.pop_front();
        }
        events.push_back(event);
    }

    pub(crate) fn disable_shell_integration(&mut self) {
        self.history_disabled = true;
        self.active_command = None;
        self.pinned_capture.disable_history();
        #[cfg(test)]
        {
            self.pending_effects.commands_started.clear();
            self.pending_effects.command_finished = None;
        }
        self.pending_effects.cwd = None;
        self.pending_effects.generated_cwd = None;
        self.pending_effects.shell_events.clear();
        self.shell_event(crate::shell_history::ShellEvent::Unavailable);
    }

    /// Drain OSC/BEL effects produced since the previous call.
    pub(crate) fn take_effects(&mut self) -> TerminalEffects {
        std::mem::take(&mut self.pending_effects)
    }

    /// Sets the retained-row bound; reducing it evicts rows immediately.
    pub(crate) fn set_max_scrollback(&mut self, n: usize) {
        self.max_scrollback = n;
        self.pinned_capture.trim(n);
        if self.trim_history() {
            self.dirty.fetch_add(1, Ordering::SeqCst);
        }
        self.reconcile_pinned_output();
    }

    fn trim_history(&mut self) -> bool {
        let trimmed = self.scrollback.len() > self.max_scrollback;
        while self.scrollback.len() > self.max_scrollback {
            self.scrollback.pop_front();
        }
        self.pending_effects.coordinates_invalidated |= trimmed;
        trimmed
    }

    /// Whether DECCKM (application cursor keys) is engaged: an
    /// alternate-scroll wheel notch then sends `ESC O A`, not `ESC [ A`.
    pub(crate) fn application_cursor_keys(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// Whether the program shows the cursor (DECTCEM).
    pub(crate) fn cursor_shown(&self) -> bool {
        self.term.mode().contains(TermMode::SHOW_CURSOR)
    }

    /// Whether DECSET 2004 (bracketed paste) is engaged.
    pub(crate) fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// The pane's live mouse-tracking `(mode, encoding)` pair. 1001, 1005 and
    /// 1016 are declined as before: only SGR (1006) changes the encoding.
    pub(crate) fn mouse_protocol(&self) -> crate::mouse::MouseProtocol {
        use crate::mouse::{MouseEncoding, MouseTracking};
        let mode = self.term.mode();
        let tracking = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseTracking::AnyEvent
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseTracking::ButtonEvent
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseTracking::Click
        } else {
            MouseTracking::Off
        };
        let encoding = if mode.contains(TermMode::SGR_MOUSE) {
            MouseEncoding::Sgr
        } else {
            MouseEncoding::X10
        };
        crate::mouse::MouseProtocol { tracking, encoding }
    }

    /// Whether the alternate screen is currently engaged. On it there is no
    /// scrollback, so a wheel notch belongs to the application.
    pub(crate) fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// This screen's current link authority, or `0` when it has none.
    pub(crate) fn link_generation(&self) -> u64 {
        self.link_generation.map_or(0, NonZeroU64::get)
    }

    /// Retire every link this screen has already handed out. A refused
    /// replacement disables authority instead of failing.
    fn retire_links(&mut self) {
        if self.link_generation.is_none() {
            return;
        }
        match LINK_GENERATIONS.allocate() {
            Some(generation) => self.link_generation = Some(generation),
            None => self.disable_link_authority(),
        }
    }

    /// Drop this screen's hyperlink authority and every range already
    /// retained, leaving the text a program printed as plain text.
    fn disable_link_authority(&mut self) {
        self.link_generation = None;
        for row in &mut self.scrollback {
            row.disable_links();
        }
    }

    #[cfg(test)]
    pub(crate) fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }

    #[cfg(test)]
    pub(crate) fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    fn dims(&self) -> (usize, usize) {
        (self.term.columns(), self.term.screen_lines())
    }

    fn cursor(&self) -> (usize, usize) {
        let point = self.term.grid().cursor.point;
        (point.column.0, point.line.0.max(0) as usize)
    }

    fn row_cells(&self, row: usize) -> &[Cell] {
        let line = &self.term.grid()[Line(row as i32)];
        &line[Column(0)..]
    }

    /// Parses `bytes` into the emulator, collecting the rows that scroll off.
    fn parse(&mut self, bytes: &[u8]) {
        let mut edited = false;
        let mut hooked = handler::Hooked {
            term: &mut self.term,
            region: &mut self.region,
            scrolled: &mut self.scrolled,
            primary: &mut self.primary,
            edited: &mut edited,
        };
        self.parser.advance(&mut hooked, bytes);
        if let Some(deadline) = self.parser.sync_timeout().sync_timeout()
            && std::time::Instant::now() >= deadline
        {
            self.parser.stop_sync(&mut hooked);
        }
        self.pending_effects.coordinates_invalidated |= edited;
    }

    /// Moves the rows that scrolled off into this screen's scrollback.
    fn take_scrolled_rows(&mut self) {
        if self.scrolled.is_empty() {
            return;
        }
        let cols = self.term.columns();
        for cells in std::mem::take(&mut self.scrolled) {
            self.scrollback_counter = match self.scrollback_counter.checked_add(1) {
                Some(next) => next,
                None => {
                    self.scrollback.clear();
                    self.pending_effects.coordinates_invalidated = true;
                    1
                }
            };
            self.scrollback.push_back(ScrollbackRow::new(
                cells,
                self.scrollback_counter,
                cols,
                self.link_generation,
            ));
        }
        self.trim_history();
    }
}

#[cfg(test)]
mod tests;

mod output;
