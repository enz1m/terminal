//! Terminal save and backend task methods.

use super::*;

impl App {
    // Connecting opens only the bus and cannot prompt to unlock an item.
    // Tests and capture sessions instead enter through their injected stores.
    pub(crate) fn credential_backend_task(capture: bool) -> Task<Message> {
        #[cfg(test)]
        {
            let _ = capture;
            Task::none()
        }
        #[cfg(not(test))]
        {
            if capture {
                return Task::none();
            }
            Task::future(async {
                let backend =
                    enzui_services::credential_store::secret_service::SecretServiceTransport::
                        connect_with_mutation_owner()
                        .await.map_or_else(|_| CredentialBackend::unavailable(), |(owner, store)| {
                            CredentialBackend::with_mutation_owner(owner, store)
                        });
                Message::CredentialBackendReady(backend)
            })
        }
    }

    /// Post (or replace) the history-copy acknowledgement at an explicit
    /// monotonic instant. The reducer supplies `Instant::now`; tests supply a
    /// fixed instant and can therefore prove replacement and expiry without
    /// sleeping. One visible slot plus a zero-entry waiting budget bounds the
    /// lifecycle even if a future producer forgets to coalesce.
    pub(crate) fn post_history_copy_acknowledgement_at(&mut self, now: Instant) {
        self.toasts.push_at(
            ToastSpec::new(crate::i18n::label("Command copied"))
                .tone(ToastTone::Success)
                .coalesce_key(HISTORY_COPY_TOAST_KEY),
            now,
        );
        self.toasts.sync_expiry(&mut self.toast_expiry, now);
    }

    pub(crate) fn save_preferences(&mut self) -> Task<Message> {
        if self.ssh_host_save_uncertain {
            self.ssh_save_error = Some(ssh_host_save::UNCERTAIN.into());
            return Task::none();
        }
        if let Some(pending) = self.pending_ssh_host_save.as_mut() {
            pending.defer_preferences();
            return Task::none();
        }
        if self.startup.is_some() || self.startup_settings_refused {
            self.ssh_save_error =
                Some("Preferences have not been loaded; saving is unavailable.".into());
            return Task::none();
        }
        self.preferences_save_generation
            .retire(enzui_core::cancellation::CancelCause::Cancelled);
        let mut snapshot = settings_store::AppPreferences::snapshot(
            &self.settings,
            self.shortcuts.overrides(),
            &self.ssh_hosts,
        );
        snapshot.ssh_host_seq = self.ssh_host_seq;
        match self.settings_store.prepare_save(&snapshot) {
            Ok(op) => self.run_preferences_save(op),
            Err(error) => {
                self.ssh_save_error = Some(error.user_summary());
                self.shortcuts.finish_save(Err(error));
                Task::none()
            }
        }
    }

    pub(crate) fn run_preferences_save(
        &mut self,
        op: enzui_core::persist::SaveOp,
    ) -> Task<Message> {
        self.preferences_save_generation
            .retire(enzui_core::cancellation::CancelCause::Cancelled);
        self.preferences_save_generation = operation_origin::RequestGeneration::new();
        let generation = self.preferences_save_generation.clone();
        let (work, stamp) = self
            .blocking
            .submit_latest(blocking_dispatch::Latest::Preferences, move |_| {
                op.execute()
            });
        let blocking = self.blocking.clone();
        Task::perform(work, move |settlement| {
            let result = settlement.into_result().unwrap_or_else(|error| {
                let superseded =
                    !blocking.owns_latest(blocking_dispatch::Latest::Preferences, stamp);
                blocking_dispatch::diagnostic(
                    "preferences",
                    blocking_dispatch::terminal_name(error),
                    blocking.live(),
                );
                Err(persistence_runtime::dispatcher_failure(error, superseded))
            });
            Message::PreferencesWorkerFinished(generation, result)
        })
    }
}
