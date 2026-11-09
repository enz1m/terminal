//! PtySession termination and custody.

use super::*;

impl PtySession {
    /// Terminate and reap this PTY's process tree, close its endpoints, and
    /// join both worker threads. Calling this more than once is a no-op. Linux
    /// teardown freezes the leader first, then captures every same-session,
    /// descendant, or exact slave-fd holder as a verified pidfd. This includes
    /// interactive jobs in other process groups and descendants which called
    /// `setsid()`.
    pub(crate) fn shutdown(&mut self) -> PtyTeardownOutcome {
        #[cfg(test)]
        assert!(
            !std::mem::take(&mut self.force_sweep_panic),
            "injected teardown pass failure"
        );
        if let Some(outcome) = self.teardown_outcome {
            return outcome;
        }
        let writer_running = self
            .input
            .as_ref()
            .is_some_and(|input| input.thread.is_some());
        if !self.terminate_started {
            self.terminate_started = true;
            if self.reader_thread.is_none() && self.exit_thread.is_none() && !writer_running {
                #[cfg(unix)]
                self.shell_startup.take();
                // With no workers, no owned child can still open either handoff.
                #[cfg(target_os = "linux")]
                {
                    let _ = self._password_secret.take();
                    self._capability_handoff.take();
                }
                let outcome = PtyTeardownOutcome::settled();
                self.teardown_outcome = Some(outcome);
                return outcome;
            }
            self.begin_termination();
        }
        self.settle_pass()
    }

    /// The one-time graceful phase: cancel the reader, freeze the tree, TERM it
    /// and wait a bounded interval before any uncatchable signal.
    pub(crate) fn begin_termination(&mut self) {
        // Close the queue before any signal so no further input can be
        // admitted, and mark it closing so queued input is discarded rather
        // than delivered; the writer thread then ends once its current chunk
        // is done. Releasing our slave copy is what unblocks a chunk already
        // stuck in the kernel: a master write only errors out once the last
        // slave holder is gone, and a child that stopped reading never drains
        // the queue on its own. Without this, one stalled write would be
        // permanent teardown debt rather than a bounded join.
        if let Some(input) = self.input.as_mut() {
            input.closing.store(true, Ordering::Release);
            input.sender.take();
        }
        self.slave.take();

        #[cfg(target_os = "linux")]
        if let Some(cancel) = self.reader_cancel.as_mut() {
            // This is independent of slave EOF: a non-owned holder may keep
            // the exact slave open and must neither be killed nor strand join.
            let _ = cancel.write_all(b"x");
        }

        #[cfg(target_os = "linux")]
        {
            // Freeze the root before discovering descendants so it cannot fork
            // behind the scan. Repeated stop-and-rescan catches children that
            // were already crossing fork/setsid when shutdown began.
            let _ = self.process_identity.freeze_and_capture_until_stable(
                &mut self.owned,
                Instant::now() + SHUTDOWN_TERM_GRACE,
            );
            let _ = self.process_identity.signal_owned_processes(
                &mut self.owned,
                rustix::process::Signal::TERM,
                true,
            );
            for member in self
                .owned
                .iter()
                .filter(|member| member.presence().is_owed())
            {
                member.signal(rustix::process::Signal::CONT);
            }
        }
        #[cfg(not(target_os = "linux"))]
        if !self.exit_done.load(Ordering::Acquire)
            && let Some(killer) = self.child_killer.as_mut()
        {
            let _ = killer.kill();
        }

        // On Linux retain the master (and thus its stable master identity)
        // until the cancellation-aware reader has joined; cancellation, not
        // slave EOF, releases that worker. The writer handle itself belongs to
        // the writer thread and is released when that thread ends.
        #[cfg(not(target_os = "linux"))]
        self.master.take();

        let term_deadline = Instant::now() + SHUTDOWN_TERM_GRACE;
        while Instant::now() < term_deadline {
            #[cfg(target_os = "linux")]
            {
                let _ = self.process_identity.signal_owned_processes(
                    &mut self.owned,
                    rustix::process::Signal::TERM,
                    true,
                );
                if self.process_identity.owed_processes(&self.owned) == 0 {
                    break;
                }
            }
            #[cfg(not(target_os = "linux"))]
            if self.exit_done.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        #[cfg(target_os = "linux")]
        {
            // Freeze/rescan to stability once more before KILL. TERM handlers
            // may have forked after the first freeze; two empty scans after all
            // discovered members receive STOP close that final race.
            let final_sweep = self.process_identity.freeze_and_capture_until_stable(
                &mut self.owned,
                Instant::now() + SHUTDOWN_TERM_GRACE,
            );
            self.freeze_stable = final_sweep.reached_stability;
            self.freeze_scans = final_sweep.scans;
            #[cfg(test)]
            {
                self.final_freeze_scans
                    .store(final_sweep.scans, Ordering::Release);
                self.final_freeze_stable
                    .store(final_sweep.reached_stability, Ordering::Release);
            }
        }
    }

    /// Finish the leader reap on this thread once no waiter can still call
    /// `wait`, using the retained pidfd.
    ///
    /// Only a wait result is proof: `Ok(Some(_))` is this thread's own reap and
    /// `ECHILD` proves somebody already completed one. `Ok(None)` means the
    /// child is still there, and any other error proves nothing — both keep
    /// leader debt. A disappeared `/proc` entry is never consulted here.
    #[cfg(target_os = "linux")]
    pub(crate) fn reap_leader_without_waiter(&self) {
        if self.exit_thread.is_some() || self.leader_reaped.load(Ordering::Acquire) {
            return;
        }
        let reaped = match rustix::process::waitid(
            rustix::process::WaitId::PidFd(self.process_identity.pidfd.as_ref().as_fd()),
            rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG,
        ) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            // Already reaped by somebody: still an observed wait result.
            Err(err) => err == rustix::io::Errno::CHILD,
        };
        if reaped {
            self.leader_reaped.store(true, Ordering::Release);
        }
    }

    /// One bounded exact signal, reap and join pass over this session's own
    /// retained resources. Safe to repeat: every step re-reads live state.
    pub(crate) fn settle_pass(&mut self) -> PtyTeardownOutcome {
        #[cfg(target_os = "linux")]
        let kill_grace = if self.settled_once {
            RETRY_KILL_GRACE
        } else {
            SHUTDOWN_KILL_GRACE
        };
        let join_grace = if self.settled_once {
            RETRY_JOIN_GRACE
        } else {
            SHUTDOWN_JOIN_GRACE
        };
        self.settled_once = true;

        #[cfg(target_os = "linux")]
        let (residual_descendants, leader_signalled, timed_out) = {
            let descendant_deadline = Instant::now() + kill_grace;
            while self.process_identity.owed_descendants(&self.owned) != 0
                && Instant::now() < descendant_deadline
            {
                let _ = self.process_identity.signal_owned_processes(
                    &mut self.owned,
                    rustix::process::Signal::KILL,
                    false,
                );
                thread::sleep(Duration::from_millis(10));
            }
            // An undeliverable leader signal is uncertainty of its own, so it is
            // recorded rather than discarded. Only a leader provably gone or
            // replaced needs no signal; an unreadable one keeps the debt.
            let leader_signalled = self
                .process_identity
                .signal_leader(rustix::process::Signal::KILL)
                .is_ok()
                || self.process_identity.leader_departed();
            let residual_descendants = self.process_identity.owed_descendants(&self.owned);
            let timed_out = residual_descendants != 0;

            let (requested, ready) = &*self.reap_request;
            let mut requested = requested
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *requested = true;
            ready.notify_one();
            (residual_descendants, leader_signalled, timed_out)
        };
        #[cfg(not(target_os = "linux"))]
        let (residual_descendants, leader_signalled, timed_out) = {
            self.child_killer.take();
            (0, true, false)
        };

        // Keep handles through both joins: pidfd termination releases the waiter,
        // and private cancellation releases the reader independently of slave EOF.
        // Each join is bounded on its own budget, so one stuck worker cannot
        // consume the other's.
        let mut parked_workers = 0;
        if let Some(input) = self.input.as_mut() {
            let joined = Arc::clone(&input.joined);
            match join_worker_bounded(&mut input.thread, &joined, Instant::now() + join_grace) {
                WorkerSettlement::Joined => {}
                WorkerSettlement::Panicked => input.panicked = true,
                WorkerSettlement::Running(handle) => {
                    input.thread = Some(handle);
                    parked_workers += 1;
                }
            }
        }
        let writer_panicked = self.input.as_ref().is_some_and(|input| input.panicked);
        let writer_joined = self
            .input
            .as_ref()
            .is_none_or(|input| input.thread.is_none());

        let waiter = join_worker_bounded(
            &mut self.exit_thread,
            &self.exit_joined,
            Instant::now() + join_grace,
        );
        let reader = join_worker_bounded(
            &mut self.reader_thread,
            &self.reader_joined,
            Instant::now() + join_grace,
        );
        match waiter {
            WorkerSettlement::Joined => {}
            WorkerSettlement::Panicked => self.waiter_panicked = true,
            WorkerSettlement::Running(handle) => {
                self.exit_thread = Some(handle);
                parked_workers += 1;
            }
        }
        match reader {
            WorkerSettlement::Joined => {}
            WorkerSettlement::Panicked => self.reader_panicked = true,
            WorkerSettlement::Running(handle) => {
                self.reader_thread = Some(handle);
                parked_workers += 1;
            }
        }
        #[cfg(target_os = "linux")]
        self.reap_leader_without_waiter();
        let waiter_joined = self.exit_thread.is_none();
        let reader_joined = self.reader_thread.is_none();
        // Proof of reap, never inference from it: only a wait result this
        // process observed sets this, so a leader whose `/proc` entry merely
        // vanished still counts as debt.
        let leader_reaped = self.leader_reaped.load(Ordering::Acquire);
        #[cfg(test)]
        let leader_reaped = leader_reaped && !self.force_incomplete;

        if parked_workers == 0 {
            #[cfg(unix)]
            self.shell_startup.take();
        }

        // Lazy /proc opens must not see a recycled fd number, including after
        // handshake timeout. A still-running worker may read the master or
        // perform that open, so this session keeps every descriptor until both
        // workers are gone. Child-held copies and already-drained password
        // bytes are outside this release boundary.
        let descriptors_released = if parked_workers == 0 {
            self.master.take();
            self.slave.take();
            #[cfg(target_os = "linux")]
            {
                self.reader_cancel.take();
                let _ = self._password_secret.take();
                self._capability_handoff.take();
            }
            true
        } else {
            false
        };

        let outcome = PtyTeardownOutcome {
            attempted: true,
            freeze_stable: self.freeze_stable,
            residual_descendants,
            leader_signalled,
            leader_reaped,
            waiter_joined,
            reader_joined,
            writer_joined,
            descriptors_released,
            timed_out,
            parked_workers,
            reader_panicked: self.reader_panicked,
            waiter_panicked: self.waiter_panicked,
            writer_panicked,
        };
        // A panic is terminal rather than parked, so it is reported even when
        // the pass otherwise completed.
        if !outcome.is_complete() || outcome.reader_panicked || outcome.waiter_panicked {
            report_teardown(&outcome, self.freeze_scans);
        }
        // Cache only a complete teardown: an incomplete pass must stay
        // retryable through the owner that retains this session.
        if outcome.is_complete() {
            self.teardown_outcome = Some(outcome);
        }
        outcome
    }

    /// This session's reserved teardown identity, allocated before its child.
    #[cfg(test)]
    pub(crate) fn teardown_token(&self) -> TeardownToken {
        self.teardown_token
    }

    /// Unwind the next teardown pass exactly once.
    #[cfg(test)]
    pub(crate) fn force_sweep_panic_once(&mut self) {
        self.force_sweep_panic = true;
    }

    /// Withhold leader settlement from the next pass.
    #[cfg(test)]
    pub(crate) fn force_incomplete_teardown(&mut self) {
        self.force_incomplete = true;
        self.teardown_outcome = None;
    }

    /// Hand this session over as teardown debt, keeping every retained
    /// descriptor, pidfd, worker handle and the reservation.
    pub(crate) fn into_debt_owner(self, outcome: PtyTeardownOutcome) -> PtyTeardownDebtOwner {
        PtyTeardownDebtOwner::new(self, outcome)
    }

    /// Move every piece of teardown custody into a new session value, leaving
    /// this one inert.
    ///
    /// The leader pidfd and each descendant pidfd, both worker handles, the pty
    /// pair, the cancellation pipe, both `/proc` handoffs and the capacity
    /// reservation all move. Nothing is closed and no slot is returned, which
    /// is what makes this safe to call from a `Drop` that cannot run a pass.
    pub(crate) fn transfer_custody(&mut self) -> Self {
        self.custody = CustodyState::Moved;
        Self {
            session_id: std::mem::take(&mut self.session_id),
            live: Mutex::new(
                self.live
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take(),
            ),
            input: self.input.take(),
            #[cfg(unix)]
            shell_startup: self.shell_startup.take(),
            screen: Arc::clone(&self.screen),
            dirty: Arc::clone(&self.dirty),
            last_seen: AtomicU64::new(self.last_seen.load(Ordering::Acquire)),
            #[cfg(test)]
            recorded_input: Mutex::new(Vec::new()),
            master: self.master.take(),
            slave: self.slave.take(),
            child_exit: Arc::clone(&self.child_exit),
            secret_delivery: self.secret_delivery,
            #[cfg(target_os = "linux")]
            process_identity: self.process_identity.transfer(),
            #[cfg(target_os = "linux")]
            reap_request: Arc::clone(&self.reap_request),
            #[cfg(target_os = "linux")]
            reader_cancel: self.reader_cancel.take(),
            #[cfg(not(target_os = "linux"))]
            child_killer: self.child_killer.take(),
            reader_thread: self.reader_thread.take(),
            exit_thread: self.exit_thread.take(),
            #[cfg(test)]
            reader_done: Arc::clone(&self.reader_done),
            #[cfg(any(test, not(target_os = "linux")))]
            exit_done: Arc::clone(&self.exit_done),
            reader_joined: Arc::clone(&self.reader_joined),
            exit_joined: Arc::clone(&self.exit_joined),
            leader_reaped: Arc::clone(&self.leader_reaped),
            teardown_token: self.teardown_token,
            _reaper_slot: self._reaper_slot.transfer(),
            teardown_outcome: self.teardown_outcome,
            #[cfg(target_os = "linux")]
            owned: std::mem::take(&mut self.owned),
            custody: CustodyState::Held,
            terminate_started: self.terminate_started,
            settled_once: self.settled_once,
            freeze_stable: self.freeze_stable,
            freeze_scans: self.freeze_scans,
            reader_panicked: self.reader_panicked,
            waiter_panicked: self.waiter_panicked,
            #[cfg(test)]
            force_incomplete: self.force_incomplete,
            #[cfg(test)]
            force_sweep_panic: self.force_sweep_panic,
            #[cfg(target_os = "linux")]
            _password_secret: self._password_secret.take(),
            #[cfg(target_os = "linux")]
            _capability_handoff: self._capability_handoff.take(),
            #[cfg(test)]
            final_freeze_scans: Arc::clone(&self.final_freeze_scans),
            #[cfg(test)]
            final_freeze_stable: Arc::clone(&self.final_freeze_stable),
        }
    }
}
