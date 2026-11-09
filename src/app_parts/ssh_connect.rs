//! Terminal SSH connect methods.

use super::*;

/// The secret as the password `ssh` is spawned with. A non-UTF-8 read
/// degrades to the in-terminal prompt rather than spawning with a lossy
/// password.
fn secret_password(secret: Option<&Secret>) -> Option<&str> {
    secret.and_then(|secret| std::str::from_utf8(secret.expose_secret()).ok())
}

impl App {
    pub(crate) fn connect_openssh_host(&mut self, host: ssh_config::OpenSshHost) -> Task<Message> {
        let endpoint = pane_endpoint::PaneEndpoint::OpenSsh(host.clone());
        let record = match endpoint.resolve(&self.ssh_hosts, &self.openssh_hosts) {
            Ok(record) => record,
            Err(reason) => {
                self.ssh_credential_notice = Some(CredentialNotice::info(reason));
                return Task::none();
            }
        };
        #[cfg(test)]
        if let Some(mut spawn) = self.ssh_spawn_override.take() {
            self.connect_ssh_record_with_endpoint(record, None, endpoint, |id, host, password| {
                spawn(&id, host, password)
            });
            self.ssh_spawn_override = Some(spawn);
            return Task::none();
        }
        let cap = self.settings.clamped_scrollback() as usize;
        let (pid, ticket) = self.begin_ssh_session(&record, endpoint);
        let worker_pid = pid.clone();
        self.spawn_pane_in_background(
            ticket,
            pid,
            move || {
                session_spawn::PaneSpawnOutcome::Ssh(
                    pty::PtySession::spawn_ssh_alias_with_scrollback(
                        worker_pid,
                        &record,
                        &host.hostname,
                        cap,
                    ),
                )
            },
            |error| session_spawn::PaneSpawnOutcome::Ssh(Err(error)),
        )
    }

    /// Connect `host`, spawning with `secret` as its password.
    ///
    /// The secret stays in its wipe-on-drop custody until the spawn worker
    /// has sealed it into the sshpass `memfd`; no plaintext copy is minted
    /// on the way.
    pub(crate) fn connect_ssh_record(
        &mut self,
        host: SshHostRecord,
        secret: Option<Secret>,
    ) -> Task<Message> {
        // Test seam: a gate installs an override so it can observe the exact
        // `Option<&str>` the reducer hands to the spawn from a real credential
        // drive. Taken out and restored so the closure does not alias `self`.
        #[cfg(test)]
        if let Some(mut spawn) = self.ssh_spawn_override.take() {
            self.connect_ssh_record_with(
                host,
                secret_password(secret.as_ref()),
                |id, host, password| spawn(&id, host, password),
            );
            self.ssh_spawn_override = Some(spawn);
            return Task::none();
        }
        let cap = self.settings.clamped_scrollback() as usize;
        let endpoint = pane_endpoint::PaneEndpoint::profile(&host);
        let (pid, ticket) = self.begin_ssh_session(&host, endpoint);
        let worker_pid = pid.clone();
        self.spawn_pane_in_background(
            ticket,
            pid,
            move || {
                let result = pty::PtySession::spawn_ssh_with_scrollback(
                    worker_pid,
                    &host,
                    secret_password(secret.as_ref()),
                    cap,
                );
                drop(secret);
                session_spawn::PaneSpawnOutcome::Ssh(result)
            },
            |error| session_spawn::PaneSpawnOutcome::Ssh(Err(error)),
        )
    }

    /// SSH connection reducer with only the PTY creation boundary injected,
    /// run inline. Production and tests share all session creation,
    /// activation and render routing after this call.
    #[cfg(test)]
    pub(crate) fn connect_ssh_record_with<F>(
        &mut self,
        host: SshHostRecord,
        password: Option<&str>,
        spawn: F,
    ) where
        F: FnOnce(String, &SshHostRecord, Option<&str>) -> Result<pty::PtySession, pty::PtyError>,
    {
        let endpoint = pane_endpoint::PaneEndpoint::profile(&host);
        self.connect_ssh_record_with_endpoint(host, password, endpoint, spawn);
    }

    #[cfg(test)]
    pub(crate) fn connect_ssh_record_with_endpoint<F>(
        &mut self,
        host: SshHostRecord,
        password: Option<&str>,
        endpoint: pane_endpoint::PaneEndpoint,
        spawn: F,
    ) where
        F: FnOnce(String, &SshHostRecord, Option<&str>) -> Result<pty::PtySession, pty::PtyError>,
    {
        let (pid, ticket) = self.begin_ssh_session(&host, endpoint);
        let result = spawn(pid.clone(), &host, password);
        self.finish_pane_spawn(ticket, &pid, session_spawn::PaneSpawnOutcome::Ssh(result));
    }

    /// Publish and activate a new SSH session whose only pane is waiting for
    /// its connection, and close the manager. Nothing here starts a process.
    ///
    /// Returns the pane id and the ticket its spawn completion must present.
    fn begin_ssh_session(
        &mut self,
        host: &SshHostRecord,
        endpoint: pane_endpoint::PaneEndpoint,
    ) -> (String, u64) {
        let pid = self.fresh_pane_id();
        let title = host.name.clone();
        let dest = if host.username.is_empty() {
            host.destination.clone()
        } else {
            format!("{}@{}", host.username, host.destination)
        };
        let mut session = Session {
            id: format!("ssh-{}", self.next_pane_seq),
            title,
            subtitle: dest.clone(),
            icon: "server".into(),
            status: SessionStatus::Idle,
            kind: SessionKind::Ssh,
            ssh_host_id: Some(host.id.clone()),
            connection_error: None,
            pinned: false,
            detached: false,
            panes: vec![scripted_pane(
                &pid,
                &crate::i18n::format(
                    crate::i18n::localizer(),
                    "Connecting to {dest}...",
                    &[("dest", &(dest))],
                ),
                &format!(
                    "{}:{}",
                    host.destination,
                    if host.path.trim().is_empty() {
                        "~"
                    } else {
                        host.path.trim()
                    }
                ),
            )],
            active_pane: 0,
            pane_grid: PaneLayout::leaf(pid.clone()).pane_grid_state(),
            layout: PaneLayout::leaf(pid.clone()),
            in_app_history: model::CommandHistory::new(),
            group_members: Vec::new(),
        };
        session.panes[0].endpoint = endpoint;
        let ticket = self.expect_pane_spawn(&pid, &session.id);
        let idx_new = self.sessions.len();
        self.sessions.push(session);
        self.activate_session(idx_new);
        self.resize_active_pty();
        self.ssh_manager_open = false;
        self.ssh_form = None;
        (pid, ticket)
    }

    /// An SSH session's spawn result, applied to its waiting pane.
    pub(crate) fn attach_ssh_spawn(
        &mut self,
        index: usize,
        pid: &str,
        result: Result<pty::PtySession, pty::PtyError>,
    ) {
        match result {
            Ok(pty_sess) => self.install_pty(pid.to_owned(), pty_sess),
            Err(error) => {
                self.sessions[index].connection_error =
                    Some(failure_translation::ssh_connection_failure(&error));
            }
        }
    }
}
