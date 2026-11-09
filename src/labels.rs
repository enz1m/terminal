//! Terminal shared labels and history metrics.

/// The accessible name of the SSH-manager affordance, shared by the sidebar
/// header's `server` icon and (at the compact rung) the titlebar kebab row.
///
/// One constant, two call sites: a compact rung that named the same destination
/// differently would publish two destinations to a screen reader and break the
/// destination-set gate's by-name comparison.
pub(crate) const SIDEBAR_SSH_LABEL: &str = "Remote sessions";

/// Stable coalescing key for the one transient acknowledgement producer.
pub(crate) const HISTORY_COPY_TOAST_KEY: &str = "history-command-copied";

/// Scroll-target id of the history overlay's virtual list.
pub(crate) const HISTORY_LIST_ID: &str = "terminal-history-list";

/// Uniform row pitch of a history row: one `enz_toolbar_icon` beside one
/// caption, which is the same rung the pane action bar uses.
pub(crate) const HISTORY_ROW_PITCH: f32 = 28.0;

/// The tallest the overlay's list may grow, in px.
///
/// This is the number that stops history from eating the terminal: the overlay
/// is a sibling of the pane in the content column, so an unbounded list would
/// take `HISTORY_ROW_PITCH` of height per retained command straight out of the
/// grid's allocation. Six-ish rows is enough to recognise the recent past; the
/// rest is reachable by scrolling inside the list.
pub(crate) const HISTORY_VIEWPORT_MAX: f32 = 6.0 * HISTORY_ROW_PITCH;

/// The list height for `len` retained commands: exactly the rows when they fit,
/// [`HISTORY_VIEWPORT_MAX`] once they do not.
///
/// A short history must not reserve a tall empty strip, and a long one must not
/// grow past the ceiling — the bound is on the *rendered* height, so it holds
/// for 3 commands, for `model::IN_APP_HISTORY_MAX`, and for a `len` the model
/// cannot even reach.
pub(crate) fn history_viewport_height(len: usize) -> f32 {
    (len as f32 * HISTORY_ROW_PITCH).min(HISTORY_VIEWPORT_MAX)
}

/// The titlebar find field's placeholder, which is also its accessible name
/// (`enz_search_field` publishes the placeholder as the field's name; a search
/// field has no separate visible label) and the label of the `session.find`
/// action the band folds from. One constant so the three cannot drift.
pub(crate) const FIND_PLACEHOLDER: &str = "Find in terminal\u{2026}";

/// What the find status line says before a query narrows anything, so the
/// results slot is never a blank strip under the field.
pub(crate) const FIND_EMPTY_STATUS: &str = "Type to search this session";
