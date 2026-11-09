//! Terminal transcript-snapshot method.

use super::*;

impl App {
    /// Install a PTY projection and refresh live find results when (and only
    /// when) that projection belongs to the active pane. `None` is the quiet
    /// control from `PtySession::take_snapshot`: it performs no assignment and
    /// does not reset the reader's current match.
    pub(crate) fn apply_transcript_snapshot(
        sessions: &mut [Session],
        find: &mut FindState,
        active_session: usize,
        session_index: usize,
        pane_index: usize,
        snapshot: Option<Vec<TranscriptRow>>,
    ) {
        let Some(lines) = snapshot else {
            return;
        };
        let active_pane = sessions
            .get(active_session)
            .map(|session| session.active_pane);
        let refresh_find = session_index == active_session && active_pane == Some(pane_index);

        let Some(pane) = sessions
            .get_mut(session_index)
            .and_then(|session| session.panes.get_mut(pane_index))
        else {
            return;
        };
        pane.transcript = lines;
        if refresh_find {
            find.search(&pane.transcript);
        }
    }
}
