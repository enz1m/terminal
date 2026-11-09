//! Terminal reducer batch 24: close-settle and teardown arms.

use super::*;

impl App {
    pub(crate) fn reduce24_close_settle_0(&mut self) -> Task<Message> {
        // The budget is spent here; the post-reducer funnel is what
        // commits a close whose debt has really settled.
        self.expire_close_budget()
    }
    pub(crate) fn reduce24_teardown_poll_1(&mut self) -> Task<Message> {
        self.poll_teardown_debts();
        if self.exit_withheld {
            return self.exit_when_settled();
        }

        Task::none()
    }
}
