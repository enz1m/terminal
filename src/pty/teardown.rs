//! PTY teardown supervision.

use super::*;

/// Why a settled teardown could not be proved complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TeardownClass {
    /// Owned descendants were still live when the post-KILL ceiling expired.
    ResidualCleanup,
    /// The leader's signal was undeliverable while that exact leader was still
    /// present.
    LeaderUnsignalled,
    /// The exact leader was still present after its waiter settled, so its
    /// status was never reaped.
    LeaderUnreaped,
    /// A worker outlived its bounded join and its handle moved to the reaper.
    WorkerParked,
    /// The pre-KILL freeze never reached two consecutive empty snapshots, so
    /// the fork window was never proved closed.
    UncertainFreeze,
    /// No pass has run yet, so nothing has been proved either way.
    Pending,
}

impl TeardownClass {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ResidualCleanup => "residual-cleanup",
            Self::LeaderUnsignalled => "leader-unsignalled",
            Self::LeaderUnreaped => "leader-unreaped",
            Self::WorkerParked => "worker-parked",
            Self::UncertainFreeze => "uncertain-freeze",
            Self::Pending => "pending",
        }
    }
}

/// What one [`PtySession::shutdown`] proved about a session's processes,
/// workers and descriptors.
///
/// Every field is a settled fact rather than an attempt, so an incomplete
/// outcome is teardown debt the caller still owns. Leader custody is reported
/// separately from descendant custody: a leader that could not be signalled or
/// reaped is incomplete even when no descendant survives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PtyTeardownOutcome {
    /// A pass has run. An unattempted teardown proves nothing, and must never
    /// read as a settlement.
    pub(crate) attempted: bool,
    pub(crate) freeze_stable: bool,
    pub(crate) residual_descendants: usize,
    pub(crate) leader_signalled: bool,
    pub(crate) leader_reaped: bool,
    pub(crate) waiter_joined: bool,
    pub(crate) reader_joined: bool,
    /// The input writer thread ended, releasing the PTY writer it owned. An
    /// unjoined writer is debt: queued input and the write handle are still
    /// alive somewhere.
    pub(crate) writer_joined: bool,
    /// Both pty halves, the cancellation pipe and both `/proc` handoffs were
    /// released.
    pub(crate) descriptors_released: bool,
    /// A bounded phase ended on its deadline rather than on its condition.
    pub(crate) timed_out: bool,
    /// Worker handles still running, so their custody is still retained.
    pub(crate) parked_workers: usize,
    /// The reader thread unwound. Terminal, not parked: its custody is
    /// released, but the fact stays reportable.
    pub(crate) reader_panicked: bool,
    /// The waiter unwound. Terminal, but it may have unwound before reaping,
    /// which leaves leader debt for the owner to finish.
    pub(crate) waiter_panicked: bool,
    /// The input writer unwound. Terminal, and its queued input is gone with
    /// it.
    pub(crate) writer_panicked: bool,
}

impl PtyTeardownOutcome {
    /// Nothing has been attempted yet: every fact is still unproved, so this is
    /// debt until a pass reports otherwise.
    pub(crate) const fn pending() -> Self {
        Self {
            attempted: false,
            freeze_stable: false,
            residual_descendants: 0,
            leader_signalled: false,
            leader_reaped: false,
            waiter_joined: false,
            reader_joined: false,
            writer_joined: false,
            descriptors_released: false,
            timed_out: false,
            parked_workers: 0,
            reader_panicked: false,
            waiter_panicked: false,
            writer_panicked: false,
        }
    }

    /// The outcome of a session that owns no process, worker or descriptor.
    pub(crate) const fn settled() -> Self {
        Self {
            attempted: true,
            freeze_stable: true,
            residual_descendants: 0,
            leader_signalled: true,
            leader_reaped: true,
            waiter_joined: true,
            reader_joined: true,
            writer_joined: true,
            descriptors_released: true,
            timed_out: false,
            parked_workers: 0,
            reader_panicked: false,
            waiter_panicked: false,
            writer_panicked: false,
        }
    }

    /// Whether every process, worker and descriptor this session owned is gone.
    pub(crate) const fn is_complete(self) -> bool {
        self.attempted
            && self.freeze_stable
            && self.residual_descendants == 0
            && self.leader_signalled
            && self.leader_reaped
            && self.waiter_joined
            && self.reader_joined
            && self.writer_joined
            && self.descriptors_released
            && !self.timed_out
            && self.parked_workers == 0
    }

    /// The single most concrete reason this teardown is incomplete.
    ///
    /// An observed survivor outranks an unproved race, and leader custody
    /// outranks a parked worker.
    pub(crate) const fn class(self) -> Option<TeardownClass> {
        if !self.attempted {
            Some(TeardownClass::Pending)
        } else if self.residual_descendants > 0 {
            Some(TeardownClass::ResidualCleanup)
        } else if !self.leader_signalled {
            Some(TeardownClass::LeaderUnsignalled)
        } else if !self.leader_reaped {
            Some(TeardownClass::LeaderUnreaped)
        } else if self.parked_workers > 0
            || !self.reader_joined
            || !self.waiter_joined
            || !self.writer_joined
        {
            Some(TeardownClass::WorkerParked)
        } else if !self.freeze_stable || self.timed_out || !self.descriptors_released {
            Some(TeardownClass::UncertainFreeze)
        } else {
            None
        }
    }
}

/// Record one bounded operational line for an incomplete teardown.
///
/// This is a diagnostic, never the user-visible settlement: the returned
/// [`PtyTeardownOutcome`] is what holds a close. Carries only counts and
/// booleans, because a pid, tty name, path or host would put native or
/// persisted bytes on the channel.
pub(crate) fn report_teardown(outcome: &PtyTeardownOutcome, freeze_scans: u64) {
    let class = outcome.class().map_or("incomplete", TeardownClass::as_str);
    eprintln!(
        "terminal-pty-teardown: class={class} scans={freeze_scans} \
         residual={} leader_signalled={} leader_reaped={} waiter_joined={} \
         reader_joined={} writer_joined={} descriptors_released={} timed_out={} \
         parked={} reader_panicked={} waiter_panicked={} writer_panicked={}",
        outcome.residual_descendants,
        outcome.leader_signalled,
        outcome.leader_reaped,
        outcome.waiter_joined,
        outcome.reader_joined,
        outcome.writer_joined,
        outcome.descriptors_released,
        outcome.timed_out,
        outcome.parked_workers,
        outcome.reader_panicked,
        outcome.waiter_panicked,
        outcome.writer_panicked,
    );
}

/// The exact identity of one reserved teardown, allocated before its child.
///
/// A settlement is always asked of one token, so a sweep that discharges one
/// retained teardown can never discharge another.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TeardownToken(u64);

impl TeardownToken {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

/// Move-only custody of one retired session that has not finished tearing down.
///
/// This owns the retiring [`PtySession`] itself, so the pinned leader identity
/// and pidfd, every discovered descendant pidfd, the reap handshake, all three
/// worker handles, the input queue, both pty halves, the cancellation pipe and
/// both `/proc` handoffs all stay owned, together with the reservation that
/// keeps this teardown charged against capacity. The outcome is metadata about
/// the last pass; this value is the authority that can still finish the work.
pub(crate) struct PtyTeardownDebtOwner {
    session: PtySession,
    outcome: PtyTeardownOutcome,
    /// Passes that unwound. Bounded, so one malformed teardown is taken out of
    /// rotation rather than retried forever.
    sweep_panics: u32,
}

impl PtyTeardownDebtOwner {
    pub(crate) fn new(mut session: PtySession, outcome: PtyTeardownOutcome) -> Self {
        // The session now lives inside an owner, so its own `Drop` must not
        // start a second transfer for this one reservation.
        session.custody = CustodyState::InOwner;
        Self {
            session,
            outcome,
            sweep_panics: 0,
        }
    }

    fn record_sweep_panic(&mut self) {
        self.sweep_panics = self.sweep_panics.saturating_add(1);
    }

    /// Whether this owner's passes have unwound too often to keep retrying.
    fn is_stalled(&self) -> bool {
        self.sweep_panics >= TEARDOWN_PANIC_LIMIT
    }

    pub(crate) fn token(&self) -> TeardownToken {
        self.session.teardown_token
    }

    /// Run one more bounded exact signal, reap and join pass over this owner's
    /// own retained resources, and republish its outcome.
    pub(crate) fn sweep(&mut self) -> PtyTeardownOutcome {
        self.outcome = self.session.shutdown();
        self.outcome
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.outcome.is_complete()
    }

    /// Let this owner's next pass complete, for a route control that must not
    /// depend on a genuinely stuck process.
    #[cfg(test)]
    pub(crate) fn allow_complete_teardown(&mut self) {
        self.session.force_incomplete = false;
    }
}

impl std::fmt::Debug for PtyTeardownDebtOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyTeardownDebtOwner")
            .field("token", &self.token().get())
            .field("complete", &self.is_complete())
            .finish_non_exhaustive()
    }
}

struct SupervisorState {
    /// Owners awaiting a pass. Never emptied wholesale: a claim removes exactly
    /// the owner it will sweep and records its token in `claimed`, so a
    /// concurrent observer can never read this as "nothing is owned".
    queue: VecDeque<PtyTeardownDebtOwner>,
    /// Tokens currently leased out of `queue` for a pass. A lease leaves this
    /// list only by publishing, requeueing, or recording the token as lost.
    claimed: Vec<TeardownToken>,
    /// Owners whose passes kept unwinding. Still owned and still charged, but
    /// out of rotation so one malformed teardown cannot spin the executor.
    stalled: Vec<PtyTeardownDebtOwner>,
    /// Typed results the reducer has not collected. Metadata only; no custody.
    reports: Vec<TeardownReport>,
    /// Live sessions plus unresolved teardowns, together.
    reserved: usize,
    next_token: u64,
    stopping: bool,
    worker: Option<thread::JoinHandle<()>>,
    /// No executor is running while owners remain. Cleared only once a
    /// replacement handle has actually been retained.
    lost: bool,
    /// Tokens whose custody can no longer be accounted for. Never cleared, and
    /// their reservations are never returned.
    lost_tokens: Vec<TeardownToken>,
    /// Refuses this many executor starts, so a control can observe lost
    /// custody without arranging a real thread failure.
    #[cfg(test)]
    fail_spawns: u32,
}

/// One typed teardown result, which the reducer may read without touching
/// custody.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TeardownReport {
    pub(crate) token: TeardownToken,
    pub(crate) outcome: PtyTeardownOutcome,
    /// This token owns nothing further: its owner has been released. Only a
    /// complete pass sets this, so an unwound pass never reads as settled.
    pub(crate) settled: bool,
}

/// Process-level teardown debt, counted per storage site.
///
/// Counted independently of the queue's contents: leased owners, stalled
/// owners, charged reservations and unaccountable tokens are reported
/// separately, so an observer can never see an empty queue while custody is in
/// flight or merely charged.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProcessTeardownDebt {
    pub(crate) queued: usize,
    pub(crate) claimed: usize,
    pub(crate) stalled: usize,
    /// Slots charged against capacity. A charged slot is proof that some
    /// session or teardown exists, even if no other count sees it yet.
    pub(crate) reserved: usize,
    pub(crate) lost: usize,
}

impl ProcessTeardownDebt {
    pub(crate) const fn owners(self) -> usize {
        self.queued + self.claimed + self.stalled
    }

    /// Whether this process owns and charges nothing at all.
    ///
    /// A charged reservation counts: it is proof that a live or unsettled
    /// session exists even when a bug or an in-flight transfer has left the
    /// queue and the lease list momentarily empty.
    pub(crate) const fn is_clear(self) -> bool {
        self.owners() == 0 && self.reserved == 0 && self.lost == 0
    }
}

/// Whether the process may end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExitAuthority {
    /// Nothing is queued, leased, stalled, charged or unaccounted for, and the
    /// executor joined cleanly. The only state that authorizes an exit.
    Clear,
    /// Exact owners or charged reservations remain, so the process lifecycle
    /// must stay alive.
    Held(ProcessTeardownDebt),
    /// Custody is unknown: no executor is running, one unwound, or a token
    /// could not be accounted for. This withholds the exit rather than
    /// authorizing it, and is only left by a replacement executor settling the
    /// exact owners that remain.
    Uncertain(ProcessTeardownDebt),
}

/// What a poll found the executor thread doing.
enum WorkerPoll {
    NeverStarted,
    Finished,
    Running,
}

/// One leased owner, held for the duration of its pass.
///
/// The token leaves `claimed` only through [`ClaimLease::publish`] or
/// `ClaimLease::requeue`. If the pass unwinds past both, `Drop` returns the
/// owner to the queue when it still exists and otherwise records the token as
/// permanently unknown, so `claimed` never names a token with no owner and no
/// route depends on the session's nested drop order.
struct ClaimLease<'a> {
    supervisor: &'a TeardownSupervisor,
    pub(crate) token: TeardownToken,
    owner: Option<PtyTeardownDebtOwner>,
    resolved: bool,
}

impl ClaimLease<'_> {
    /// Run one bounded pass on the leased owner.
    fn sweep(&mut self) -> PtyTeardownOutcome {
        match self.owner.as_mut() {
            Some(owner) => owner.sweep(),
            None => PtyTeardownOutcome::pending(),
        }
    }

    /// Publish the pass result and release or requeue the owner.
    ///
    /// Returns whether this token is now settled.
    fn publish(&mut self, outcome: PtyTeardownOutcome) -> bool {
        let settled = self
            .owner
            .as_ref()
            .is_some_and(PtyTeardownDebtOwner::is_complete);
        let released = {
            let mut state = self.supervisor.lock();
            state.claimed.retain(|held| *held != self.token);
            state.reports.push(TeardownReport {
                token: self.token,
                outcome,
                settled,
            });
            match self.owner.take() {
                Some(owner) if settled => Some(owner),
                Some(owner) => {
                    state.queue.push_back(owner);
                    None
                }
                // The pass consumed the owner, which is custody nobody can
                // account for any more.
                None => {
                    record_lost_in(&mut state, self.token);
                    None
                }
            }
        };
        self.resolved = true;
        // A completed owner drops with no lock held, because releasing its
        // reservation re-locks.
        drop(released);
        settled
    }

    /// Requeue the owner after an unwound pass, publishing an uncertain report.
    ///
    /// Never marks the token settled. An owner that keeps unwinding is moved
    /// out of rotation rather than retried forever; it stays owned and stays
    /// charged.
    fn requeue_uncertain(&mut self) {
        let mut state = self.supervisor.lock();
        state.claimed.retain(|held| *held != self.token);
        state.reports.push(TeardownReport {
            token: self.token,
            outcome: PtyTeardownOutcome::pending(),
            settled: false,
        });
        match self.owner.take() {
            Some(mut owner) => {
                owner.record_sweep_panic();
                if owner.is_stalled() {
                    state.stalled.push(owner);
                } else {
                    state.queue.push_back(owner);
                }
            }
            None => record_lost_in(&mut state, self.token),
        }
        self.resolved = true;
    }
}

impl Drop for ClaimLease<'_> {
    fn drop(&mut self) {
        if self.resolved {
            return;
        }
        let mut state = self.supervisor.lock();
        state.claimed.retain(|held| *held != self.token);
        match self.owner.take() {
            // Pushed rather than dropped, so no nested session drop runs while
            // this lock is held.
            Some(owner) => state.queue.push_back(owner),
            None => record_lost_in(&mut state, self.token),
        }
    }
}

/// Record one token as unaccountable, at most once.
fn record_lost_in(state: &mut SupervisorState, token: TeardownToken) {
    if !state.lost_tokens.contains(&token) {
        state.lost_tokens.push(token);
    }
}

/// Process-level supervisor that owns exact PTY teardown custody off the
/// reducer.
///
/// A slot and a token are reserved before a child is spawned, so an unresolved
/// teardown is always accounted for and never needs an unreserved fallback.
/// Every signal, wait and join runs on this supervisor's own thread; the
/// reducer only submits owners and collects typed reports.
pub(crate) struct TeardownSupervisor {
    state: Mutex<SupervisorState>,
    wake: Condvar,
    /// Live sessions plus unresolved teardowns this process may own at once.
    capacity: usize,
    /// Owners the executor admits at once. A refusal leaves the owner with its
    /// caller, where it stays visible as that caller's own debt.
    admission: usize,
}

impl TeardownSupervisor {
    fn lock(&self) -> std::sync::MutexGuard<'_, SupervisorState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reserve one slot and token, or `None` at capacity or token exhaustion.
    ///
    /// Exhaustion refuses the new child and leaves existing debt untouched; the
    /// counter never wraps. No pass runs here, so spawning never pays a stuck
    /// teardown's budget.
    pub(crate) fn reserve(&'static self) -> Option<ReaperReservation> {
        let mut state = self.lock();
        if state.reserved >= self.capacity {
            return None;
        }
        let next = state.next_token.checked_add(1)?;
        state.next_token = next;
        state.reserved += 1;
        Some(ReaperReservation {
            supervisor: self,
            token: TeardownToken(next),
            charged: true,
        })
    }

    /// Typed process debt, read per storage site.
    pub(crate) fn debt(&self) -> ProcessTeardownDebt {
        let state = self.lock();
        ProcessTeardownDebt {
            queued: state.queue.len(),
            claimed: state.claimed.len(),
            stalled: state.stalled.len(),
            reserved: state.reserved,
            lost: state.lost_tokens.len(),
        }
    }

    /// Record one token as unaccountable. Idempotent.
    pub(crate) fn record_lost(&self, token: TeardownToken) {
        record_lost_in(&mut self.lock(), token);
    }

    /// Hand one exact owner to the executor.
    ///
    /// Constant time and never blocking, because the caller may be the reducer.
    /// Refused once the admission limit is reached or a stop has been
    /// requested; the owner comes back to the caller, which keeps it visible.
    #[expect(
        clippy::result_large_err,
        reason = "Refusal returns the exact teardown owner without a new allocation"
    )]
    pub(crate) fn submit(
        &'static self,
        owner: PtyTeardownDebtOwner,
    ) -> Result<(), PtyTeardownDebtOwner> {
        {
            let mut state = self.lock();
            if state.stopping || state.queue.len() + state.claimed.len() >= self.admission {
                return Err(owner);
            }
            state.queue.push_back(owner);
            self.ensure_worker(&mut state);
        }
        self.wake.notify_all();
        Ok(())
    }

    /// Take custody unconditionally, for a caller that cannot retain an owner.
    ///
    /// A fallback `Drop` and a close that is discarding its app state both have
    /// nowhere else to put an owner, so the admission limit does not apply:
    /// refusing here would release descriptors and join handles a live child is
    /// still using. Capacity already bounds how many owners can exist at all.
    ///
    /// Custody arriving here also un-does a stop request, because new debt
    /// means the process is no longer clear to end.
    pub(crate) fn adopt_final(&'static self, owner: PtyTeardownDebtOwner) {
        {
            let mut state = self.lock();
            state.stopping = false;
            state.queue.push_back(owner);
            self.ensure_worker(&mut state);
        }
        self.wake.notify_all();
    }

    /// Take every typed result the executor has published.
    ///
    /// Reports are metadata: collecting them moves no custody and settles
    /// nothing by itself.
    pub(crate) fn collect_reports(&self) -> Vec<TeardownReport> {
        std::mem::take(&mut self.lock().reports)
    }

    /// Start an executor if none is retained.
    ///
    /// Never starts a replacement while a handle is still held: a running or
    /// unjoined worker may still be touching the owners this one would claim.
    /// `lost` is cleared only after the new handle has been retained.
    fn ensure_worker(&'static self, state: &mut SupervisorState) {
        if state.worker.is_some() {
            return;
        }
        #[cfg(test)]
        if state.fail_spawns > 0 {
            state.fail_spawns -= 1;
            state.lost = true;
            return;
        }
        match thread::Builder::new()
            .name("terminal-emulator-teardown".into())
            .spawn(move || self.run())
        {
            Ok(handle) => {
                state.worker = Some(handle);
                state.lost = false;
            }
            // Without an executor the queued owners cannot be settled at all,
            // which is unknown custody the exit route must withhold on.
            Err(_) => state.lost = true,
        }
    }

    fn run(&'static self) {
        loop {
            let Some(mut lease) = self.claim() else {
                if self.park_or_finish() {
                    return;
                }
                continue;
            };
            // One malformed teardown must not stop progress for unrelated
            // owners, and must never publish itself as settled. The owner is
            // only borrowed by the pass, so it survives the unwind and goes
            // back into rotation.
            let swept = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lease.sweep()));
            let settled = match swept {
                Ok(outcome) => lease.publish(outcome),
                Err(_) => {
                    lease.requeue_uncertain();
                    false
                }
            };
            drop(lease);
            // One edge-triggered bell per round: the reducer collects typed
            // reports on that wake rather than polling a timer.
            wake_bus().notify();
            if !settled {
                thread::sleep(TEARDOWN_ROUND_PAUSE);
            }
        }
    }

    /// Lease the next owner, recording its token so the debt count still
    /// includes it while its pass runs.
    fn claim(&self) -> Option<ClaimLease<'_>> {
        let mut state = self.lock();
        let owner = state.queue.pop_front()?;
        let token = owner.token();
        state.claimed.push(token);
        Some(ClaimLease {
            supervisor: self,
            token,
            owner: Some(owner),
            resolved: false,
        })
    }

    /// Park until there is work, or finish once a stop leaves nothing owned.
    fn park_or_finish(&self) -> bool {
        let state = self.lock();
        if state.stopping && state.queue.is_empty() && state.claimed.is_empty() {
            return true;
        }
        let (parked, _) = self
            .wake
            .wait_timeout(state, TEARDOWN_IDLE_PARK)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(parked);
        false
    }

    /// Report whether the process may exit, and retry a lost executor.
    ///
    /// Poll-only: an executor that has not finished is reported rather than
    /// waited on, so the reducer never blocks here. Only
    /// [`ExitAuthority::Clear`] authorizes an exit, and it requires an empty
    /// queue, no lease, no stalled owner, no charged reservation, no
    /// unaccountable token, and a clean join.
    pub(crate) fn poll_exit_authority(&'static self) -> ExitAuthority {
        // A lost executor is retried here, which is the only route that runs
        // while an authorized exit waits. Never while a handle is retained.
        //
        // Retried even with nothing owed: a worker that unwound after the last
        // owner settled, or before claiming anything, leaves loss with no owed
        // owner to trigger a restart, and that must not be a permanent exit
        // block. With zero debt the replacement observes the stop at once and
        // joins cleanly on the next poll, which is what clears the loss.
        {
            let mut state = self.lock();
            if state.lost && state.worker.is_none() {
                self.ensure_worker(&mut state);
            }
        }
        self.wake.notify_all();
        if self.lock().lost {
            // The replacement could not be started either, so this stays
            // unknown and is retried on a later poll.
            return ExitAuthority::Uncertain(self.debt());
        }
        let debt = self.debt();
        if debt.lost > 0 {
            // A replacement's clean join clears supervisor loss, never a token
            // whose owner nobody holds: that custody is unaccountable for the
            // rest of the process.
            return ExitAuthority::Uncertain(debt);
        }
        if !debt.is_clear() {
            return ExitAuthority::Held(debt);
        }
        // Nothing is owned or charged, so the executor may be asked to finish.
        // Requesting the stop earlier would refuse admission to owners still
        // being retired, which is how custody gets stranded.
        self.lock().stopping = true;
        self.wake.notify_all();
        let poll = {
            let state = self.lock();
            match state.worker.as_ref() {
                None => WorkerPoll::NeverStarted,
                Some(handle) if handle.is_finished() => WorkerPoll::Finished,
                Some(_) => WorkerPoll::Running,
            }
        };
        match poll {
            WorkerPoll::NeverStarted => ExitAuthority::Clear,
            WorkerPoll::Running => ExitAuthority::Held(debt),
            WorkerPoll::Finished => self.join_finished_worker(),
        }
    }

    /// Join a worker already observed finished, and re-read the debt.
    ///
    /// The handle is removed first, so no route can join it twice or start a
    /// replacement while it is still held. An unwound worker may have returned
    /// owners to the queue during that unwind, so the decision is made on the
    /// debt read after the join rather than before it. Starting a replacement
    /// is deliberately not this method's job, which is why it needs no static
    /// receiver.
    fn join_finished_worker(&self) -> ExitAuthority {
        // Taken and joined in separate statements, so no lock is held across
        // the join even though the handle was already observed finished.
        let handle = self.lock().worker.take();
        let joined = handle.map(thread::JoinHandle::join);
        if let Some(Err(_)) = joined {
            // Only the loss is recorded here. Starting the replacement is left
            // to the single pump in `poll_exit_authority`, so exactly one route
            // decides that and it can never run while a handle is retained.
            self.lock().lost = true;
            return ExitAuthority::Uncertain(self.debt());
        }
        let debt = self.debt();
        if self.lock().lost || debt.lost > 0 {
            ExitAuthority::Uncertain(debt)
        } else if debt.is_clear() {
            ExitAuthority::Clear
        } else {
            ExitAuthority::Held(debt)
        }
    }

    /// Queue an owner without starting the executor, so a settlement control
    /// can drive exact passes itself.
    #[cfg(test)]
    pub(crate) fn adopt(&self, owner: PtyTeardownDebtOwner) {
        self.lock().queue.push_back(owner);
    }

    /// Refuse the next `starts` executor starts, for a control that must
    /// observe lost custody without arranging a real thread failure.
    #[cfg(test)]
    pub(crate) fn fail_next_spawns_for_test(&self, starts: u32) {
        self.lock().fail_spawns = starts;
    }

    /// Install an executor that unwinds at once, so a control can observe a
    /// failed supervisor join with nothing owed.
    ///
    /// Any handle already retained is detached rather than joined; a parked
    /// executor returns on its next wake once a stop is requested.
    #[cfg(test)]
    pub(crate) fn fail_worker_for_test(&self) {
        let handle = thread::Builder::new()
            .name("terminal-emulator-teardown-probe".into())
            .spawn(|| panic!("injected executor failure"))
            .expect("the probe executor must start");
        self.lock().worker = Some(handle);
    }

    /// Whether this executor is currently recorded as lost.
    #[cfg(test)]
    pub(crate) fn is_lost_for_test(&self) -> bool {
        self.lock().lost
    }

    /// Lease one exact owner by token, run one pass on the calling thread, and
    /// return its outcome; `None` once that token owns nothing.
    ///
    /// The token-addressed form a settlement control uses. It leases through
    /// the same RAII path as the executor, so the claim accounting is identical.
    #[cfg(test)]
    pub(crate) fn sweep_token(&self, token: TeardownToken) -> Option<PtyTeardownOutcome> {
        let mut lease = {
            let mut state = self.lock();
            let at = state
                .queue
                .iter()
                .position(|owner| owner.token() == token)?;
            let owner = state.queue.remove(at)?;
            state.claimed.push(token);
            ClaimLease {
                supervisor: self,
                token,
                owner: Some(owner),
                resolved: false,
            }
        };
        let outcome = lease.sweep();
        let settled = lease.publish(outcome);
        (!settled).then_some(outcome)
    }

    /// Whether this executor still owns the given exact token, whether it is
    /// queued, leased or stalled.
    #[cfg(test)]
    pub(crate) fn holds_token(&self, token: TeardownToken) -> bool {
        let state = self.lock();
        state.claimed.contains(&token)
            || state.queue.iter().any(|owner| owner.token() == token)
            || state.stalled.iter().any(|owner| owner.token() == token)
    }

    /// Address one exact retained owner and let its next pass complete.
    #[cfg(test)]
    pub(crate) fn allow_complete_for_test(&self, token: TeardownToken) -> bool {
        let mut state = self.lock();
        match state.queue.iter_mut().find(|owner| owner.token() == token) {
            Some(owner) => {
                owner.allow_complete_teardown();
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(capacity: usize, next_token: u64) -> Self {
        Self {
            state: Mutex::new(SupervisorState::new(next_token)),
            wake: Condvar::new(),
            capacity,
            admission: TEARDOWN_ADMISSION,
        }
    }
}

impl SupervisorState {
    const fn new(next_token: u64) -> Self {
        Self {
            queue: VecDeque::new(),
            claimed: Vec::new(),
            stalled: Vec::new(),
            reports: Vec::new(),
            reserved: 0,
            next_token,
            stopping: false,
            worker: None,
            lost: false,
            lost_tokens: Vec::new(),
            #[cfg(test)]
            fail_spawns: 0,
        }
    }
}

/// One reserved slot and token, charged until its teardown completes.
///
/// Not optional anywhere it is held, so a missing reservation is impossible by
/// construction. [`ReaperReservation::transfer`] moves the charge to a new
/// value instead of releasing it, which is what lets a fallback `Drop` hand
/// custody on rather than give the slot back while a child is still alive.
pub(crate) struct ReaperReservation {
    supervisor: &'static TeardownSupervisor,
    pub(crate) token: TeardownToken,
    charged: bool,
}

impl ReaperReservation {
    /// Move this reservation's charge into a new value, leaving this one inert.
    pub(crate) fn transfer(&mut self) -> Self {
        let moved = Self {
            supervisor: self.supervisor,
            token: self.token,
            charged: self.charged,
        };
        self.charged = false;
        moved
    }

    /// Keep this slot charged for the rest of the process.
    ///
    /// Used when custody became unaccountable: returning the slot would let a
    /// new child take the place of one nobody can account for.
    pub(crate) fn strand(&mut self) {
        self.charged = false;
    }
}

impl Drop for ReaperReservation {
    fn drop(&mut self) {
        if !self.charged {
            return;
        }
        let mut state = self.supervisor.lock();
        state.reserved = state.reserved.saturating_sub(1);
    }
}

/// The process-wide supervisor every PTY session reserves from before spawning.
pub(crate) fn teardown_supervisor() -> &'static TeardownSupervisor {
    static SUPERVISOR: OnceLock<TeardownSupervisor> = OnceLock::new();
    SUPERVISOR.get_or_init(|| TeardownSupervisor {
        state: Mutex::new(SupervisorState::new(0)),
        wake: Condvar::new(),
        capacity: TEARDOWN_REAPER_CAPACITY,
        admission: TEARDOWN_ADMISSION,
    })
}
// The executor settles exact custody on its own thread, so a session and its
// owner have to be movable across threads. A regression here is a design
// error rather than a lint, so it fails at this declaration.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<PtySession>();
    assert_send::<PtyTeardownDebtOwner>();
};

/// How one worker thread settled inside its bounded join.
pub(crate) enum WorkerSettlement {
    /// Joined cleanly, so its custody is released.
    Joined,
    /// Unwound. Terminal rather than parked, and reportable.
    Panicked,
    /// Still running, so its custody is still retained.
    Running(thread::JoinHandle<()>),
}

/// Wait up to `deadline` for `handle` to finish, joining it if it does.
///
/// A still-running handle is returned rather than joined: joining it would
/// block past every signal and cancellation budget already spent, with no
/// remaining bound. A panic is distinguished from a clean join because a waiter
/// that unwound before reaping leaves leader debt behind.
pub(crate) fn join_worker_bounded(
    handle: &mut Option<thread::JoinHandle<()>>,
    joined: &AtomicBool,
    deadline: Instant,
) -> WorkerSettlement {
    let Some(taken) = handle.take() else {
        return WorkerSettlement::Joined;
    };
    loop {
        if taken.is_finished() {
            let clean = taken.join().is_ok();
            joined.store(clean, Ordering::Release);
            return if clean {
                WorkerSettlement::Joined
            } else {
                WorkerSettlement::Panicked
            };
        }
        if Instant::now() >= deadline {
            return WorkerSettlement::Running(taken);
        }
        thread::sleep(WORKER_JOIN_POLL);
    }
}
