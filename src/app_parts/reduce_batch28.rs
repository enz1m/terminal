//! Terminal reducer batch 28: credential and SSH-form arms.

use super::*;

impl App {
    pub(crate) fn reduce28_credential_backend_ready_0(
        &mut self,
        backend: CredentialBackend,
    ) -> Task<Message> {
        // A capture fixture pins the backend: the boot connect races
        // the fixture message, and letting a late real (or
        // `Unavailable`) store land would silently repaint the
        // scripted state as "no keyring" — which is exactly the shot
        // the fixture exists to distinguish it from.
        if !self.credential_backend_pinned {
            self.credentials = backend.store();
            self.credential_mutation_owner = backend.mutation_owner();
            self.credential_mutation_store = backend.mutation_store();
            if backend.is_unavailable() {
                self.ssh_credential_notice = Some(CredentialNotice::info(
                    "No keyring service is available. Remembered SSH credentials are unavailable; nothing will be written to disk.",
                ));
            }
        }
        self.endpoint_credentials_ready = true;
        self.secure_pins.backend_ready();
        self.resume_endpoint_restores()
    }
    pub(crate) fn reduce28_install_capture_keyring_1(
        &mut self,
        fixture: capture_keyring::CaptureKeyring,
    ) -> Task<Message> {
        // Everything after the store swap is the ORDINARY path: one
        // saved profile, the manager open, and a real remember
        // operation whose typed outcome paints the notice. A shot of
        // "locked" is the reducer's own words, not a caption.
        self.credential_mutation_owner = None;
        self.credential_mutation_store = None;
        self.credential_mutations.clear();
        self.credential_ops.retain(|op| !op.owner_mutation);
        self.credentials = fixture.store();
        self.credential_backend_pinned = true;
        self.ssh_hosts = vec![SshHostRecord {
            id: capture_keyring::FIXTURE_PROFILE_ID.into(),
            name: capture_keyring::FIXTURE_PROFILE_NAME.into(),
            destination: capture_keyring::FIXTURE_DESTINATION.into(),
            port: 22,
            username: "deploy".into(),
            path: String::new(),
            auth: SshAuthMode::Password,
            notes: String::new(),
        }];
        self.ssh_manager_open = true;
        self.ssh_form = None;
        self.remember_secret_task(
            capture_keyring::FIXTURE_PROFILE_ID.into(),
            CredentialSlot::Password,
            capture_keyring::FIXTURE_SECRET.into(),
        )
    }
    pub(crate) fn reduce28_open_ssh_add_form_2(&mut self) -> Task<Message> {
        self.ssh_form = Some(SshFormPresentation::add());

        Task::none()
    }
    pub(crate) fn reduce28_open_ssh_edit_form_3(&mut self, idx: usize) -> Task<Message> {
        if let Some(host) = self.ssh_hosts.get(idx) {
            self.ssh_form = Some(SshFormPresentation::edit(host));
        }

        Task::none()
    }
    #[cfg(test)]
    pub(crate) fn reduce28_close_ssh_form_4(&mut self) -> Task<Message> {
        self.update(Message::SshFormEvent(FormDialogEvent::Resolved(
            enzui_core::modal::DismissCause::CancelAction,
        )))
    }
}
