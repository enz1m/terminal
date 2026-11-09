//! Terminal SSH form presentation.

use super::*;

// The edit snapshot is metadata-only: it seeds auth retargeting and prevents
// a stale form from authorizing changes to a replacement credential owner.
#[derive(Debug)]
pub(crate) struct SshFormPresentation {
    pub(crate) presenter: FormPresenter,
    pub(crate) editing_id: Option<String>,
    pub(crate) initial_record: Option<SshHostRecord>,
    pub(crate) save_error: Option<String>,
    pub(crate) generation: operation_origin::RequestGeneration,
}

impl SshFormPresentation {
    pub(crate) fn add() -> Self {
        Self {
            presenter: FormPresenter::new(SshHostForm::form_model(None, SshAuthModeKind::Password)),
            editing_id: None,
            initial_record: None,
            save_error: None,
            generation: operation_origin::RequestGeneration::new(),
        }
    }

    pub(crate) fn edit(record: &SshHostRecord) -> Self {
        let auth_kind = record.auth.kind();
        Self {
            presenter: FormPresenter::new(SshHostForm::form_model(Some(record), auth_kind)),
            editing_id: Some(record.id.clone()),
            initial_record: Some(record.clone()),
            save_error: None,
            generation: operation_origin::RequestGeneration::new(),
        }
    }

    pub(crate) fn restore_submission(&mut self, submission: &FormSubmission) {
        let auth_kind = submission
            .value(&SSH_FORM_AUTH_MODE.into())
            .and_then(SshAuthModeKind::from_form_value)
            .unwrap_or(SshAuthModeKind::Password);
        self.presenter = FormPresenter::new(SshHostForm::form_model(
            self.initial_record.as_ref(),
            auth_kind,
        ));
        for (id, value) in submission.iter() {
            let _ = self
                .presenter
                .update::<Message>(FormDialogEvent::Edited(id.clone(), value.clone()), None);
        }
        self.presenter.retarget(SshHostForm::form_model(
            self.initial_record.as_ref(),
            auth_kind,
        ));
    }

    pub(crate) fn set_saving(&mut self) -> bool {
        let session = self.presenter.session();
        let Some(fields) = session
            .model()
            .fields()
            .iter()
            .map(|field| {
                let value = session.value(&field.id)?;
                if field.required && value.is_empty() {
                    return None;
                }
                Some(field.clone().initial(value.clone()).read_only(true))
            })
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        let Ok(model) = enzui_core::form::FormModel::new(fields) else {
            return false;
        };
        self.presenter.retarget(model);
        self.save_error = Some(ssh_host_save::SAVING.into());
        true
    }

    pub(crate) fn auth_kind(&self) -> SshAuthModeKind {
        self.presenter
            .session()
            .value(&SSH_FORM_AUTH_MODE.into())
            .and_then(SshAuthModeKind::from_form_value)
            .unwrap_or(SshAuthModeKind::Password)
    }

    pub(crate) fn retarget_for_auth_choice(&mut self) {
        let auth_kind = self.auth_kind();
        // Seed the retargeted model's INITIALS from the edit record when the
        // presentation has one. `FormSession::retarget` still preserves the
        // live draft for every surviving field (that is the whole point of
        // the API), so a normal path never falls back to these initials.
        // Falling back to record metadata rather than empty strings when a
        // future retarget regression drops a draft keeps the user's
        // name/host/port/etc. visible instead of silently blanking them.
        // The record carries no secret bytes, so this reseeding never places
        // a password or passphrase into a form field.
        self.presenter.retarget(SshHostForm::form_model(
            self.initial_record.as_ref(),
            auth_kind,
        ));
    }

    #[cfg(test)]
    pub(crate) fn text(&self, id: &'static str) -> &str {
        self.presenter
            .session()
            .value(&id.into())
            .and_then(FieldValue::as_text)
            .unwrap_or_default()
    }
}
