use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use enzui_core::{
    blocking_task::BlockingTaskCancellation,
    cancellation::CancelCause,
    form::{FormSubmission, SecretText},
    persist::{PersistError, SaveOp},
};
use iced::Task;

use crate::{
    App, Message, SshHostRecord,
    blocking_dispatch::Latest,
    operation_origin::{OperationOrigin, RequestGeneration},
    settings_store::AppPreferences,
    ssh_credentials::CredentialSlot,
    ssh_host_ids::Reservation,
};

pub(crate) const UNCERTAIN: &str = "Preferences publication could not be confirmed. Credentials were not changed. Restore verified preferences and restart before retrying.";
pub(crate) const SAVING: &str = "Saving connection… Other preference saves will wait. Cancelling cannot undo a publication already in progress.";

#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    NotPublished,
    Published,
    Uncertain,
    Failed(PersistError),
}

pub(crate) struct Pending {
    generation: RequestGeneration,
    form_generation: RequestGeneration,
    origin: Option<OperationOrigin>,
    expected: Vec<SshHostRecord>,
    expected_sequence: u64,
    reservation: Reservation,
    candidate: Vec<SshHostRecord>,
    submitted: Option<FormSubmission>,
    deferred_preferences: bool,
    cancellation: Option<BlockingTaskCancellation<Outcome>>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.generation.retire(CancelCause::Disconnected);
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }
}

impl Pending {
    pub(crate) fn cancel(&self) {
        self.generation.retire(CancelCause::Cancelled);
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }

    pub(crate) fn defer_preferences(&mut self) {
        self.deferred_preferences = true;
    }
}

impl App {
    pub(crate) fn begin_ssh_host_save(
        &mut self,
        reservation: Reservation,
        snapshot: AppPreferences,
        submission: FormSubmission,
        op: SaveOp,
    ) -> Task<Message> {
        let origin = self.operation_origin();
        let Some(form) = self.ssh_form.as_mut() else {
            return Task::none();
        };
        form.restore_submission(&submission);
        if !form.set_saving() {
            self.refuse_ssh_save("The connection form is unavailable. Reopen it and retry.".into());
            return Task::none();
        }
        let generation = RequestGeneration::new();
        self.pending_ssh_host_save = Some(Pending {
            generation: generation.clone(),
            form_generation: form.generation.clone(),
            origin,
            expected: self.ssh_hosts.clone(),
            expected_sequence: self.ssh_host_seq,
            reservation,
            candidate: snapshot.ssh_hosts,
            submitted: Some(submission),
            deferred_preferences: false,
            cancellation: None,
        });
        self.preferences_save_generation
            .retire(CancelCause::Cancelled);
        self.ssh_save_error = None;
        let entry_claimed = Arc::new(AtomicBool::new(false));
        let worker_entry = entry_claimed.clone();
        let worker_generation = generation.clone();
        // No newer preferences operation is prepared while this slot is held:
        // SaveOp's successful superseded no-op is not a publication receipt.
        let (work, _stamp) = self
            .blocking
            .submit_latest(Latest::Preferences, move |context| {
                if worker_entry
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                    || context.is_cancelled()
                    || !worker_generation.is_live()
                {
                    return Outcome::NotPublished;
                }
                match op.execute() {
                    Ok(()) => Outcome::Published,
                    Err(error) => Outcome::Failed(error),
                }
            });
        if let Some(pending) = self.pending_ssh_host_save.as_mut() {
            pending.cancellation = Some(work.cancellation());
        }
        let blocking = self.blocking.clone();
        Task::perform(work, move |settlement| {
            let outcome = settlement.into_result().unwrap_or_else(|error| {
                // The terminal itself is a diagnostic; the user-facing class is
                // decided by whether the write can still have started, which is
                // the only distinction that keeps the profile list truthful.
                crate::blocking_dispatch::diagnostic(
                    "ssh-host",
                    crate::blocking_dispatch::terminal_name(error),
                    blocking.live(),
                );
                // Cancellation may settle before the closure exits. Claiming
                // entry here must also prevent that closure from starting later.
                if entry_claimed
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    Outcome::NotPublished
                } else {
                    Outcome::Uncertain
                }
            });
            Message::SshHostSaveFinished(generation, outcome)
        })
    }

    fn ssh_host_save_current(&self, pending: &Pending) -> bool {
        pending.generation.is_live()
            && self.ssh_form.as_ref().is_some_and(|form| {
                form.generation == pending.form_generation
                    && form.editing_id.as_deref()
                        == pending
                            .reservation
                            .edit_index
                            .map(|index| pending.expected[index].id.as_str())
            })
            && self.ssh_hosts == pending.expected
            && self.ssh_host_seq == pending.expected_sequence
            && pending.origin.as_ref().is_none_or(|origin| {
                self.operation_origin_is_live(origin)
                    && self.sessions.get(self.active).is_some_and(|session| {
                        session.id == origin.session
                            && session
                                .panes
                                .get(session.active_pane)
                                .is_some_and(|pane| pane.id == origin.pane)
                    })
            })
    }

    pub(crate) fn reconcile_ssh_host_save(&mut self) {
        if let Some(pending) = self.pending_ssh_host_save.as_ref()
            && !self.ssh_host_save_current(pending)
        {
            pending.cancel();
        }
        if let Some(pending) = self.pending_ssh_host_save.as_mut()
            && !self
                .ssh_form
                .as_ref()
                .is_some_and(|form| form.generation == pending.form_generation)
        {
            pending.submitted = None;
        }
    }

    pub(crate) fn cancel_ssh_host_save(&self) {
        if let Some(pending) = &self.pending_ssh_host_save {
            pending.cancel();
        }
    }

    pub(crate) fn finish_ssh_host_save(
        &mut self,
        generation: RequestGeneration,
        outcome: Outcome,
    ) -> Task<Message> {
        if !self
            .pending_ssh_host_save
            .as_ref()
            .is_some_and(|pending| pending.generation == generation)
        {
            return Task::none();
        }
        let Some(mut pending) = self.pending_ssh_host_save.take() else {
            return Task::none();
        };
        let current = self.ssh_host_save_current(&pending);
        let deferred = pending.deferred_preferences;
        if !current || !matches!(&outcome, Outcome::Published) {
            let unmodified = matches!(&outcome, Outcome::NotPublished);
            // An I/O error can follow rename, and cancellation cannot stop a
            // running filesystem call. Do not overwrite an unconfirmed result.
            self.ssh_host_save_uncertain |= !unmodified;
            let summary = if unmodified {
                "The connection was not published. Saved profiles and credentials are unchanged. Retry Save connection.".to_owned()
            } else if let Outcome::Failed(error) = &outcome {
                format!("{}. {UNCERTAIN}", error.user_summary())
            } else {
                UNCERTAIN.to_owned()
            };
            if self
                .ssh_form
                .as_ref()
                .is_some_and(|form| form.generation == pending.form_generation)
                && let Some(submission) = pending.submitted.as_ref()
            {
                self.restore_ssh_submission(submission, summary);
            } else {
                self.ssh_save_error = Some(summary);
            }
            return if deferred && !self.ssh_host_save_uncertain {
                self.save_preferences()
            } else {
                Task::none()
            };
        }

        self.retire_connection_requests(&pending.reservation.id);
        self.ssh_hosts = std::mem::take(&mut pending.candidate);
        self.ssh_host_seq = pending.reservation.sequence;
        self.ssh_form = None;
        self.ssh_save_error = None;
        self.ssh_credential_notice = None;
        let id = pending.reservation.id.clone();
        let active_slot = self
            .ssh_hosts
            .iter()
            .find(|host| host.id == id)
            .and_then(|host| Self::secret_slot_for(&host.auth));
        let mut tasks = Vec::new();
        if let Some(submission) = pending.submitted.take() {
            let field = match active_slot {
                Some(CredentialSlot::Password) => Some(crate::model::SSH_FORM_PASSWORD),
                Some(CredentialSlot::Passphrase) => Some(crate::model::SSH_FORM_PASSPHRASE),
                None => None,
            };
            // Empty input preserves the undisplayed credential rather than
            // authorizing its deletion during an ordinary metadata edit.
            if let Some(slot) = active_slot
                && let Some(field) = field
                && let Some(text) = submission
                    .text(&field.into())
                    .filter(|text| !text.is_empty())
                && let Some((_, future)) = self.remember_secret_value_op(
                    id.clone(),
                    slot,
                    SecretText::new(text.to_owned()),
                )
            {
                tasks.push(self.spawn_credential(future));
            }
        }
        // Only an explicit edit authorizes removal of an existing owner's
        // inactive credentials; adding a fresh owner has no such authority.
        if pending.reservation.edit_index.is_some() {
            for slot in CredentialSlot::ALL {
                if Some(slot) != active_slot {
                    tasks.push(self.forget_secret_task(id.clone(), slot));
                }
            }
        }
        if deferred {
            tasks.push(self.save_preferences());
        }
        Task::batch(tasks)
    }
}
