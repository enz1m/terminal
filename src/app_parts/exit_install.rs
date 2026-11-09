//! Terminal exit authority and PTY install methods.

use super::*;

impl App {
    /// Whether the process may end, read from the executor rather than from any
    /// copied outcome.
    pub(crate) fn exit_authority(&self) -> pty::ExitAuthority {
        self.teardown.poll_exit_authority()
    }

    pub(crate) fn install_pty(&mut self, id: String, pty: pty::PtySession) {
        pty.set_max_scrollback(self.settings.clamped_scrollback() as usize);
        self.restored_outputs.remove(&id);
        self.finished_outputs.remove(&id);
        self.restored_grid_sizes.remove(&id);
        self.ptys.insert(id, pty);
    }
}
