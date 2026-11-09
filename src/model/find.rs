//! In-terminal find bar state.

use super::*;
use enzui_widgets::find_surface::FindSurfaceState;

// ---------------------------------------------------------------------------
// Find bar state
// ---------------------------------------------------------------------------

/// State for the in-terminal find bar.
///
/// Presentation remains inline because removing its measured row would change
/// the terminal grid and PTY dimensions. Navigation is nevertheless the shared
/// [`FindSurfaceState`] model: it stores the active match's stable key instead
/// of an index into a freshly-derived vector.
#[derive(Clone, Debug, Default)]
pub(crate) struct FindState {
    /// Whether the inline find bar is visible.
    pub open: bool,
    /// Current search query text.
    pub query: String,
    /// The caller-owned, ordered match collection used for both paint and count.
    pub matches: Vec<FindMatch>,
    pub(crate) navigation: FindSurfaceState<FindMatch>,
    pub(crate) next_key: u64,
    /// The viewport scroll offset that was applied to show the active match.
    pub scroll_to_row: Option<usize>,
    /// Per-row scan results of the previous search, valid for `scan_needle`.
    ///
    /// A query re-runs on every PTY drain, and nearly every row of the new
    /// projection is a row the previous scan already read: retained rows keep
    /// their content revision for life. Reusing their results makes a drain's
    /// search cost the new and changed rows, not the whole scrollback.
    pub(crate) scans: Vec<RowScan>,
    pub(crate) scan_needle: Option<String>,
}

/// One row's scan result under one needle, keyed by its content revision.
#[derive(Clone, Debug)]
pub(crate) struct RowScan {
    revision: u64,
    /// Hash of the row text; computed only for rows with matches, which are
    /// the only rows whose match keys need it.
    fingerprint: u64,
    cols: Box<[usize]>,
}

/// A single find match location carrying an app-stable navigation key.
#[derive(Clone, Debug)]
pub(crate) struct FindMatch {
    key: u64,
    row_id: String,
    line_fingerprint: u64,
    /// Row index in the pane's current transcript projection.
    pub row: usize,
    /// Column (byte offset in `TranscriptLine::text`).
    pub col: usize,
    /// Length of the match in bytes.
    pub len: usize,
}

// FindSurfaceState compares keys. Coordinates deliberately do not participate:
// a streaming projection can move the same row while its selection remains.
impl PartialEq for FindMatch {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for FindMatch {}

fn pop_unused(indices: &mut VecDeque<usize>, used: &[bool]) -> Option<usize> {
    while let Some(index) = indices.pop_front() {
        if !used[index] {
            return Some(index);
        }
    }
    None
}

impl FindState {
    /// Open the inline presentation while entering the shared state machine.
    pub(crate) fn open(&mut self) {
        self.open = true;
        self.navigation
            .apply(&FindSurfaceEvent::Open, &self.matches);
    }

    /// Replace the query and reset the shared active-key selection.
    pub(crate) fn set_query(&mut self, query: String) {
        self.query = query.clone();
        self.navigation
            .apply(&FindSurfaceEvent::QueryChanged(query), &self.matches);
    }

    /// Zero-based position of the shared state's active stable key.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn current(&self) -> Option<usize> {
        self.navigation.active_index(&self.matches)
    }

    /// The active match in the current projection.
    #[must_use]
    pub(crate) fn active_match(&self) -> Option<&FindMatch> {
        self.navigation.active_match(&self.matches)
    }

    fn sync_scroll_to_active(&mut self) {
        self.scroll_to_row = self.active_match().map(|hit| hit.row);
    }

    /// Update matches by searching the transcript. Case-insensitive.
    ///
    /// Existing matches keep their stable keys first by row id, then by exact
    /// line content for the visible-row → scrollback-id transition. Fresh
    /// output receives fresh keys. Reconciliation therefore follows the same
    /// content even when its numeric row index changes.
    pub(crate) fn search(&mut self, transcript: &[TranscriptRow]) {
        self.search_impl(transcript, true);
    }

    fn search_impl(&mut self, transcript: &[TranscriptRow], allow_ascii_fast: bool) {
        if self.navigation.query() != self.query {
            self.navigation.apply(
                &FindSurfaceEvent::QueryChanged(self.query.clone()),
                &self.matches,
            );
        }
        if self.query.is_empty() {
            self.scans = Vec::new();
            self.scan_needle = None;
            self.matches.clear();
            self.navigation.reconcile(&self.matches);
            self.navigation.set_searching(false);
            self.scroll_to_row = None;
            return;
        }

        let old = std::mem::take(&mut self.matches);
        let mut exact: HashMap<(String, u64, usize, usize), VecDeque<usize>> = HashMap::new();
        let mut content: HashMap<(u64, usize, usize), VecDeque<usize>> = HashMap::new();
        for (index, hit) in old.iter().enumerate() {
            exact
                .entry((hit.row_id.clone(), hit.line_fingerprint, hit.col, hit.len))
                .or_default()
                .push_back(index);
            content
                .entry((hit.line_fingerprint, hit.col, hit.len))
                .or_default()
                .push_back(index);
        }
        let mut used = vec![false; old.len()];
        let needle = self.query.to_lowercase();
        // Deterministic scan counters. Accumulate locally so
        // instrumentation does not add one RefCell borrow to every line in the
        // release benchmark; publish once after the real scan.
        let mut metrics = SearchMetrics {
            searches: 1,
            ..SearchMetrics::default()
        };
        // The ASCII fast path.
        //
        // `search` runs on EVERY keystroke (`Message::FindQueryChanged`), over
        // the WHOLE transcript. The line loop used to call
        // `line.text.to_lowercase()` unconditionally, which is one heap
        // allocation and one full copy **per line per keystroke**: typing an
        // eight-character query into a 100 000-line scrollback allocated and
        // freed 800 000 strings whose only purpose was to be searched once.
        // That is the cost item 6 asked about, and it is invisible in a
        // wall-clock number on a fast host, which is why it is counted rather
        // than only timed.
        //
        // The fast path applies when the needle is ASCII (already lowercased
        // above) and the line is ASCII: then case-insensitive matching is
        // `eq_ignore_ascii_case` on byte windows, needs no owned copy, and —
        // this is the part that makes it safe — byte offsets ARE character
        // offsets, so the `col` it reports is identical to the one the
        // lowercased path reported. A non-ASCII line falls back to the
        // original allocating path, because case folding can change a
        // string's LENGTH (`İ` folds to two chars) and a byte offset into
        // the folded copy is then not an offset into the original.
        let needle_is_ascii = needle.is_ascii();
        // Previous results apply only to the same needle on the production
        // path; the legacy reference always scans.
        let mut previous =
            if allow_ascii_fast && self.scan_needle.as_deref() == Some(needle.as_str()) {
                std::mem::take(&mut self.scans)
            } else {
                Vec::new()
            };
        let mut scans = Vec::with_capacity(transcript.len());
        // Rows keep their order between projections (retained rows are only
        // appended and evicted from the front), so a forward merge on the
        // revision finds every reusable row in one pass. A row it misses is
        // simply scanned again.
        let mut cursor = 0usize;
        for (row, line) in transcript.iter().enumerate() {
            let revision = line.content_revision();
            while previous
                .get(cursor)
                .is_some_and(|scan| scan.revision < revision)
            {
                cursor += 1;
            }
            let scan = match previous.get_mut(cursor) {
                Some(scan) if scan.revision == revision => {
                    cursor += 1;
                    metrics.lines_reused += 1;
                    RowScan {
                        revision,
                        fingerprint: scan.fingerprint,
                        cols: std::mem::take(&mut scan.cols),
                    }
                }
                _ => {
                    metrics.lines_scanned += 1;
                    // Text only: a deferred scrollback row answers without
                    // materialising (and retaining) its per-cell projection.
                    let text = line.text();
                    let fast = allow_ascii_fast && needle_is_ascii && text.is_ascii();
                    let haystack: std::borrow::Cow<'_, str> = if fast {
                        std::borrow::Cow::Borrowed(text)
                    } else {
                        metrics.line_allocations += 1;
                        std::borrow::Cow::Owned(text.to_lowercase())
                    };
                    let mut cols = Vec::new();
                    let mut start = 0;
                    while let Some(pos) = find_from(
                        &haystack,
                        &needle,
                        start,
                        fast,
                        &mut metrics.ascii_candidates_checked,
                    ) {
                        let col = start + pos;
                        cols.push(col);
                        start = col + 1;
                    }
                    let fingerprint = if cols.is_empty() {
                        0
                    } else {
                        let mut hasher = DefaultHasher::new();
                        text.hash(&mut hasher);
                        hasher.finish()
                    };
                    RowScan {
                        revision,
                        fingerprint,
                        cols: cols.into_boxed_slice(),
                    }
                }
            };
            let line_fingerprint = scan.fingerprint;
            for &col in &scan.cols {
                let exact_key = (line.id().to_owned(), line_fingerprint, col, needle.len());
                let content_key = (line_fingerprint, col, needle.len());
                let old_index = exact
                    .get_mut(&exact_key)
                    .and_then(|indices| pop_unused(indices, &used))
                    .or_else(|| {
                        content
                            .get_mut(&content_key)
                            .and_then(|indices| pop_unused(indices, &used))
                    });
                let key = if let Some(index) = old_index {
                    used[index] = true;
                    old[index].key
                } else {
                    let key = self.next_key;
                    self.next_key = self.next_key.wrapping_add(1);
                    key
                };
                self.matches.push(FindMatch {
                    key,
                    row_id: line.id().to_owned(),
                    line_fingerprint,
                    row,
                    col,
                    len: needle.len(),
                });
            }
            scans.push(scan);
        }
        if allow_ascii_fast {
            self.scans = scans;
            self.scan_needle = Some(needle);
        }
        SEARCH_METRICS.with(|total| {
            let mut total = total.borrow_mut();
            total.searches += metrics.searches;
            total.lines_scanned += metrics.lines_scanned;
            total.lines_reused += metrics.lines_reused;
            total.line_allocations += metrics.line_allocations;
            total.ascii_candidates_checked += metrics.ascii_candidates_checked;
        });
        self.navigation.reconcile(&self.matches);
        self.navigation.set_searching(false);
        self.sync_scroll_to_active();
    }

    /// Advance to the next stable-key match, wrapping around.
    pub(crate) fn next_match(&mut self) {
        self.navigation
            .apply(&FindSurfaceEvent::Next, &self.matches);
        self.sync_scroll_to_active();
    }

    /// Go to the previous stable-key match, wrapping around.
    pub(crate) fn prev_match(&mut self) {
        self.navigation
            .apply(&FindSurfaceEvent::Previous, &self.matches);
        self.sync_scroll_to_active();
    }

    /// The status string: "M of N" or "No results" or empty.
    pub(crate) fn status(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            return "No results".to_owned();
        }
        self.navigation.count_text(&self.matches)
    }

    /// Close the find bar and clear state.
    pub(crate) fn close(&mut self) {
        self.navigation
            .apply(&FindSurfaceEvent::Dismissed, &self.matches);
        self.navigation.clear();
        self.open = false;
        self.query.clear();
        self.matches.clear();
        self.scroll_to_row = None;
        self.scans = Vec::new();
        self.scan_needle = None;
    }
}

/// Compute the `[start, end)` slice of transcript rows that should be rendered
/// into the fixed-height terminal grid so that `focus_row` is visible.
///
/// The terminal grid renders the slice it is given top-down in a
/// height-`Fill` column; there is no scroll widget, so a match on a
/// scrolled-off row is only brought on-screen by *changing the slice the grid
/// receives*. This function is the consumer-side hinge: `content()` calls it
/// with the real `visible_rows` from `CellGeometry::rows_for` and passes the
/// resulting slice to `terminal_grid`. If it stops honouring `focus_row` the
/// grid silently ignores the find scroll target — which is exactly the
/// inert-control trap the tests here guard against.
///
/// * `total_rows`   — number of rows in the transcript.
/// * `visible_rows` — how many rows fit in the pane (0 or huge are handled).
/// * `focus_row`    — `None` keeps the bottom of the buffer pinned (normal
///   terminal behaviour); `Some(r)` guarantees row `r` is within the window,
///   scrolling the minimum needed to reveal it.
pub(crate) fn visible_window(
    total_rows: usize,
    visible_rows: usize,
    focus_row: Option<usize>,
) -> (usize, usize) {
    if total_rows == 0 {
        return (0, 0);
    }
    // A zero/absurd viewport shows everything rather than nothing.
    if visible_rows == 0 || visible_rows >= total_rows {
        return (0, total_rows);
    }
    let start = match focus_row {
        // No focus: pin to the bottom (last `visible_rows` lines), like a shell.
        None => total_rows - visible_rows,
        Some(row) => {
            let row = row.min(total_rows - 1);
            // Bottom edge of the current bottom-pinned window.
            let bottom_start = total_rows - visible_rows;
            if row >= bottom_start {
                // Already visible in the bottom-pinned window.
                bottom_start
            } else {
                // Center the match, clamped so we never scroll past the ends.
                row.saturating_sub(visible_rows / 2)
                    .min(total_rows - visible_rows)
            }
        }
    };
    (start, (start + visible_rows).min(total_rows))
}

// ---------------------------------------------------------------------------
// Command history truncation
// ---------------------------------------------------------------------------

/// Maximum display length for a command in the history overlay.
#[cfg(test)]
pub(crate) const HISTORY_DISPLAY_MAX_LEN: usize = 40;

/// Deterministic counters for one find-in-scrollback scan.
///
/// # Why counters and not only a stopwatch
///
/// The review asked to *benchmark* search and memory over a very long
/// history. A wall-clock budget sized to survive host variance is far too
/// loose to catch the failure it is named for — a regression that doubles the
/// per-line allocation count disappears into scheduler noise on an idle
/// machine and only shows up on the user's. These counters are exact,
/// reproducible and independent of the host: `lines_scanned` is the size of
/// the problem, `line_allocations` is the per-line heap traffic, and
/// `searches` proves the scan ran at all (a zero-match query that returned
/// early would otherwise look like an infinitely fast search).
///
/// Thread-local because `search` is called from the UI thread only, and a
/// global would make two concurrently running tests read each other's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchMetrics {
    /// Completed `FindState::search` scans over a non-empty query.
    pub searches: u64,
    /// Transcript lines whose text was read across those scans.
    pub lines_scanned: u64,
    /// Lines whose previous result under the same needle was reused instead.
    pub lines_reused: u64,
    /// Owned lowercased copies allocated, one per non-ASCII line visited.
    pub line_allocations: u64,
    /// First-byte candidates whose full byte window was compared on the ASCII
    /// path. Candidate discovery uses `str::find(char)` (the standard
    /// library's memchr-backed single-byte search), rather than checking every
    /// possible window.
    pub ascii_candidates_checked: u64,
}

thread_local! {
    /// The live counters. Read and reset through [`take_search_metrics`].
    pub static SEARCH_METRICS: std::cell::RefCell<SearchMetrics> =
        const { std::cell::RefCell::new(SearchMetrics {
            searches: 0,
            lines_scanned: 0,
            lines_reused: 0,
            line_allocations: 0,
            ascii_candidates_checked: 0,
        }) };
}

/// Find `needle` in `haystack` at or after `start`.
///
/// `fast` selects the allocation-free ASCII path: `haystack` is the ORIGINAL
/// line (any case) and `needle` is already lowercase, so the comparison is
/// `eq_ignore_ascii_case` over byte windows. Otherwise `haystack` is already
/// lowercased and a plain substring search is correct. Both return a byte
/// offset relative to `start`.
fn find_from(
    haystack: &str,
    needle: &str,
    start: usize,
    fast: bool,
    ascii_candidates_checked: &mut u64,
) -> Option<usize> {
    let rest = haystack.get(start..)?;
    if !fast {
        return rest.find(needle);
    }
    let ned = needle.as_bytes();
    let (&first, _) = ned.split_first()?;
    if rest.len() < ned.len() {
        return None;
    }

    // `str::find` for one ASCII `char` is the standard library's
    // memchr-backed byte search. Probe the lower/upper spelling of the first
    // byte and compare only those candidate windows; the previous
    // `(0..=len-n).find(..)` compared at every byte and regressed precisely the
    // fast substring search this optimization was meant to preserve.
    let lower = first.to_ascii_lowercase() as char;
    let upper = first.to_ascii_uppercase() as char;
    let mut cursor = 0usize;
    while cursor + ned.len() <= rest.len() {
        let tail = &rest[cursor..];
        let lo = tail.find(lower);
        let up = if upper == lower {
            None
        } else {
            tail.find(upper)
        };
        let candidate = match (lo, up) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => return None,
        };
        let at = cursor + candidate;
        if at + ned.len() > rest.len() {
            return None;
        }
        *ascii_candidates_checked += 1;
        if rest.as_bytes()[at..at + ned.len()].eq_ignore_ascii_case(ned) {
            return Some(at);
        }
        cursor = at + 1;
    }
    None
}
