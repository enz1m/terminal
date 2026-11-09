use crate::{
    App, Message,
    operation_origin::RequestGeneration,
    pinned_secret::{self, PanePayload, Reference},
};
use enzui_core::{
    close_guard::{CloseDecision, CloseGuardAction, CloseRequest, SaveOutcome, UnsavedItem},
    form::SecretText,
};
use enzui_services::credential_store::CredentialStore;
use iced::Task;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Instant,
};

#[path = "pinned_storage/machinery.rs"]
mod machinery;

use machinery::{
    CLOSE_SETTLE_BUDGET, CLOSE_TICK, CloseStage, Closing, Failure, Info, JobState, Lease, Work,
    WorkerGuard, key, pending, run,
};
pub(crate) use machinery::{CloseDebt, Completion, SecurePins};

impl App {
    pub(crate) fn queue_secure_pin_save(&mut self) -> Task<Message> {
        if self.startup.is_some() || self.startup_sessions_refused {
            return self.finish_secure_exit();
        }
        self.sync_pinned_capture();
        let abandoned = self
            .secure_pins
            .in_flight
            .as_ref()
            .is_some_and(|lease| lease.state.lock().map_or(true, |state| state.abandoned));
        if abandoned {
            if let Some(lease) = self.secure_pins.in_flight.take() {
                if let Some(key) = key(&lease.info) {
                    self.secure_pins
                        .failed
                        .insert(key.to_owned(), Failure::Cancelled);
                }
                if let Info::Save { reference, .. } = &lease.info
                    && !self
                        .secure_pins
                        .pending_deletes
                        .iter()
                        .any(|r| r.key == reference.key)
                {
                    self.secure_pins.pending_deletes.push(reference.clone());
                }
            }
            self.session_save_in_flight = false;
        }
        if self.secure_pins.in_flight.is_some() {
            return Task::none();
        }
        if self.session_store.is_write_blocked() {
            self.session_save_error =
                Some("Saved state requires explicit replacement authorization.".into());
            return self.finish_secure_exit();
        }
        if self.secure_pins.metadata_failed {
            return self.finish_secure_exit();
        }
        for session in &self.sessions {
            if session.pinned && !self.secure_pins.references.contains_key(&session.id) {
                match Reference::new() {
                    Some(reference) => {
                        self.secure_pins
                            .references
                            .insert(session.id.clone(), reference);
                    }
                    None => {
                        self.secure_pins
                            .failed
                            .insert(String::new(), Failure::Invalid);
                    }
                }
            }
        }
        let mut document = self.pinned_snapshot();
        let present: HashSet<_> = document
            .sessions
            .iter()
            .filter_map(|s| s.archive.as_ref().map(|r| r.key.clone()))
            .collect();
        for reference in self.secure_pins.references.values() {
            if !present.contains(&reference.key) {
                pending(&mut document, reference);
            }
        }
        for reference in self.secure_pins.references.values() {
            if self.secure_pins.clear_requested.contains(&reference.key) {
                pending(&mut document, reference);
            }
        }
        let mut work = None;
        for reference in &document.pending_deletes {
            if present.contains(&reference.key)
                && !self.secure_pins.failed.contains_key(&reference.key)
            {
                work = Some(Work::Delete(reference.clone()));
                break;
            }
        }
        if work.is_none()
            && self.endpoint_credentials_ready
            && self.main_window.is_some_and(|id| self.windows.is_open(id))
        {
            for saved in &document.sessions {
                if self.secure_pins.restore_pending.contains(&saved.id)
                    && saved.archive.as_ref().is_some_and(|r| {
                        r.marker.is_some()
                            && !self.secure_pins.failed.contains_key(&r.key)
                            && !document
                                .pending_deletes
                                .iter()
                                .any(|pending| pending.key == r.key)
                    })
                    && (!saved.detached || self.detached_window_of(&saved.id).is_some())
                {
                    work = Some(Work::Load(saved.clone()));
                    break;
                }
            }
        }
        if work.is_none() {
            for saved in &document.sessions {
                let Some(reference) = &saved.archive else {
                    continue;
                };
                if self.secure_pins.restore_pending.contains(&saved.id)
                    || self.secure_pins.failed.contains_key(&reference.key)
                    || document
                        .pending_deletes
                        .iter()
                        .any(|r| r.key == reference.key)
                {
                    continue;
                }
                let Some(session) = self.sessions.iter().find(|s| s.id == saved.id) else {
                    continue;
                };
                let payload: Vec<_> = session
                    .panes
                    .iter()
                    .map(|pane| PanePayload {
                        id: pane.id.clone(),
                        endpoint: pane.endpoint.clone(),
                        cwd: SecretText::new(pane.cwd.clone()),
                        output: self.pinned_output_for(&pane.id),
                    })
                    .collect();
                let has_output = payload.iter().any(|pane| {
                    pane.output.rows().next().is_some() || pane.output.commands().next().is_some()
                });
                if !has_output {
                    if reference.marker.is_some() {
                        work = Some(Work::Delete(reference.clone()));
                    }
                    continue;
                }
                match pinned_secret::seal(
                    &saved.id,
                    reference,
                    self.settings.clamped_scrollback() as usize,
                    &payload,
                ) {
                    Ok((secret, marker)) => {
                        if reference.marker.as_ref() != Some(&marker) {
                            work = Some(Work::Save {
                                info: Info::Save {
                                    owner: saved.id.clone(),
                                    reference: reference.clone(),
                                    marker,
                                },
                                secret,
                            });
                            break;
                        }
                    }
                    Err(()) => {
                        self.secure_pins
                            .failed
                            .insert(reference.key.clone(), Failure::Invalid);
                    }
                }
            }
        }
        if work.is_none()
            && let Some(reference) = document
                .pending_deletes
                .iter()
                .find(|reference| !self.secure_pins.failed.contains_key(&reference.key))
        {
            work = Some(Work::Delete(reference.clone()));
        }
        if let Some(Work::Delete(reference)) = &work {
            pending(&mut document, reference);
        }
        let explicit_empty = std::mem::take(&mut self.empty_save_requested);
        let work = match work {
            Some(work) => work,
            None if document != self.session_snapshot || explicit_empty => Work::Metadata,
            None => return self.finish_secure_exit(),
        };
        self.secure_pins.pending_deletes = document.pending_deletes.clone();
        let generation = RequestGeneration::new();
        let state = Arc::new(Mutex::new(JobState::default()));
        self.secure_pins.in_flight = Some(Lease {
            info: work.info(),
            generation: generation.clone(),
            state: state.clone(),
        });
        self.session_save_in_flight = true;
        let mut guard = WorkerGuard {
            generation: generation.clone(),
            state: state.clone(),
            complete: false,
        };
        let backend: Arc<dyn CredentialStore> =
            if self.secure_pins.capture_mode || self.credential_backend_pinned {
                Arc::new(enzui_services::credential_store::UnavailableCredentialStore)
            } else {
                self.credentials.clone()
            };
        let future = run(
            work,
            document,
            self.session_store.clone(),
            backend,
            generation,
            self.secure_pins.workspace.clone(),
            state,
            self.blocking.clone(),
        );
        Task::perform(
            async move {
                let result = future.await;
                guard.finish();
                result
            },
            |completion| Message::PinnedStorageFinished(Box::new(completion)),
        )
    }

    pub(crate) fn pinned_output_for(&self, pane: &str) -> Arc<crate::pinned_output::PinnedOutput> {
        self.ptys
            .get(pane)
            .map(super::pty::PtySession::pinned_output)
            .or_else(|| self.restored_outputs.get(pane).cloned())
            .or_else(|| self.finished_outputs.get(pane).cloned())
            .unwrap_or_default()
    }
    pub(crate) fn finish_pinned_storage(&mut self, mut result: Completion) {
        if !self
            .secure_pins
            .in_flight
            .as_ref()
            .is_some_and(|lease| lease.generation == result.generation)
        {
            return;
        }
        if !result.generation.is_live() {
            result.failure = Some(Failure::Cancelled);
            result.restored = None;
        }
        self.secure_pins.in_flight = None;
        self.session_save_in_flight = false;
        if result.workspace != self.secure_pins.workspace {
            return;
        }
        if let Some(state) = result.published {
            self.session_snapshot = state.clone();
            self.secure_pins.pending_deletes = state.pending_deletes.clone();
            let keep: HashSet<_> = state
                .sessions
                .iter()
                .filter_map(|s| s.archive.as_ref().map(|r| r.key.clone()))
                .chain(state.pending_deletes.iter().map(|r| r.key.clone()))
                .collect();
            self.secure_pins
                .references
                .retain(|_, r| keep.contains(&r.key));
            for saved in state.sessions {
                if let Some(reference) = saved.archive {
                    self.secure_pins.references.insert(saved.id, reference);
                }
            }
        }
        if result.failure.is_none()
            && let Info::Delete(reference) = &result.info
        {
            self.secure_pins.restore_pending.retain(|id| {
                self.secure_pins
                    .references
                    .get(id)
                    .is_some_and(|r| r.key != reference.key)
            });
            if self.secure_pins.clear_requested.remove(&reference.key) {
                let pane_ids: Vec<_> = self
                    .sessions
                    .iter()
                    .filter(|session| {
                        self.secure_pins
                            .references
                            .get(&session.id)
                            .is_some_and(|r| r.key == reference.key)
                    })
                    .flat_map(|s| s.panes.iter().map(|p| p.id.clone()))
                    .collect();
                for id in pane_ids {
                    if let Some(pty) = self.ptys.get(&id) {
                        pty.clear_pinned_output();
                    }
                    self.finished_outputs.remove(&id);
                    if self.restored_outputs.contains_key(&id) {
                        self.invalidate_pane_coordinates(&id);
                        self.restored_outputs.insert(id.clone(), Arc::default());
                        if let Some(pane) = self
                            .sessions
                            .iter_mut()
                            .flat_map(|s| &mut s.panes)
                            .find(|p| p.id == id)
                        {
                            pane.transcript.clear();
                        }
                    }
                }
            }
            for index in 0..self.sessions.len() {
                if self.sessions[index]
                    .panes
                    .iter()
                    .all(|pane| !self.ptys.contains_key(&pane.id))
                {
                    self.sessions[index].in_app_history =
                        self.restore_output_history(&self.sessions[index].panes);
                }
            }
        }
        if let Some(failure) = result.failure {
            if (!matches!(result.info, Info::Load(_)) && failure.owes_durable_state())
                || (failure == Failure::Cancelled && matches!(result.info, Info::Metadata))
            {
                self.secure_pins.metadata_failed = true;
                self.session_save_error = Some(failure.label().into());
            }
            if let Some(key) = key(&result.info) {
                self.secure_pins.failed.insert(key.to_owned(), failure);
            }
        } else {
            self.session_save_error = None;
            if let Some(key) = key(&result.info) {
                self.secure_pins.failed.remove(key);
            }
        }
        if let Info::Load(saved) = &result.info
            && result.failure.is_none()
        {
            let valid = self
                .sessions
                .iter()
                .find(|s| s.id == saved.id && s.pinned)
                .is_some_and(|s| {
                    s.panes.len() == saved.panes.len()
                        && s.panes.iter().zip(&saved.panes).all(|(p, q)| {
                            p.id == q.id
                                && p.endpoint == q.endpoint
                                && !self.ptys.contains_key(&p.id)
                        })
                })
                && self.secure_pins.references.get(&saved.id) == saved.archive.as_ref();
            if !valid {
                if self.sessions.iter().any(|session| {
                    session.id == saved.id
                        && session.pinned
                        && session
                            .panes
                            .iter()
                            .all(|pane| !self.ptys.contains_key(&pane.id))
                }) {
                    if let Some(reference) = &saved.archive {
                        self.secure_pins
                            .failed
                            .insert(reference.key.clone(), Failure::Invalid);
                    }
                } else {
                    self.secure_pins.restore_pending.remove(&saved.id);
                }
                return;
            }
            if let Some(decoded) = result.restored {
                for (id, (output, cwd)) in decoded.panes {
                    self.invalidate_pane_coordinates(&id);
                    if let Some(pane) = self
                        .sessions
                        .iter_mut()
                        .flat_map(|s| &mut s.panes)
                        .find(|p| p.id == id)
                    {
                        pane.transcript = output.render(&id);
                        pane.cwd = cwd.expose_secret().to_owned();
                    }
                    self.restored_outputs.insert(id, output);
                }
                for index in 0..self.sessions.len() {
                    self.sessions[index].in_app_history =
                        self.restore_output_history(&self.sessions[index].panes);
                    crate::pinned_sessions::restore_member_histories(
                        &mut self.sessions[index].group_members,
                        &self.restored_outputs,
                    );
                }
                self.secure_pins.restore_pending.remove(&saved.id);
            } else if let Some(reference) = &saved.archive {
                self.secure_pins
                    .failed
                    .insert(reference.key.clone(), Failure::Invalid);
            }
        }
    }
    pub(crate) fn note_pinned_removal(&mut self, owner: &str) {
        if let Some(reference) = self.secure_pins.references.get(owner).cloned() {
            self.secure_pins.failed.remove(&reference.key);
            if !self
                .secure_pins
                .pending_deletes
                .iter()
                .any(|item| item.key == reference.key)
            {
                self.secure_pins.pending_deletes.push(reference);
            }
        }
    }

    pub(crate) fn request_pinned_clear(&mut self, reference: Reference) {
        self.secure_pins.failed.remove(&reference.key);
        self.secure_pins.clear_requested.insert(reference.key);
    }

    /// Starts the secure close for `windows`, reviewed as a request named by
    /// `requested`.
    ///
    /// A teardown (`vetoable` false) is a close whose surface is already gone:
    /// it still publishes what it owes, but it cannot be held for a decision.
    pub(crate) fn start_close(
        &mut self,
        requested: iced::window::Id,
        windows: Vec<iced::window::Id>,
        vetoable: bool,
    ) -> Task<Message> {
        self.credential_mutation_admission_open = false;
        self.poll_teardown_debts();
        if let Some(closing) = self.secure_pins.closing.as_mut() {
            for window in windows {
                if !closing.windows.contains(&window) {
                    closing.windows.push(window);
                }
            }
            closing.vetoable &= vetoable;
            // A request arriving while the first one settles must not queue a
            // second write of the same state.
            return Task::none();
        }
        self.secure_pins.closing = Some(Closing {
            windows,
            requested,
            stage: CloseStage::Draining {
                deadline: Instant::now() + CLOSE_SETTLE_BUDGET,
            },
            attempts: 0,
            vetoable,
            reported: None,
        });
        if let Some(lease) = &self.secure_pins.in_flight
            && matches!(lease.info, Info::Load(_))
        {
            lease.cancel();
        }
        let queued = self.queue_secure_pin_save();
        // The review is opened on this turn, while the work it waits for is
        // still running. Without it the guard would first be asked at the
        // moment the debt settles, so the settled publication would open a
        // review instead of releasing the one that held the close.
        if self.close_deadline().is_none() {
            return queued;
        }
        let debt = self.close_debt();
        if let CloseGuardAction::Close(_) = self.review_close(debt) {
            let committed = self.commit_close();
            return Task::batch([queued, committed]);
        }
        queued
    }

    /// Durable debt that must settle before the process may end.
    ///
    /// Pool occupancy is read from the dispatcher itself, because a cancelled
    /// or superseded write keeps its worker until the call returns.
    pub(crate) fn close_debt(&self) -> Option<CloseDebt> {
        // A surviving child outranks every durable write: the close may not be
        // emitted while this process still owns one.
        if !self.teardown_debts.is_empty() {
            return Some(CloseDebt::TeardownIncomplete);
        }
        if let Some(owner) = self.credential_mutation_owner.as_ref() {
            let snapshot = owner.snapshot();
            if snapshot.uncertain_keys() > 0 {
                return Some(CloseDebt::CredentialUncertain);
            }
            if !self.credential_mutations.is_empty()
                || snapshot.admitted() > 0
                || snapshot.is_wire_active()
                || (!snapshot.is_accepting() && !snapshot.is_drained())
            {
                return Some(CloseDebt::CredentialMutations);
            }
        }
        if self.secure_pins.metadata_failed || self.preferences_write_failed {
            return Some(CloseDebt::Failed);
        }
        if self.secure_pins.in_flight.is_some() || self.pending_ssh_host_save.is_some() {
            return Some(CloseDebt::Publishing);
        }
        (self.blocking.snapshot().live() > 0).then_some(CloseDebt::Draining)
    }

    /// The held close's reason, once waiting has been given up on.
    pub(crate) fn close_veto(&self) -> Option<CloseDebt> {
        match self.secure_pins.closing.as_ref()?.stage {
            CloseStage::Vetoed { debt } => Some(debt),
            CloseStage::Draining { .. } => None,
        }
    }

    fn close_deadline(&self) -> Option<Instant> {
        match self.secure_pins.closing.as_ref()?.stage {
            CloseStage::Draining { deadline } => Some(deadline),
            CloseStage::Vetoed { .. } => None,
        }
    }

    /// Gives up waiting once the budget is spent.
    ///
    /// A settled close is still committed by [`App::finish_secure_exit`], which
    /// the funnel reaches only after any newly owed write has been queued; a
    /// tick that closed the windows itself could race that write.
    pub(crate) fn expire_close_budget(&mut self) -> Task<Message> {
        let Some(deadline) = self.close_deadline() else {
            return Task::none();
        };
        if Instant::now() < deadline {
            return Task::none();
        }
        let Some(debt) = self.close_debt() else {
            return Task::none();
        };
        self.give_up_close(debt)
    }

    /// The single action the close banner offers. Discarding owed state is
    /// reachable only after a retry has been asked for and failed.
    pub(crate) fn close_veto_action(&self) -> (&'static str, Message) {
        // Live processes cannot be discarded, so the only offer is another
        // bounded cleanup attempt while the window stays open.
        if self.close_veto() == Some(CloseDebt::TeardownIncomplete) {
            return ("Retry cleanup", Message::RetryTeardown);
        }
        if self.close_discard_offered() {
            ("Close without saving", Message::CloseWithoutSaving)
        } else {
            ("Retry saving", Message::RetryClose)
        }
    }

    /// Re-reads the executor's typed teardown reports and re-arms a close
    /// vetoed by them.
    ///
    /// The executor is already re-attempting every owner on its own thread, so
    /// this route collects results and re-offers any owner the executor was
    /// too busy to admit. It never signals, waits or joins.
    pub(crate) fn retry_teardown(&mut self) -> Task<Message> {
        self.poll_teardown_debts();
        let Some(closing) = self.secure_pins.closing.as_mut() else {
            return Task::none();
        };
        if !matches!(closing.stage, CloseStage::Vetoed { .. }) {
            return Task::none();
        }
        closing.attempts = closing.attempts.saturating_add(1);
        closing.stage = CloseStage::Draining {
            deadline: Instant::now() + CLOSE_SETTLE_BUDGET,
        };
        Task::none()
    }

    fn close_discard_offered(&self) -> bool {
        self.close_veto().is_some_and(|debt| {
            !matches!(
                debt,
                CloseDebt::CredentialMutations
                    | CloseDebt::CredentialUncertain
                    | CloseDebt::TeardownIncomplete
            )
        }) && self
            .secure_pins
            .closing
            .as_ref()
            .is_some_and(|closing| closing.attempts > 0)
    }

    /// Wakes the close reducer while it waits, and stops once it is vetoed.
    pub(crate) fn close_settle_subscription(&self) -> iced::Subscription<Message> {
        match self
            .secure_pins
            .closing
            .as_ref()
            .map(|closing| closing.stage)
        {
            Some(CloseStage::Draining { deadline }) => iced::time::every(CLOSE_TICK)
                .with(deadline)
                .map(|(_deadline, _now)| Message::CloseSettle),
            _ => iced::Subscription::none(),
        }
    }

    /// Re-arms a vetoed close and re-attempts every owed write.
    pub(crate) fn retry_close(&mut self) -> Task<Message> {
        let Some(closing) = self.secure_pins.closing.as_mut() else {
            return Task::none();
        };
        if !matches!(closing.stage, CloseStage::Vetoed { .. }) {
            return Task::none();
        }
        closing.attempts = closing.attempts.saturating_add(1);
        closing.stage = CloseStage::Draining {
            deadline: Instant::now() + CLOSE_SETTLE_BUDGET,
        };
        self.secure_pins.retry();
        self.session_save_error = None;
        let credentials = self.retry_credential_mutations(true);
        let preferences = if std::mem::take(&mut self.preferences_write_failed) {
            self.save_preferences()
        } else {
            Task::none()
        };
        Task::batch([credentials, preferences])
    }

    /// Commits a vetoed close on the user's explicit decision, leaving the
    /// unwritten state recorded rather than pretending it was saved.
    pub(crate) fn close_without_saving(&mut self) -> Task<Message> {
        if !self.close_discard_offered() {
            return Task::none();
        }
        let discarded = matches!(
            self.secure_pins.guard.decide(CloseDecision::Discard),
            CloseGuardAction::Close(_)
        );
        if discarded || self.force_close_approved() {
            return self.commit_close();
        }
        Task::none()
    }

    // The guard is the only authority that may approve a close: it holds the
    // request while durable state is owed, and reads a settled debt as the save
    // it was waiting for.
    fn review_close(&mut self, debt: Option<CloseDebt>) -> CloseGuardAction<&'static str> {
        let Some(requested) = self
            .secure_pins
            .closing
            .as_ref()
            .map(|closing| closing.requested)
        else {
            return CloseGuardAction::Hold;
        };
        if !self.secure_pins.guard.is_pending() {
            let owed = debt.map(|debt| UnsavedItem::new(debt.key(), debt.summary()));
            return self
                .secure_pins
                .guard
                .request(CloseRequest::for_window(requested), owed);
        }
        if debt.is_some() {
            return CloseGuardAction::Hold;
        }
        let CloseGuardAction::Save { key, attempt } =
            self.secure_pins.guard.decide(CloseDecision::SaveAll)
        else {
            return CloseGuardAction::Hold;
        };
        self.secure_pins
            .guard
            .save_finished(&key, &attempt, SaveOutcome::Saved)
            .unwrap_or(CloseGuardAction::Hold)
    }

    // Non-vetoable shutdown, which the guard answers unconditionally.
    fn force_close_approved(&mut self) -> bool {
        let Some(requested) = self
            .secure_pins
            .closing
            .as_ref()
            .map(|closing| closing.requested)
        else {
            return false;
        };
        matches!(
            self.secure_pins.guard.force_close(requested),
            CloseGuardAction::Close(_)
        )
    }

    fn commit_close(&mut self) -> Task<Message> {
        let Some(closing) = self.secure_pins.closing.take() else {
            return Task::none();
        };
        if self.secure_pins.failed() {
            eprintln!("{}", pinned_secret::NOT_SAVED);
        }
        // Final shutdown stops center admission and joins the connector before
        // the surfaces go.
        #[cfg(feature = "center")]
        self.shutdown_center();
        // Any session still live at commit time is retired here. Leaving one to
        // a later field drop would abandon a child after the exit was already
        // authorized, so this is what makes the exit gate total.
        self.retire_every_live_pty();
        // The app state that held these tickets is about to go away, so every
        // owner it still retained is offered to the process executor.
        self.transfer_teardown_debts_to_process();
        let mut tasks = Vec::new();
        for window in closing.windows {
            // The Closed events these produce are this app's own: they retire
            // ledger and window state and nothing else.
            self.intentional_closes
                .insert(window, crate::CloseIntent::Teardown);
            tasks.push(iced::window::close(window));
        }
        // The exit is the executor's decision, not this route's. While it still
        // owns a child the process stays alive with no surfaces, because ending
        // it would abandon that child rather than settle it.
        tasks.push(self.exit_when_settled());
        Task::batch(tasks)
    }

    /// Emit the process exit only once the executor proves it owns and charges
    /// nothing.
    ///
    /// Authority is the executor's own poll, never a copied outcome or a static
    /// vector. The poll never waits: an executor that has not finished is
    /// reported as held and retried on the next tick. Lost custody is not a
    /// settlement — it withholds the exit for as long as the process runs,
    /// unless a replacement executor settles the exact owners that remain.
    pub(crate) fn exit_when_settled(&mut self) -> Task<Message> {
        match self.exit_authority() {
            crate::pty::ExitAuthority::Clear => {
                self.exit_withheld = false;
                iced::exit()
            }
            crate::pty::ExitAuthority::Held(debt) => {
                self.exit_withheld = true;
                crate::blocking_dispatch::diagnostic("exit-held", "pty-teardown", debt.owners());
                Task::none()
            }
            crate::pty::ExitAuthority::Uncertain(debt) => {
                // Nobody can account for this custody, so there is nothing to
                // authorize. The tick keeps retrying a replacement executor.
                self.exit_withheld = true;
                crate::blocking_dispatch::diagnostic(
                    "exit-teardown-uncertain",
                    "pty-teardown",
                    debt.owners() + debt.lost + debt.reserved,
                );
                Task::none()
            }
        }
    }

    /// Wakes the reducer while an authorized exit waits for the executor.
    pub(crate) fn teardown_exit_subscription(&self) -> iced::Subscription<Message> {
        if self.exit_withheld {
            iced::time::every(CLOSE_TICK).map(|_| Message::TeardownPoll)
        } else {
            iced::Subscription::none()
        }
    }

    /// Reviews a held close, and commits it once nothing is owed.
    ///
    /// Call this after the funnel's queue pass: that pass takes any newly owed
    /// write first, so a close that still owes something is held here. A close
    /// whose budget is already spent keeps its veto, which only the user's
    /// retry or discard moves.
    pub(crate) fn settle_close(&mut self) -> Task<Message> {
        if self.secure_pins.closing.is_none() {
            return Task::none();
        }
        self.poll_teardown_debts();
        self.drain_credential_owner_if_ready();
        if let Some(debt) = self.close_debt() {
            let live = self.blocking.snapshot().live();
            if let Some(closing) = self.secure_pins.closing.as_mut()
                && closing.reported != Some(debt)
            {
                closing.reported = Some(debt);
                crate::blocking_dispatch::diagnostic("close-held", debt.name(), live);
            }
            return Task::none();
        }
        self.finish_secure_exit()
    }

    // Reached once no write is owed, so the publication ordering the funnel
    // enforces still holds here.
    fn finish_secure_exit(&mut self) -> Task<Message> {
        let Some(deadline) = self.close_deadline() else {
            return Task::none();
        };
        self.drain_credential_owner_if_ready();
        let debt = self.close_debt();
        if let CloseGuardAction::Close(_) = self.review_close(debt) {
            return self.commit_close();
        }
        let Some(debt) = debt else {
            return Task::none();
        };
        if !debt.is_failure() && Instant::now() < deadline {
            return Task::none();
        }
        self.give_up_close(debt)
    }

    fn drain_credential_owner_if_ready(&self) {
        // Profile jobs can submit credential work after their metadata completes.
        let Some(owner) = self.credential_mutation_owner.as_ref() else {
            return;
        };
        let snapshot = owner.snapshot();
        if self.secure_pins.in_flight.is_none()
            && self.pending_ssh_host_save.is_none()
            && self.credential_mutations.is_empty()
            && snapshot.admitted() == 0
            && !snapshot.is_wire_active()
            && snapshot.uncertain_keys() == 0
        {
            owner.drain();
        }
    }

    // A destroyed surface cannot report ordinary persistence failure, but the
    // process must still retain an owner with accepted or uncertain mutations.
    fn give_up_close(&mut self, debt: CloseDebt) -> Task<Message> {
        crate::blocking_dispatch::diagnostic("close", debt.name(), self.blocking.snapshot().live());
        if matches!(
            debt,
            CloseDebt::CredentialMutations | CloseDebt::CredentialUncertain
        ) && let Some(owner) = self.credential_mutation_owner.as_ref()
        {
            let snapshot = owner.snapshot();
            eprintln!(
                "terminal credential close held: admitted={} wire_active={} drained={} uncertain_keys={}",
                snapshot.admitted(),
                snapshot.is_wire_active(),
                snapshot.is_drained(),
                snapshot.uncertain_keys()
            );
        }
        let vetoable = self
            .secure_pins
            .closing
            .as_ref()
            .is_some_and(|closing| closing.vetoable);
        if !vetoable {
            // A destroyed surface cannot present Retry or Keep Open, so this
            // close commits. Committing closes surfaces only: every exact
            // teardown owner moves to the process executor, and the exit stays
            // withheld until that executor owns nothing.
            if matches!(
                debt,
                CloseDebt::CredentialMutations | CloseDebt::CredentialUncertain
            ) {
                return Task::none();
            }
            if self.force_close_approved() {
                return self.commit_close();
            }
            return Task::none();
        }
        if let Some(closing) = self.secure_pins.closing.as_mut() {
            closing.stage = CloseStage::Vetoed { debt };
        }
        Task::none()
    }

    #[cfg(test)]
    pub(crate) fn rewind_close_deadline(&mut self) {
        if let Some(closing) = self.secure_pins.closing.as_mut()
            && let CloseStage::Draining { deadline } = &mut closing.stage
        {
            *deadline = Instant::now();
        }
    }

    #[cfg(test)]
    pub(crate) fn closing_windows(&self) -> Option<usize> {
        self.secure_pins
            .closing
            .as_ref()
            .map(|closing| closing.windows.len())
    }
}
