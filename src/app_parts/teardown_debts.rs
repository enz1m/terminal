//! Terminal teardown-debt methods.

use super::*;

impl App {
    /// Retire every live PTY through the exact owner route.
    ///
    /// Called wherever the pane metadata that addresses a child is about to go
    /// away: workspace adoption, and the committed close. The orphan pass is
    /// what makes this total — a PTY keyed by a pane no longer in any session
    /// still owns a child, and leaving it to a later field drop would abandon
    /// one. Constant work per session: nothing here signals, waits or joins.
    pub(crate) fn retire_every_live_pty(&mut self) {
        let live: Vec<(String, String)> = self
            .sessions
            .iter()
            .flat_map(|session| {
                session
                    .panes
                    .iter()
                    .map(|pane| (session.id.clone(), pane.id.clone()))
            })
            .filter(|(_, pane)| self.ptys.contains_key(pane))
            .collect();
        for (session_id, pane_id) in &live {
            self.retire_pty(session_id, pane_id);
        }
        let orphans: Vec<String> = self.ptys.keys().cloned().collect();
        for pane_id in &orphans {
            self.retire_pty("", pane_id);
        }
        debug_assert!(
            self.ptys.is_empty(),
            "retirement must be total: every live PTY leaves through its owner"
        );
    }

    /// Whether this pane still owns an unresolved teardown, which must not be
    /// replaced by a fresh child on the same pane.
    pub(crate) fn pane_has_teardown_debt(&self, pane_id: &str) -> bool {
        self.teardown_debts.iter().any(|debt| debt.pane == pane_id)
    }

    /// Give up the app-side tickets for a close whose surface is already gone.
    ///
    /// Every owner this app still retained moves to the executor, which cannot
    /// refuse it: this route has nowhere else to put one. Nothing is reduced to
    /// a counter, and the process may not exit until the executor itself
    /// reports no owner left.
    pub(crate) fn transfer_teardown_debts_to_process(&mut self) {
        for mut debt in self.teardown_debts.drain(..) {
            if let Some(owner) = debt.owner.take() {
                self.teardown.adopt_final(owner);
            }
        }
    }
}

/// A test app hands every PTY and retained teardown it still owns to its own
/// executor when it goes away, instead of letting each session's `Drop` queue
/// on the process executor behind every other parallel test.
#[cfg(test)]
impl Drop for App {
    fn drop(&mut self) {
        self.transfer_teardown_debts_to_process();
        for (_, pty) in self.ptys.drain() {
            self.teardown
                .adopt_final(pty.into_debt_owner(pty::PtyTeardownOutcome::pending()));
        }
    }
}
