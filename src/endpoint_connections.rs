use crate::model::SessionStatus;
use crate::operation_origin::OperationOrigin;
use crate::pane_endpoint::PaneEndpoint;
use crate::pane_layout::SplitDirection;
use crate::ssh_credentials::SshCredentialResult;
use crate::{App, Message, pty, scripted_pane};
use iced::Task;

pub(crate) struct PendingPaneConnection {
    pub(crate) origin: OperationOrigin,
    endpoint: PaneEndpoint,
    cwd: String,
}

/// An endpoint spawn's PTY and the cwd label its pane adopts.
type SpawnedEndpoint = Result<(pty::PtySession, String), pty::PtyError>;

#[cfg(test)]
pub(crate) type EndpointSpawnOverride = Box<
    dyn FnMut(&str, &PaneEndpoint, &str, Option<&str>) -> Result<pty::PtySession, pty::PtyError>,
>;

impl App {
    /// Start pane `id`'s endpoint, spawning with `secret` as its password.
    ///
    /// The host is resolved here, from in-memory records; the PTY spawn and
    /// its capability handshake run on the spawn worker, and the pane keeps
    /// its current text until [`Self::install_endpoint_spawn`] attaches the
    /// result. The secret stays in its wipe-on-drop custody until the worker
    /// has sealed it into the sshpass `memfd`.
    pub(crate) fn start_endpoint_spawn(
        &mut self,
        id: &str,
        endpoint: &PaneEndpoint,
        cwd: &str,
        secret: Option<crate::Secret>,
    ) -> Task<Message> {
        #[cfg(test)]
        if let Some(spawn) = self.endpoint_spawn_override.as_mut() {
            let password = secret
                .as_ref()
                .and_then(|secret| std::str::from_utf8(secret.expose_secret()).ok());
            let result = spawn(id, endpoint, cwd, password).map(|pty| (pty, cwd.to_owned()));
            self.install_endpoint_spawn(id, result);
            return Task::none();
        }
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.panes.iter().any(|pane| pane.id == id))
            .map(|session| session.id.clone())
        else {
            return Task::none();
        };
        let cap = self.settings.clamped_scrollback() as usize;
        let pid = id.to_owned();
        let cwd = cwd.to_owned();
        let work: Box<dyn FnOnce() -> SpawnedEndpoint + Send> = match endpoint {
            PaneEndpoint::Local => Box::new(move || {
                pty::PtySession::spawn_shell_at_with_scrollback(pid, Some(&cwd), cap)
                    .map(|(pty, cwd)| (pty, crate::shell_history::cwd::path_label(&cwd.effective)))
            }),
            _ => {
                let mut host = match endpoint.resolve(&self.ssh_hosts, &self.openssh_hosts) {
                    Ok(host) => host,
                    Err(reason) => {
                        self.install_endpoint_spawn(id, Err(pty::PtyError::Other(reason.into())));
                        return Task::none();
                    }
                };
                host.path = endpoint.remote_directory(&cwd, &host.path);
                match endpoint {
                    PaneEndpoint::OpenSsh(alias) => {
                        let hostname = alias.hostname.clone();
                        Box::new(move || {
                            pty::PtySession::spawn_ssh_alias_with_scrollback(
                                pid, &host, &hostname, cap,
                            )
                            .map(|pty| (pty, cwd))
                        })
                    }
                    _ => Box::new(move || {
                        let password = secret
                            .as_ref()
                            .and_then(|secret| std::str::from_utf8(secret.expose_secret()).ok());
                        let spawned =
                            pty::PtySession::spawn_ssh_with_scrollback(pid, &host, password, cap);
                        drop(secret);
                        spawned.map(|pty| (pty, cwd))
                    }),
                }
            }
        };
        let ticket = self.expect_pane_spawn(id, &session);
        self.spawn_pane_in_background(
            ticket,
            id.to_owned(),
            move || crate::session_spawn::PaneSpawnOutcome::Endpoint(work()),
            |error| crate::session_spawn::PaneSpawnOutcome::Endpoint(Err(error)),
        )
    }

    pub(crate) fn pane_endpoint_error(&mut self, id: &str, reason: &str) {
        for session in &mut self.sessions {
            if let Some(pane) = session.panes.iter_mut().find(|pane| pane.id == id) {
                // Keep the endpoint and cwd even when launch fails; a failure is not a Local pane.
                pane.status = SessionStatus::Error;
                if let Some(output) = self.restored_outputs.get(id) {
                    pane.transcript = output.render(id);
                    pane.transcript
                        .extend(crate::failed_pane(id, reason).transcript);
                } else {
                    pane.transcript = crate::failed_pane(id, reason).transcript;
                }
                session.connection_error = Some(reason.to_owned());
                session.status = SessionStatus::Error;
                break;
            }
        }
        self.invalidate_sidebar();
    }

    pub(crate) fn install_endpoint_spawn(
        &mut self,
        id: &str,
        result: Result<(pty::PtySession, String), pty::PtyError>,
    ) {
        match result {
            Ok((pty, cwd)) => {
                self.install_pty(id.to_owned(), pty);
                if let Some(pane) = self
                    .sessions
                    .iter_mut()
                    .flat_map(|session| &mut session.panes)
                    .find(|pane| pane.id == id)
                {
                    pane.cwd = cwd;
                    pane.status = SessionStatus::Idle;
                    pane.transcript.clear();
                }
                let target = self
                    .sessions
                    .iter()
                    .enumerate()
                    .find(|(_, session)| session.panes.iter().any(|pane| pane.id == id))
                    .map(|(index, session)| (index, session.id.clone()));
                if let Some((index, owner)) = target {
                    if let Some(window) = self.detached_window_of(&owner) {
                        if let Some(size) =
                            self.detached_windows.get(&window).map(|window| window.size)
                        {
                            self.resize_detached_pty(&owner, size);
                        }
                    } else if index == self.active {
                        self.resize_active_pty();
                    }
                }
            }
            Err(error) => self.pane_endpoint_error(
                id,
                &crate::failure_translation::ssh_connection_failure(&error),
            ),
        }
    }

    pub(crate) fn start_pane_endpoint(&mut self, session_id: &str, pane_id: &str) -> Task<Message> {
        if self.openssh_discovery.enabled && !self.ptys.contains_key(pane_id) {
            let endpoint = self
                .sessions
                .iter()
                .find(|s| s.id == session_id)
                .and_then(|s| s.panes.iter().find(|p| p.id == pane_id))
                .map(|p| p.endpoint.clone());
            if let Some(endpoint @ PaneEndpoint::OpenSsh(_)) = endpoint
                && let Some(origin) = self.operation_origin_for(session_id, pane_id)
            {
                return self.begin_openssh_reload(crate::ssh_discovery_app::Intent::Reconnect {
                    origin,
                    endpoint,
                });
            }
        }
        self.start_pane_endpoint_current(session_id, pane_id)
    }

    pub(crate) fn start_pane_endpoint_current(
        &mut self,
        session_id: &str,
        pane_id: &str,
    ) -> Task<Message> {
        let Some(pane) = self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| session.panes.iter().find(|pane| pane.id == pane_id))
        else {
            return Task::none();
        };
        // An unresolved teardown still owns this pane's previous child, so a
        // replacement must not start beside it.
        if self.ptys.contains_key(pane_id)
            || self.pane_has_teardown_debt(pane_id)
            || self.pane_spawn_pending(pane_id)
        {
            return Task::none();
        }
        let endpoint = pane.endpoint.clone();
        let cwd = pane.cwd.clone();
        let Some(origin) = self.operation_origin_for(session_id, pane_id) else {
            return Task::none();
        };
        let host = match &endpoint {
            PaneEndpoint::Local => None,
            _ => match endpoint.resolve(&self.ssh_hosts, &self.openssh_hosts) {
                Ok(host) => Some(host),
                Err(reason) => {
                    self.pane_endpoint_error(pane_id, reason);
                    return Task::none();
                }
            },
        };
        if let (PaneEndpoint::Profile(_), Some(host)) = (&endpoint, &host)
            && let Some(slot) = Self::secret_slot_for(&host.auth)
        {
            let retired: Vec<_> = self
                .pending_pane_connections
                .iter()
                .filter(|pending| pending.origin.pane == pane_id)
                .map(|pending| pending.origin.generation.clone())
                .collect();
            self.credential_ops
                .retain(|op| !retired.contains(&op.origin.generation));
            self.pending_pane_connections
                .retain(|pending| pending.origin.pane != pane_id);
            let token = self.next_ssh_credential_token();
            let (origin, future) = self.load_secret_op_for(origin, host.id.clone(), slot, token);
            self.pending_pane_connections.push(PendingPaneConnection {
                origin,
                endpoint,
                cwd,
            });
            return self.spawn_credential(future);
        }
        self.start_endpoint_spawn(pane_id, &endpoint, &cwd, None)
    }

    pub(crate) fn pane_connection_is_current(&self, origin: &OperationOrigin) -> bool {
        self.pending_pane_connections
            .iter()
            .find(|pending| pending.origin == *origin)
            .is_none_or(|pending| {
                self.sessions
                    .iter()
                    .find(|session| session.id == origin.session)
                    .and_then(|session| session.panes.iter().find(|pane| pane.id == origin.pane))
                    .is_some_and(|pane| pane.endpoint == pending.endpoint)
                    && pending
                        .endpoint
                        .resolve(&self.ssh_hosts, &self.openssh_hosts)
                        .is_ok()
            })
    }

    pub(crate) fn resume_pending_pane_connection(
        &mut self,
        origin: &OperationOrigin,
        result: SshCredentialResult,
    ) -> Option<Task<Message>> {
        let index = self
            .pending_pane_connections
            .iter()
            .position(|pending| pending.origin == *origin)?;
        let pending = self.pending_pane_connections.remove(index);
        if matches!(result, SshCredentialResult::Cancelled) {
            self.pane_endpoint_error(
                &origin.pane,
                "SSH connection cancelled. Open Remote to reconnect explicitly.",
            );
            return Some(Task::none());
        }
        let secret = match result {
            SshCredentialResult::Retrieved(handle) => handle.take(),
            _ => None,
        };
        Some(self.start_endpoint_spawn(&origin.pane, &pending.endpoint, &pending.cwd, secret))
    }

    pub(crate) fn reconcile_endpoint_connections(&mut self) {
        let retired: Vec<_> = self.pending_pane_connections.iter().filter(|pending| {
            !pending.origin.generation.is_live() || !self.pane_connection_is_current(&pending.origin)
        }).map(|pending| (pending.origin.generation.clone(), pending.origin.pane.clone(),
            pending.endpoint.resolve(&self.ssh_hosts, &self.openssh_hosts).err()
                .unwrap_or("SSH connection cancelled or superseded. Open Remote to reconnect explicitly.")))
            .collect();
        for (generation, id, reason) in retired {
            self.credential_ops
                .retain(|op| op.origin.generation != generation);
            self.pending_pane_connections
                .retain(|pending| pending.origin.generation != generation);
            self.pane_endpoint_error(&id, reason);
        }
        self.pending_endpoint_restores.retain(|id| {
            self.sessions
                .iter()
                .any(|session| session.panes.iter().any(|pane| &pane.id == id))
        });
    }

    pub(crate) fn resume_endpoint_restores(&mut self) -> Task<Message> {
        if !self.main_window.is_some_and(|id| self.windows.is_open(id)) {
            return Task::none();
        }
        let mut tasks = Vec::new();
        for session in std::mem::take(&mut self.pending_detached_restores) {
            if self
                .sessions
                .iter()
                .any(|item| item.id == session && item.detached)
                && self.detached_window_of(&session).is_none()
            {
                tasks.push(self.open_detached_endpoint_window(session));
            }
        }
        if self.endpoint_credentials_ready {
            for id in std::mem::take(&mut self.pending_endpoint_restores) {
                let session = self
                    .sessions
                    .iter()
                    .find(|session| session.panes.iter().any(|pane| pane.id == id))
                    .map(|session| (session.id.clone(), session.detached));
                if let Some((session, detached)) = session {
                    if detached && self.detached_window_of(&session).is_none() {
                        self.pending_endpoint_restores.push(id);
                    } else {
                        tasks.push(self.start_pane_endpoint(&session, &id));
                    }
                }
            }
        }
        Task::batch(tasks)
    }

    pub(crate) fn split_endpoint_pane(&mut self, direction: SplitDirection) -> Task<Message> {
        let Some(session) = self.sessions.get(self.active) else {
            return Task::none();
        };
        if session.panes.len() >= crate::pane_layout::MAX_PANES {
            self.pane_feedback = Some(crate::i18n::format(
                crate::i18n::localizer(),
                "Pane limit reached: at most {} terminal surfaces can be rendered.",
                &[("arg0", &(crate::pane_layout::MAX_PANES))],
            ));
            return Task::none();
        }
        let Some(source) = session.panes.get(session.active_pane) else {
            return Task::none();
        };
        let (source_id, endpoint, cwd) = (
            source.id.clone(),
            source.endpoint.clone(),
            source.cwd.clone(),
        );
        let new_id = self.fresh_pane_id();
        let session = &mut self.sessions[self.active];
        if !session.layout.split_leaf(&source_id, direction, &new_id) {
            return Task::none();
        }
        let mut pane = scripted_pane(&new_id, "Connecting to the same SSH endpoint…", &cwd);
        pane.endpoint = endpoint;
        session.panes.push(pane);
        session.active_pane = session.panes.len() - 1;
        for member in &mut session.group_members {
            member.split_owned_pane(&source_id, direction, &new_id);
        }
        session
            .pane_grid
            .replace_tree(session.layout.pane_grid_tree());
        let session_id = session.id.clone();
        self.pane_feedback = None;
        self.clear_selection_on_reflow();
        let task = self.start_pane_endpoint(&session_id, &new_id);
        self.resize_active_pty();
        task
    }
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
