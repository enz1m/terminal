//! Terminal SSH credential methods.

use super::*;

impl App {
    /// Advance a profile deletion that is waiting on its secrets.
    ///
    /// The profile record is removed only when EVERY slot answered with an
    /// outcome that permits removal. One refusal (denied, locked-dismissed,
    /// cancelled, failed) keeps the profile and says so, because a profile
    /// removed while its item survives is a secret nothing can ever reach or
    /// clean up again.
    pub(crate) fn resolve_pending_deletion(
        &mut self,
        origin: &operation_origin::OperationOrigin,
        owner: &str,
        result: &SshCredentialResult,
    ) -> Task<Message> {
        let Some(pending) = self.pending_secret_deletion.as_mut() else {
            return Task::none();
        };
        if pending.profile_id != owner || !pending.operations.contains(&origin.generation) {
            return Task::none();
        }
        pending
            .operations
            .retain(|generation| *generation != origin.generation);
        pending.refused |= !result.permits_profile_removal();
        if !pending.operations.is_empty() {
            return Task::none();
        }
        let pending = self
            .pending_secret_deletion
            .take()
            .expect("pending deletion checked above");
        if pending.refused {
            let name = &pending.profile_name;
            self.ssh_credential_notice = Some(CredentialNotice::info(format!(
                "“{name}” was kept: its remembered secret could not be removed."
            )));
            return Task::none();
        }
        self.ssh_hosts.retain(|host| host.id != pending.profile_id);
        self.save_preferences()
    }

    /// The slot a saved profile's auth mode uses, if it has a secret at all.
    pub(crate) fn secret_slot_for(auth: &SshAuthMode) -> Option<CredentialSlot> {
        match auth {
            SshAuthMode::Password => Some(CredentialSlot::Password),
            SshAuthMode::KeyFile { .. } => Some(CredentialSlot::Passphrase),
            SshAuthMode::Agent => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn edit_ssh_form_text(&mut self, id: &'static str, value: String) -> Task<Message> {
        self.update(Message::SshFormEvent(FormDialogEvent::Edited(
            id.into(),
            FieldValue::Text(value),
        )))
    }

    pub(crate) fn ssh_host_reservation(
        &self,
        editing_id: Option<&str>,
    ) -> Result<ssh_host_ids::Reservation, &'static str> {
        if self.ssh_host_save_uncertain {
            return Err(ssh_host_save::UNCERTAIN);
        }
        if self.pending_ssh_host_save.is_some()
            || self.pending_secret_deletion.is_some()
            || self.pending_delete_ssh_host.is_some()
        {
            return Err(
                "A saved connection operation is still pending. Wait for it to finish before retrying.",
            );
        }
        if self.startup.is_some()
            || self.startup_settings_refused
            || self.settings_store.is_write_blocked()
        {
            return Err(
                "Saved preferences are unavailable. No connection was changed. Restore readable preferences and restart before retrying.",
            );
        }
        let reservation = ssh_host_ids::reserve(&self.ssh_hosts, self.ssh_host_seq, editing_id)
            .map_err(ssh_host_ids::Refusal::summary)?;
        if let Some(index) = reservation.edit_index
            && self
                .ssh_form
                .as_ref()
                .and_then(|form| form.initial_record.as_ref())
                != self.ssh_hosts.get(index)
        {
            return Err(ssh_host_ids::Refusal::MissingEdit.summary());
        }
        if self
            .credential_ops
            .iter()
            .any(|op| op.owner == reservation.id)
        {
            return Err(
                "A credential operation for this connection is still pending. Wait for it to finish before retrying.",
            );
        }
        Ok(reservation)
    }

    pub(crate) fn refuse_ssh_save(&mut self, summary: String) {
        if let Some(form) = self.ssh_form.as_mut() {
            form.save_error = Some(summary.clone());
        }
        self.ssh_save_error = Some(summary);
    }

    pub(crate) fn restore_ssh_submission(&mut self, submission: &FormSubmission, summary: String) {
        // A successful presenter submission is terminal, even when domain
        // admission refuses it; rebuilding is required to make Save retryable.
        if let Some(form) = self.ssh_form.as_mut() {
            form.restore_submission(submission);
        }
        self.refuse_ssh_save(summary);
    }

    pub(crate) fn update_ssh_form(&mut self, event: FormDialogEvent) -> Task<Message> {
        if matches!(&event, FormDialogEvent::SubmitRequested)
            && self.refuse_new_credential_mutation()
        {
            return Task::none();
        }
        if self.pending_ssh_host_save.is_some() {
            if matches!(&event, FormDialogEvent::Resolved(_)) {
                self.cancel_ssh_host_save();
                self.ssh_form = None;
                return Task::none();
            }
            if matches!(&event, FormDialogEvent::SubmitRequested) {
                return Task::none();
            }
            let scope = self.ssh_manager_scope();
            return self
                .ssh_form
                .as_mut()
                .map_or_else(Task::none, |form| form.presenter.update(event, scope).1);
        }
        if matches!(&event, FormDialogEvent::SubmitRequested) {
            let Some(form) = self.ssh_form.as_ref() else {
                return Task::none();
            };
            if let Err(summary) = self.ssh_host_reservation(form.editing_id.as_deref()) {
                self.refuse_ssh_save(summary.into());
                return Task::none();
            }
        }
        let auth_changed = matches!(
            &event,
            FormDialogEvent::Edited(id, FieldValue::Choice(_))
                if id.as_str() == SSH_FORM_AUTH_MODE
        );
        let scope = self.ssh_manager_scope();
        let Some(form) = self.ssh_form.as_mut() else {
            return Task::none();
        };
        let (outcome, task) = form.presenter.update(event, scope);

        if auth_changed && outcome.is_none() {
            form.retarget_for_auth_choice();
        }

        match outcome {
            Some(FormOutcome::Submitted(submission)) => {
                let editing_id = form.editing_id.clone();
                self.save_ssh_submission(editing_id, submission)
            }
            Some(FormOutcome::Dismissed(_)) => {
                self.ssh_form = None;
                task
            }
            None => task,
        }
    }

    pub(crate) fn save_ssh_submission(
        &mut self,
        editing_id: Option<String>,
        submission: FormSubmission,
    ) -> Task<Message> {
        let reservation = match self.ssh_host_reservation(editing_id.as_deref()) {
            Ok(reservation) => reservation,
            Err(summary) => {
                self.restore_ssh_submission(&submission, summary.into());
                return Task::none();
            }
        };
        let Some(mut draft) = SshHostForm::from_submission(editing_id, &submission) else {
            self.restore_ssh_submission(
                &submission,
                "The connection form is unavailable. Reopen it and retry.".into(),
            );
            return Task::none();
        };
        let password = Secret::from_string(std::mem::take(&mut draft.password));
        let passphrase = Secret::from_string(std::mem::take(&mut draft.passphrase));
        let Some(record) = draft.to_record(&reservation.id) else {
            self.restore_ssh_submission(
                &submission,
                "The connection is invalid. Correct the form and retry.".into(),
            );
            return Task::none();
        };
        let active_slot = Self::secret_slot_for(&record.auth);
        let changes_credentials = reservation.edit_index.is_some()
            || match active_slot {
                Some(CredentialSlot::Password) => !password.is_empty(),
                Some(CredentialSlot::Passphrase) => !passphrase.is_empty(),
                None => false,
            };
        if changes_credentials && self.operation_origin().is_none() {
            self.restore_ssh_submission(&submission,
                "There is no live credential owner. Open a terminal and retry saving the connection.".into());
            return Task::none();
        }
        let mut snapshot = settings_store::AppPreferences::snapshot(
            &self.settings,
            self.shortcuts.overrides(),
            &self.ssh_hosts,
        );
        snapshot.ssh_host_seq = reservation.sequence;
        match reservation.edit_index {
            Some(index) => snapshot.ssh_hosts[index] = record,
            None => snapshot.ssh_hosts.push(record),
        }
        // Prepare is I/O-free and checks replacement authority and the complete
        // candidate before any host or credential state can change.
        let op = match self.settings_store.prepare_save(&snapshot) {
            Ok(op) => op,
            Err(error) => {
                self.restore_ssh_submission(
                    &submission,
                    crate::i18n::format(
                        crate::i18n::localizer(),
                        "{}. No connection was changed. Retry Save connection.",
                        &[("arg0", &(error.user_summary()))],
                    ),
                );
                return Task::none();
            }
        };
        self.begin_ssh_host_save(reservation, snapshot, submission, op)
    }
}
