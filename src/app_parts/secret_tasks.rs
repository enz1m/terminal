//! Terminal secret-task methods.

use super::*;

impl App {
    pub(crate) fn remember_secret_task(
        &mut self,
        owner: String,
        slot: CredentialSlot,
        secret: String,
    ) -> Task<Message> {
        let Some((_, fut)) = self.remember_secret_op(owner, slot, secret) else {
            return Task::none();
        };
        self.spawn_credential(fut)
    }

    pub(crate) fn forget_secret_task(
        &mut self,
        owner: String,
        slot: CredentialSlot,
    ) -> Task<Message> {
        let Some((_, fut)) = self.forget_secret_op(owner, slot) else {
            return Task::none();
        };
        self.spawn_credential(fut)
    }

    /// Drive `task` and every task its messages return to quiescence. A saved
    /// connection now settles on the blocking owner (`SshHostSaveFinished`)
    /// before the host exists, so a fixture that drops the task leaves the
    /// save pending and refuses the next one. Parked credential futures are
    /// left alone (see [`Self::drive_credentials`]): a gate may hold them.
    #[cfg(test)]
    pub(crate) fn settle_for_test(&mut self, task: Task<Message>) {
        use std::time::{Duration, Instant};
        // A stream that stays pending this long is a subscription-shaped or
        // held task, not settling work: it is dropped, not waited on.
        const QUIET: Duration = Duration::from_millis(500);
        let mut pending = vec![task];
        for _ in 0..16 {
            let mut next = Vec::new();
            for task in pending {
                let Some(mut stream) = iced_test::runtime::task::into_stream(task) else {
                    continue;
                };
                while let Some(action) = next_before(&mut stream, Instant::now() + QUIET) {
                    if let iced_test::runtime::Action::Output(message) = action {
                        next.push(self.update(message));
                    }
                }
            }
            if next.is_empty() {
                return;
            }
            pending = next;
        }
        panic!("the task kept producing work past its settle budget");
    }

    /// Give this app its own teardown executor, so a gate that waits for its
    /// PTYs' teardown does not queue behind every parallel test's shells on
    /// the process-wide one (each may take the 3 s TERM grace).
    #[cfg(test)]
    pub(crate) fn use_private_teardown_for_test(&mut self) {
        self.teardown = Box::leak(Box::new(pty::TeardownSupervisor::for_test(64, 0)));
    }

    /// Run every parked credential future to completion, feeding each
    /// completion message back through `update`, until none remain.
    #[cfg(test)]
    pub(crate) fn drive_credentials(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        while !self.credential_futures.is_empty() {
            for fut in std::mem::take(&mut self.credential_futures) {
                let message = iced_test::futures::futures::executor::block_on(fut);
                tasks.push(self.update(message));
            }
        }
        Task::batch(tasks)
    }
}

/// The stream's next item, or `None` when it ends or stays pending until
/// `deadline`.
#[cfg(test)]
fn next_before<S>(stream: &mut S, deadline: std::time::Instant) -> Option<S::Item>
where
    S: iced::futures::Stream + Unpin,
{
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread;

    struct Unpark(thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match std::pin::Pin::new(&mut *stream).poll_next(&mut context) {
            Poll::Ready(item) => return item,
            Poll::Pending => {
                let now = std::time::Instant::now();
                if now >= deadline {
                    return None;
                }
                thread::park_timeout(deadline - now);
            }
        }
    }
}
