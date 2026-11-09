//! Terminal PTY write methods.

use super::*;

impl App {
    /// Write a whole chunk (a paste, a recalled command) to the active pane.
    pub(crate) fn write_active_pty(&mut self, bytes: &[u8]) {
        self.write_active_pty_as(bytes, pty::InputAdmission::Whole);
    }

    /// Write keys or pointer reports to the active pane; they may join the
    /// newest unstarted typed chunk.
    pub(crate) fn type_active_pty(&mut self, bytes: &[u8]) {
        self.write_active_pty_as(bytes, pty::InputAdmission::Typed);
    }

    fn write_active_pty_as(&mut self, bytes: &[u8], admission: pty::InputAdmission) {
        let Some(pid) = self.active_pane_id() else {
            return;
        };
        let Some(session_id) = self
            .sessions
            .get(self.active)
            .map(|session| session.id.clone())
        else {
            return;
        };
        self.write_pane_pty_as(&session_id, &pid, bytes, admission);
    }

    /// Write to the pane identified by a detached window's stable coordinates.
    /// The caller has already validated `(window, session, pane)`; resolving by
    /// ids again here is intentional defence against reorder and close races.
    pub(crate) fn write_pane_pty(&mut self, session_id: &str, pane_id: &str, bytes: &[u8]) {
        self.write_pane_pty_as(session_id, pane_id, bytes, pty::InputAdmission::Whole);
    }

    /// [`Self::write_pane_pty`] for keys, IME commits and pointer reports.
    pub(crate) fn type_pane_pty(&mut self, session_id: &str, pane_id: &str, bytes: &[u8]) {
        self.write_pane_pty_as(session_id, pane_id, bytes, pty::InputAdmission::Typed);
    }

    pub(crate) fn try_write_pane_pty(
        &mut self,
        session_id: &str,
        pane_id: &str,
        bytes: &[u8],
    ) -> bool {
        self.write_pane_pty_as(session_id, pane_id, bytes, pty::InputAdmission::Whole)
    }

    fn write_pane_pty_as(
        &mut self,
        session_id: &str,
        pane_id: &str,
        bytes: &[u8],
        admission: pty::InputAdmission,
    ) -> bool {
        #[cfg(test)]
        {
            self.pty_write_log.push(bytes.to_vec());
            self.targeted_pty_write_log
                .push((pane_id.to_owned(), bytes.to_vec()));
        }
        let addressed = self.sessions.iter().any(|session| {
            session.id == session_id && session.panes.iter().any(|pane| pane.id == pane_id)
        });
        let result =
            addressed
                .then(|| self.ptys.get(pane_id))
                .flatten()
                .map(|pty| match admission {
                    pty::InputAdmission::Whole => pty.write(bytes),
                    pty::InputAdmission::Typed => pty.write_typed(bytes),
                });
        match result {
            Some(Ok(())) => true,
            Some(Err(error)) => {
                if let Some(refusal) = error.input_refusal() {
                    self.pane_feedback = Some(refusal.user_summary().to_owned());
                }
                false
            }
            None => false,
        }
    }
}
