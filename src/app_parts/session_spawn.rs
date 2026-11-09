//! Terminal pane spawn methods: new local tabs, and the worker every PTY spawn
//! runs on.
//!
//! Starting a shell or `ssh` opens a PTY, forks the child and then waits up to
//! the PTY capability handshake budget (500 ms) for the child to prove it
//! inherited its capability FIFO, and resolving a local working directory
//! stats the filesystem. None of that may run on the reducer thread: the pane
//! that will hold the PTY (a new tab, including the first one at startup, a
//! split, an SSH session or a reconnecting endpoint pane) is published at once
//! with its placeholder text, and the spawn runs on a short-lived worker whose
//! result is attached by [`Message::PaneSpawned`].

use super::*;

/// The injected spawn: pane id and requested directory to a live PTY.
pub(crate) type LocalSpawnResult =
    Result<(pty::PtySession, pty::LocalCwdResolution), pty::PtyError>;

/// What a local shell spawn hands back to the reducer.
#[derive(Debug)]
pub(crate) struct LocalSpawnOutcome {
    /// The resolved directory's label, shown by a pane whose spawn failed.
    pub(crate) fallback_cwd: String,
    pub(crate) result: LocalSpawnResult,
}

impl LocalSpawnOutcome {
    /// A spawn that never ran, for a worker that could not start or stopped.
    fn failed(fallback_cwd: String, error: pty::PtyError) -> Self {
        Self {
            fallback_cwd,
            result: Err(error),
        }
    }
}

/// A finished background spawn, by the kind of pane waiting for it.
#[derive(Debug)]
pub(crate) enum PaneSpawnOutcome {
    /// The only pane of a new local tab.
    Tab(LocalSpawnOutcome),
    /// A new local split pane.
    Split(LocalSpawnOutcome),
    /// The only pane of a new SSH session.
    Ssh(Result<pty::PtySession, pty::PtyError>),
    /// A pane (re)starting its saved endpoint, with the cwd label it adopts.
    Endpoint(Result<(pty::PtySession, String), pty::PtyError>),
}

impl PaneSpawnOutcome {
    fn into_pty(self) -> Option<pty::PtySession> {
        match self {
            Self::Tab(outcome) | Self::Split(outcome) => outcome.result.ok().map(|(pty, _)| pty),
            Self::Ssh(result) => result.ok(),
            Self::Endpoint(result) => result.ok().map(|(pty, _)| pty),
        }
    }
}

/// Resolve the directory and run `spawn`. This is the whole blocking part of
/// opening a local pane, so production runs it on the spawn worker.
pub(crate) fn run_local_spawn<F>(
    pane: String,
    explicit: bool,
    requested_cwd: Option<String>,
    spawn: F,
) -> LocalSpawnOutcome
where
    F: FnOnce(String, Option<&str>) -> LocalSpawnResult,
{
    let resolved = pty::resolve_local_cwd(requested_cwd.as_deref());
    let explicit_refused = explicit && (resolved.warning.is_some() || resolved.requested.is_none());
    let result = if explicit_refused {
        Err(pty::PtyError::Spawn(
            "Requested working directory is no longer available.".into(),
        ))
    } else {
        spawn(pane, requested_cwd.as_deref())
    };
    LocalSpawnOutcome {
        fallback_cwd: shell_history::cwd::path_label(&resolved.effective),
        result,
    }
}

/// The label a local pane shows until its spawn has resolved the directory.
/// Nothing here touches the filesystem.
pub(crate) fn placeholder_cwd(requested: Option<&str>) -> String {
    requested.map_or_else(
        || {
            shell_history::cwd::path_label(
                &std::env::var_os("HOME").map_or_else(|| "/".into(), std::path::PathBuf::from),
            )
        },
        str::to_owned,
    )
}

impl App {
    pub(crate) fn new_local_session(&mut self) -> Task<Message> {
        let cap = self.settings.clamped_scrollback() as usize;
        let explicit = self.launch_directory.is_some();
        self.new_local_session_in_background(move |id, cwd| {
            if explicit {
                pty::PtySession::spawn_explicit_directory(id, cwd, cap)
            } else {
                pty::PtySession::spawn_shell_at_with_scrollback(id, cwd, cap)
            }
        })
    }

    /// `new_local_session` with the shell spawn injected, run off the reducer
    /// thread.
    ///
    /// The session is published, activated and rendered before the spawn
    /// starts; the returned task delivers the PTY (or the reason there is
    /// none) to [`Self::finish_pane_spawn`].
    pub(crate) fn new_local_session_in_background<F>(&mut self, spawn: F) -> Task<Message>
    where
        F: FnOnce(String, Option<&str>) -> LocalSpawnResult + Send + 'static,
    {
        let (pid, ticket, explicit, requested_cwd) = self.begin_local_session();
        let fallback = placeholder_cwd(requested_cwd.as_deref());
        let worker_pid = pid.clone();
        self.spawn_pane_in_background(
            ticket,
            pid,
            move || {
                PaneSpawnOutcome::Tab(run_local_spawn(worker_pid, explicit, requested_cwd, spawn))
            },
            move |error| PaneSpawnOutcome::Tab(LocalSpawnOutcome::failed(fallback.clone(), error)),
        )
    }

    /// `new_local_session` with the shell spawn injected and run inline.
    ///
    /// A spawn can fail TRANSIENTLY (EAGAIN starting the PTY reader thread on a
    /// loaded host), and the old code's only answer was a panic inside `pty.rs`.
    /// Now that it is an error value the app has to SAY so, and the only honest
    /// way to gate what the user sees when the shell will not start is to hand
    /// this function a spawn that fails. It shares every step but the worker
    /// with [`Self::new_local_session_in_background`], which production uses.
    pub(crate) fn new_local_session_with<F>(&mut self, spawn: F)
    where
        F: FnOnce(String, Option<&str>) -> LocalSpawnResult,
    {
        let (pid, ticket, explicit, requested_cwd) = self.begin_local_session();
        let outcome = run_local_spawn(pid.clone(), explicit, requested_cwd, spawn);
        self.finish_pane_spawn(ticket, &pid, PaneSpawnOutcome::Tab(outcome));
    }

    /// Publish and activate a new local session whose only pane is waiting
    /// for its shell. Nothing here touches the filesystem or a process.
    ///
    /// Returns the pane id, the ticket its spawn completion must present, and
    /// the directory request the spawn resolves.
    fn begin_local_session(&mut self) -> (String, u64, bool, Option<String>) {
        let explicit = self.launch_directory.take();
        let requested_cwd = explicit.clone().or_else(|| self.active_local_cwd());
        // The resolved label replaces this once the spawn has stat'ed it.
        let initial_cwd = placeholder_cwd(requested_cwd.as_deref());
        let pid = self.fresh_pane_id();
        let idx = self.sessions.len();
        let title = format!("session-{idx}");
        let session = Session {
            id: format!("local-new-{}", self.next_pane_seq),
            title,
            subtitle: "Local shell".into(),
            icon: "terminal".into(),
            status: SessionStatus::Idle,
            kind: SessionKind::Local,
            ssh_host_id: None,
            connection_error: None,
            pinned: false,
            detached: false,
            panes: vec![scripted_pane(&pid, "Starting shell…", &initial_cwd)],
            active_pane: 0,
            pane_grid: PaneLayout::leaf(pid.clone()).pane_grid_state(),
            layout: PaneLayout::leaf(pid.clone()),
            in_app_history: model::CommandHistory::new(),
            group_members: Vec::new(),
        };
        let ticket = self.expect_pane_spawn(&pid, &session.id);
        self.sessions.push(session);
        self.activate_session(idx);
        self.resize_active_pty();
        (pid, ticket, explicit.is_some(), requested_cwd)
    }

    /// Record that pane `pid` of `session` waits for a spawn, and return the
    /// ticket its completion must present.
    pub(crate) fn expect_pane_spawn(&mut self, pid: &str, session: &str) -> u64 {
        self.next_pane_spawn_ticket += 1;
        let ticket = self.next_pane_spawn_ticket;
        self.pending_pane_spawns
            .insert(pid.to_owned(), (ticket, session.to_owned()));
        ticket
    }

    /// Whether pane `pid` is waiting for a background spawn, so a second
    /// start must not run beside it.
    pub(crate) fn pane_spawn_pending(&self, pid: &str) -> bool {
        self.pending_pane_spawns.contains_key(pid)
    }

    /// Run `work` on a spawn worker and deliver its outcome for `ticket`.
    ///
    /// `lost` builds the outcome for a spawn that never ran: the worker thread
    /// could not be started (EAGAIN under memory pressure), which settles at
    /// once, or it stopped without answering. Either way the pane says why, as
    /// a failed spawn would.
    pub(crate) fn spawn_pane_in_background<W, L>(
        &mut self,
        ticket: u64,
        pid: String,
        work: W,
        lost: L,
    ) -> Task<Message>
    where
        W: FnOnce() -> PaneSpawnOutcome + Send + 'static,
        L: Fn(pty::PtyError) -> PaneSpawnOutcome + Send + 'static,
    {
        let (sender, receiver) = iced::futures::channel::oneshot::channel();
        // A dropped receiver (the app is gone) drops the session here, which
        // hands its exact custody to the process teardown executor.
        let started = std::thread::Builder::new()
            .name("terminal-pty-spawn".into())
            .spawn(move || {
                let _ = sender.send(work());
            });
        if let Err(error) = started {
            self.finish_pane_spawn(ticket, &pid, lost(pty::PtyError::Spawn(error.to_string())));
            return Task::none();
        }
        Task::perform(receiver, move |delivered| {
            let outcome = delivered.unwrap_or_else(|_| {
                lost(pty::PtyError::Spawn(
                    "The terminal start-up worker stopped.".into(),
                ))
            });
            Message::PaneSpawned(ticket, pid, enzui_parent_lease::Handoff::new(outcome))
        })
    }

    /// Attach a finished spawn to the pane that is waiting for it.
    ///
    /// A pane that was closed, or replaced by a workspace restore, while its
    /// child was starting no longer owns the result: the PTY leaves by the
    /// owner route like any other retired shell, so the child is not orphaned.
    pub(crate) fn finish_pane_spawn(&mut self, ticket: u64, pid: &str, outcome: PaneSpawnOutcome) {
        let owner = match self.pending_pane_spawns.get(pid) {
            Some((pending, session)) if *pending == ticket => Some(session.clone()),
            _ => None,
        };
        let current = owner.is_some();
        if current {
            self.pending_pane_spawns.remove(pid);
        }
        let target = self
            .sessions
            .iter()
            .position(|session| session.panes.iter().any(|pane| pane.id == pid))
            .filter(|_| current && !self.ptys.contains_key(pid));
        let Some(index) = target else {
            if let Some(pty) = outcome.into_pty() {
                self.retire_owned_pty(owner.as_deref().unwrap_or_default(), pid, pty);
            }
            return;
        };
        match outcome {
            PaneSpawnOutcome::Tab(outcome) => self.attach_local_spawn(index, pid, outcome, true),
            PaneSpawnOutcome::Split(outcome) => self.attach_local_spawn(index, pid, outcome, false),
            PaneSpawnOutcome::Ssh(result) => self.attach_ssh_spawn(index, pid, result),
            PaneSpawnOutcome::Endpoint(result) => {
                self.install_endpoint_spawn(pid, result);
                return;
            }
        }
        self.resize_session_ptys(index);
        self.invalidate_sidebar();
    }

    /// A local tab's or split's spawn result, applied to its waiting pane.
    /// A new tab drops its "Starting shell…" placeholder; a split pane never
    /// did, and still does not.
    fn attach_local_spawn(
        &mut self,
        index: usize,
        pid: &str,
        outcome: LocalSpawnOutcome,
        clear_placeholder: bool,
    ) {
        let session = &mut self.sessions[index];
        let Some(slot) = session.panes.iter().position(|pane| pane.id == pid) else {
            return;
        };
        match outcome.result {
            Ok((pty, cwd)) => {
                let pane = &mut session.panes[slot];
                pane.cwd = shell_history::cwd::path_label(&cwd.effective);
                self.git_lookups_due.push((pid.to_owned(), cwd.effective));
                if clear_placeholder {
                    pane.transcript.clear();
                }
                self.install_pty(pid.to_owned(), pty);
            }
            Err(e) => {
                session.connection_error = Some(failure_translation::shell_spawn_failure(&e));
                session.status = SessionStatus::Error;
                // The sidebar row shows the reason, but the CONTENT pane
                // is what the user is looking at, and a pane still saying
                // "Starting shell…" forever is the silent failure. Replace the
                // optimistic transcript with the error.
                let pane = &mut session.panes[slot];
                *pane = failed_pane(pid, session.connection_error.as_deref().unwrap_or(""));
                pane.cwd = outcome.fallback_cwd;
            }
        }
    }
}
