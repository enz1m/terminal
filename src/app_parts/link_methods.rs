//! Terminal link-open methods.

use super::*;

impl App {
    #[cfg(test)]
    pub(crate) fn finish_link_open(&mut self, result: Result<(), link_open::LinkOpenError>) {
        self.pane_feedback = result.err().map(|error| {
            eprintln!(
                "terminal link-open failure: kind={} diagnostic=\"{}\"",
                error.kind().as_str(),
                error.diagnostic()
            );
            error.user_summary().to_owned()
        });
    }

    pub(crate) fn link_target_at(&self, cell: (i64, usize)) -> Option<link_open::OpenTarget> {
        let session = self.sessions.get(self.active)?;
        let pane = session.panes.get(session.active_pane)?;
        let row = usize::try_from(cell.0).ok()?;
        let line = pane.transcript.get(row)?;
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        // The reported location is only a filesystem base when it is a local
        // absolute path; a host-qualified or malformed label stays display
        // metadata and resolves nothing, so no token resolves either.
        let base = shell_history::cwd::local_path(&pane.cwd)
            .or_else(|| escape_policy::resolvable_local_base(&pane.cwd));
        let allow_local_path = pane.endpoint.kind() == SessionKind::Local && base.is_some();
        link_open::target_at(
            line.as_ref(),
            cell.1,
            &base.unwrap_or_default(),
            home.as_deref(),
            allow_local_path,
            self.live_link_generation(&pane.id),
        )
    }
}
