/// `monochrome` splits on **grapheme clusters**, not `char`s, and its
/// cell vector stays **column-indexed**.
///
/// # The defect this gates
///
/// This was `text.chars().map(..)`, one cell per `char`. A US flag is one
/// cluster of two `char`s, so it became two cells, and a lone regional
/// indicator paints as its cyan fallback glyph — the reported "paints as the
/// letters US". Ten call sites build rows this way (the shell-exited banner,
/// the failed-pane reason, the demo transcript), so this is a second,
/// independent instance of the defect on a path the terminal parser never
/// touches.
///
/// # The two claims, which are different
///
/// 1. **The cluster is whole**: the cell at the cluster's start column holds
///    all of its bytes, so the grid can paint a flag.
/// 2. **The indexing is still by COLUMN**: a double-width cluster is followed
///    by a blank continuation cell, because `grid_view::row_cells` matches
///    `cursor_col` against the cell's `.enumerate()` index. Emitting one cell
///    per cluster would paint correctly while putting the block cursor left
///    of where it belongs — the drift this issue warns about.
///
/// # Sabotage
///
/// Restore `text.chars()`: `\u{1F1FA}\u{1F1F8}` yields two cells each holding
/// half the flag, and claim 1 fails. Independently, drop the continuation
/// push: the CJK row's `cursor_col` drops from 4 to 3 and claim 2 fails.
/// One `monochrome` case: label, input text, the columns it occupies, and
/// where each cluster must land. Named so the table below does not trip
/// clippy's `type_complexity` (and reads better than a 4-tuple in a slice).
type MonoCase<'a> = (&'a str, &'a str, usize, &'a [(&'a str, usize)]);
