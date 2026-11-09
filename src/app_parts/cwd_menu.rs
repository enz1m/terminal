//! Terminal cwd and menu methods.

use super::*;

impl App {
    pub(crate) fn active_local_cwd(&self) -> Option<String> {
        let session = self.sessions.get(self.active)?;
        session
            .panes
            .get(session.active_pane)
            .filter(|pane| pane.endpoint == pane_endpoint::PaneEndpoint::Local)
            .map(|pane| pane.cwd.clone())
            .filter(|cwd| !cwd.is_empty())
    }

    // ----- update ----------------------------------------------------------

    /// Re-derive an open context menu's rows from live state.
    ///
    /// Rebuilding the whole menu surface from `App` every frame lets a row's
    /// enabled flag track the world: moving a tab to the top disables `Move up`
    /// under an already-open menu. Owning a `MenuModel` in state, which the
    /// framework host requires because it borrows the model for the element's
    /// lifetime and the model carries the roving highlight, would otherwise
    /// freeze those flags at open time.
    ///
    /// `MenuModel::reconcile` is the framework's answer, and it is why the
    /// highlight is held **by id**: it swaps the entries and then re-lands the
    /// active row on the same key, falling to the nearest focusable row only if
    /// that key is gone or went disabled. Rows stay live AND the cursor does
    /// not jump.
    ///
    /// Run as a **post-pass**, after the arm rather than before it. A pre-pass
    /// reconciles against the state the message is about to change, so the
    /// frame drawn straight after a `MoveSessionUp` would still show the old
    /// enabled flags — one frame stale, silently fixed by the next message,
    /// which is the worst kind of bug to meet in a screenshot.
    ///
    /// This also covers the LABEL flips, which is what makes it visible here in
    /// a way it was not in files-app: `Pin tab`/`Unpin tab` and
    /// `Detach tab`/`Re-dock tab` swap text on the very state their own rows
    /// change, so without this pass a user could click `Pin tab` and watch the
    /// row keep saying `Pin tab`.
    pub(crate) fn reconcile_open_context_menu(&mut self) {
        let Some(rows) = context_menu::model_for_open(self) else {
            return;
        };
        if let Some(open) = self.context_menu.as_mut() {
            let _ = open.model.reconcile(rows.entries().to_vec());
        }
    }
}
