//! Terminal PTY retire methods.

use super::*;

impl App {
    /// Retire one pane's PTY by handing its exact owner to the process
    /// executor, and record the ticket that holds every window close.
    ///
    /// This is the only route that removes a live PTY. No signal, wait or join
    /// runs here: the owner moves whole, and the executor performs every pass
    /// on its own thread.
    pub(crate) fn retire_pty(&mut self, session_id: &str, pane_id: &str) {
        let Some(pty) = self.ptys.remove(pane_id) else {
            return;
        };
        self.retire_owned_pty(session_id, pane_id, pty);
    }

    /// [`Self::retire_pty`] for a PTY this app owns but never installed: a
    /// background spawn whose pane was closed or replaced before it finished.
    pub(crate) fn retire_owned_pty(
        &mut self,
        _session_id: &str,
        pane_id: &str,
        pty: pty::PtySession,
    ) {
        let pending = pty::PtyTeardownOutcome::pending();
        let owner = pty.into_debt_owner(pending);
        let token = owner.token();
        // A refusal is not a loss: the owner comes back and stays owned here,
        // visible as debt, until the executor has room for it.
        let retained = self.teardown.submit(owner).err();
        self.teardown_debts.push(TeardownDebt {
            pane: pane_id.to_owned(),
            token,
            outcome: pending,
            settled: false,
            owner: retained,
        });
    }

    /// Collect the executor's typed reports and re-offer any owner it was too
    /// busy to admit.
    ///
    /// Reports are metadata: a report for one token can only ever update that
    /// token's ticket. Nothing on this thread signals, waits or joins.
    pub(crate) fn poll_teardown_debts(&mut self) -> usize {
        for report in self.teardown.collect_reports() {
            if let Some(debt) = self
                .teardown_debts
                .iter_mut()
                .find(|debt| debt.token == report.token)
            {
                debt.outcome = report.outcome;
                debt.settled = report.settled;
            }
        }
        for debt in &mut self.teardown_debts {
            if let Some(owner) = debt.owner.take() {
                debt.owner = self.teardown.submit(owner).err();
            }
        }
        self.teardown_debts.retain(|debt| !debt.settled);
        if let Some(debt) = self.teardown_debts.first() {
            let class = debt
                .outcome
                .class()
                .unwrap_or(pty::TeardownClass::UncertainFreeze);
            blocking_dispatch::diagnostic(
                "teardown-owed",
                class.as_str(),
                usize::try_from(debt.token.get()).unwrap_or(usize::MAX),
            );
        }
        self.teardown_debts.len()
    }
}
