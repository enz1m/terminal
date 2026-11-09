//! Terminal grid-pointer gesture probe.

/// The terminal grid's last reported pointer coordinates.
///
/// A press records the `anchor`, drags and the release move `focus`, and
/// `holding` says whether a gesture is still live. Rows are **transcript** rows,
/// not viewport rows: the grid reports its position within the rendered window
/// and `App` adds the window's first row, so a coordinate keeps its meaning
/// after the transcript scrolls.
///
/// The selection model (`App::selections`) is built from these coordinates
/// Painting, normalisation and clipboard copy live there, not here.
///
/// Not `Copy`: the probe names the pane owning the live gesture by stable id,
/// which survives a sibling closing mid-drag where an index would retarget the
/// gesture at whoever took that slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GridPointerProbe {
    /// Where the gesture started, in `(transcript row, column)`.
    pub(crate) anchor: Option<(i64, usize)>,
    /// Where the pointer is now (or ended), in `(transcript row, column)`.
    pub(crate) focus: Option<(i64, usize)>,
    /// Original hit of a word/line click, retained until the pointer moves.
    pub(crate) expanded_at: Option<(i64, usize)>,
    /// Whether the pointer is still down.
    pub(crate) holding: bool,
    /// `true` when the last report was past the end of its row — the
    /// distinction a selection needs to decide whether it swallows the rest of
    /// the line.
    pub(crate) focus_past_line_end: bool,
    /// The pane the live gesture belongs to, by stable id.
    ///
    /// A drag or release report from any other pane is ignored, so a gesture
    /// that started in one leaf cannot rewrite a sibling's range when the
    /// pointer crosses the seam.
    pub(crate) pane: Option<String>,
}
