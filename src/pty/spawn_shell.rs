//! PtySession shell spawning.

use super::*;

impl PtySession {
    /// Spawn a login shell in the safe default cwd.
    #[cfg(test)]
    pub(crate) fn spawn_shell(session_id: impl Into<String>) -> Result<Self, PtyError> {
        Self::spawn_shell_at(session_id, None).map(|(s, _)| s)
    }

    /// Spawn a login shell whose reader notifies a caller-owned wake bus.
    ///
    /// Production deliberately coalesces every pane onto [`wake_bus`]. Tests
    /// that consume a bell destructively need a private bus, or another test's
    /// PTY can supply or steal their signal while the binary runs in parallel.
    #[cfg(test)]
    pub(crate) fn spawn_shell_with_wake_bus(
        session_id: impl Into<String>,
        wake: &'static WakeBus,
    ) -> Result<Self, PtyError> {
        Self::spawn_shell_at_with_wake_bus(session_id, None, wake).map(|(s, _)| s)
    }

    /// Spawn the deterministic child used by the app's exit-reporting test.
    ///
    /// Unlike [`PtySession::spawn_shell_with_wake_bus`], this does not read the
    /// process-global `$SHELL`, `$HOME`, or the user's startup files. The child
    /// disables tty echo, emits [`EXIT_PROBE_READY`], blocks in the shell
    /// builtin `read` until the test writes one line, then exits. After the
    /// handshake its private wake bus can therefore ring for EOF rather than
    /// for echoed input, an unrelated prompt, or another parallel test's output.
    #[cfg(test)]
    pub(crate) fn spawn_exit_probe_with_wake_bus(
        session_id: impl Into<String>,
        wake: &'static WakeBus,
    ) -> Result<Self, PtyError> {
        Self::spawn_exit_probe_with_code(session_id, wake, 23)
    }

    #[cfg(test)]
    pub(crate) fn spawn_exit_probe_with_code(
        session_id: impl Into<String>,
        wake: &'static WakeBus,
        code: u8,
    ) -> Result<Self, PtyError> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| PtyError::OpenPty(err.to_string()))?;
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(format!(
            "stty -echo; printf '{EXIT_PROBE_READY}\\n'; IFS= read -r line; exit {code}"
        ));
        cmd.env("TERM", "xterm-256color");
        Self::finish_spawn_with_wake_bus(session_id, pair, cmd, wake, default_reader_spawner)
    }

    /// Spawn a child that prints `marker` once and then sleeps, holding its
    /// PTY open. Used to prove the PRODUCTION reader projects real output from
    /// the exact master it was given.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn spawn_output_probe_with_wake_bus(
        session_id: impl Into<String>,
        marker: &str,
        wake: &'static WakeBus,
    ) -> Result<Self, PtyError> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| PtyError::OpenPty(err.to_string()))?;
        // The marker stays in the sleeping child's argv so the RAII cleanup can
        // find it even after the probe has exec'd.
        let mut cmd = CommandBuilder::new("/bin/bash");
        cmd.arg("-c");
        cmd.arg("printf '%s\\n' \"$1\"; exec -a \"$1-output\" /bin/sleep 20");
        cmd.arg("output-probe");
        cmd.arg(marker);
        cmd.env("TERM", "xterm-256color");
        Self::finish_spawn_with_wake_bus(session_id, pair, cmd, wake, default_reader_spawner)
    }

    /// Spawn an interactive job-control session whose leader and background
    /// jobs ignore HUP and TERM. One job has a different process group in the
    /// original session; another calls `setsid()`. Both retain PTY descriptors.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn spawn_shutdown_probe(
        session_id: impl Into<String>,
        marker: &str,
    ) -> Result<Self, PtyError> {
        let (pair, cmd) = real_hup_ignoring_pair_and_cmd(marker, true)
            .ok_or_else(|| PtyError::OpenPty("openpty unavailable".into()))?;
        Self::finish_spawn(session_id, pair, cmd)
    }

    /// Spawn a real child while deterministically denying the FIFO handshake.
    /// The resulting session must retain no capability and fall back to the
    /// pinned leader/session ancestry ownership path.
    #[cfg(all(test, target_os = "linux"))]
    pub(in crate::pty) fn spawn_failed_capability_handshake_probe(
        session_id: impl Into<String>,
        marker: &str,
        capability_path: PathBuf,
        path_was_visible: Arc<AtomicBool>,
        fixture: &capability::tests::RuntimeFixture,
    ) -> Result<Self, PtyError> {
        let (pair, mut cmd) = real_hup_ignoring_pair_and_cmd(marker, false)
            .ok_or_else(|| PtyError::OpenPty("openpty unavailable".into()))?;
        fixture.configure(&mut cmd);
        Self::finish_spawn_with_wake_bus_controlled(
            session_id,
            pair,
            cmd,
            wake_bus(),
            default_reader_spawner,
            None,
            PtySpawnTestControl {
                capability_path: Some(capability_path),
                path_was_visible: Some(path_was_visible),
                fail_capability_handshake: true,
                ..PtySpawnTestControl::default()
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn worker_witnesses(&self) -> WorkerWitnesses {
        WorkerWitnesses {
            reader_done: Arc::clone(&self.reader_done),
            exit_done: Arc::clone(&self.exit_done),
            reader_joined: Arc::clone(&self.reader_joined),
            exit_joined: Arc::clone(&self.exit_joined),
            writer_joined: self
                .input
                .as_ref()
                .map(|input| Arc::clone(&input.joined))
                .unwrap_or_default(),
            final_freeze_scans: Arc::clone(&self.final_freeze_scans),
            final_freeze_stable: Arc::clone(&self.final_freeze_stable),
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn process_id(&self) -> Option<u32> {
        Some(self.process_identity.pid)
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn owned_process_ids(&self) -> Vec<u32> {
        let mut owned = Vec::new();
        let _ = self.process_identity.refresh_owned_processes(&mut owned);
        owned.into_iter().map(|member| member.pid).collect()
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn tty_name_for_tests(&self) -> Option<PathBuf> {
        self.process_identity.tty_name.clone()
    }

    /// Does `pid` hold this session's inherited capability descriptor?
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn capability_held_by(&self, pid: u32) -> bool {
        self.process_identity
            .capability
            .as_ref()
            .is_some_and(|capability| capability.is_held_by(pid))
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn has_capability(&self) -> bool {
        self.process_identity.capability.is_some()
    }

    /// Spawn a deterministic child that reports the numeric value of exactly
    /// one byte read from its PTY. This is the rendered-key routing control for
    /// Unlike a login shell, it makes Escape observable without relying
    /// on readline state, prompt text, or tty echo.
    #[cfg(test)]
    pub(crate) fn spawn_input_probe_with_wake_bus(
        session_id: impl Into<String>,
        wake: &'static WakeBus,
    ) -> Result<Self, PtyError> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| PtyError::OpenPty(err.to_string()))?;
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(format!(
            "stty -echo -icanon min 1 time 0; printf '{INPUT_PROBE_READY}\\n'; \
             byte=$(dd bs=1 count=1 2>/dev/null | od -An -tu1); \
             printf 'enzui-input-probe-byte-%s\\n' \"$(printf %s \"$byte\" | tr -d ' ')\""
        ));
        cmd.env("TERM", "xterm-256color");
        Self::finish_spawn_with_wake_bus(session_id, pair, cmd, wake, default_reader_spawner)
    }

    /// Spawn with the engine's default history limit and report cwd resolution.
    #[cfg(test)]
    pub(crate) fn spawn_shell_at(
        session_id: impl Into<String>,
        requested_cwd: Option<&str>,
    ) -> Result<(Self, LocalCwdResolution), PtyError> {
        Self::spawn_shell_at_with_wake_bus(session_id, requested_cwd, wake_bus())
    }

    #[cfg(test)]
    pub(crate) fn spawn_shell_at_with_wake_bus(
        session_id: impl Into<String>,
        requested_cwd: Option<&str>,
        wake: &'static WakeBus,
    ) -> Result<(Self, LocalCwdResolution), PtyError> {
        Self::spawn_shell_at_configured(session_id, requested_cwd, wake, emulator::MAX_SCROLLBACK)
    }

    pub(crate) fn spawn_shell_at_with_scrollback(
        session_id: impl Into<String>,
        requested_cwd: Option<&str>,
        scrollback_lines: usize,
    ) -> Result<(Self, LocalCwdResolution), PtyError> {
        Self::spawn_shell_at_configured(session_id, requested_cwd, wake_bus(), scrollback_lines)
    }

    pub(crate) fn spawn_shell_at_configured(
        session_id: impl Into<String>,
        requested_cwd: Option<&str>,
        wake: &'static WakeBus,
        scrollback_lines: usize,
    ) -> Result<(Self, LocalCwdResolution), PtyError> {
        let cwd = resolve_local_cwd(requested_cwd);
        Self::spawn_local_resolved(session_id, cwd, wake, scrollback_lines)
    }

    pub(crate) fn spawn_explicit_directory(
        session_id: impl Into<String>,
        requested_cwd: Option<&str>,
        scrollback_lines: usize,
    ) -> Result<(Self, LocalCwdResolution), PtyError> {
        let cwd = resolve_local_cwd(requested_cwd);
        if cwd.warning.is_some() || cwd.requested.is_none() {
            return Err(PtyError::Spawn(
                "Requested working directory is no longer available.".into(),
            ));
        }
        Self::spawn_local_resolved(session_id, cwd, wake_bus(), scrollback_lines)
    }

    fn spawn_local_resolved(
        session_id: impl Into<String>,
        cwd: LocalCwdResolution,
        wake: &'static WakeBus,
        scrollback_lines: usize,
    ) -> Result<(Self, LocalCwdResolution), PtyError> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| PtyError::OpenPty(err.to_string()))?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let shell = if std::path::Path::new(&shell).is_absolute() {
            PathBuf::from(shell)
        } else {
            resolve_executable(&shell).ok_or_else(|| {
                PtyError::Spawn("Selected shell executable authority is unavailable.".into())
            })?
        };
        let mut cmd = CommandBuilder::new(shell);
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", &home);
        }
        Self::configure_local_shell(&mut cmd, &cwd.effective);

        let session = Self::finish_spawn_configured(
            session_id,
            pair,
            cmd,
            wake,
            default_reader_spawner,
            None,
            scrollback_lines,
            #[cfg(all(test, target_os = "linux"))]
            PtySpawnTestControl::default(),
        )?;
        Ok((session, cwd))
    }

    pub(crate) fn configure_local_shell(cmd: &mut CommandBuilder, cwd: &std::path::Path) {
        cmd.env("TERM", "xterm-256color");
        cmd.env("TERM_PROGRAM", "enzui-terminal");
        cmd.cwd(cwd);
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn spawn_history_probe(
        session_id: &str,
        root: &std::path::Path,
    ) -> Result<Self, PtyError> {
        let mut cmd = CommandBuilder::new("/bin/bash");
        cmd.args(["--noprofile", "--norc", "-i"]);
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.env("HOME", root);
        for (key, name) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            cmd.env(key, root.join(name));
        }
        cmd.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/no-bus", root.display()),
        );
        cmd.env("HISTFILE", "/dev/null");
        Self::configure_local_shell(&mut cmd, root);
        // Set PS1 after /bin/sh resets it, without loading system or user bashrc hooks.
        cmd.env(
            "PROMPT_COMMAND",
            format!("{BASH_PROMPT_COMMAND}\nPS1='fixture-prompt> '"),
        );
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize::default())
            .map_err(|err| PtyError::OpenPty(err.to_string()))?;
        Self::finish_spawn_with_wake_bus_controlled(
            session_id,
            pair,
            cmd,
            Box::leak(Box::new(WakeBus::default())),
            default_reader_spawner,
            None,
            PtySpawnTestControl {
                capability_path: Some(root.join("capability.fifo")),
                ..PtySpawnTestControl::default()
            },
        )
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn spawn_native_history_probe(
        id: &str,
        root: &std::path::Path,
        executable: &std::path::Path,
    ) -> Result<Self, PtyError> {
        Self::spawn_native_history_probe_at(id, root, executable, root)
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn spawn_native_history_probe_at(
        id: &str,
        root: &std::path::Path,
        executable: &std::path::Path,
        cwd: &std::path::Path,
    ) -> Result<Self, PtyError> {
        let mut cmd = CommandBuilder::new(executable);
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.env("HOME", root);
        cmd.env("SHELL", executable);
        cmd.env("INPUTRC", "/dev/null");
        cmd.env("HISTFILE", "/dev/null");
        cmd.env("fish_history", "");
        cmd.env("LC_ALL", "C.UTF-8");
        for (key, name) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            cmd.env(key, root.join(name));
        }
        cmd.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/no-bus", root.display()),
        );
        Self::configure_local_shell(&mut cmd, cwd);
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::OpenPty(error.to_string()))?;
        Self::finish_spawn_configured(
            id,
            pair,
            cmd,
            Box::leak(Box::new(WakeBus::default())),
            default_reader_spawner,
            None,
            500,
            PtySpawnTestControl {
                capability_path: Some(root.join("native-capability.fifo")),
                ..PtySpawnTestControl::default()
            },
        )
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn generated_startup_path(&self) -> Option<PathBuf> {
        self.shell_startup
            .as_ref()
            .map(|startup| startup.path().to_owned())
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn spawn_agent_output_probe(
        id: &str,
        root: &std::path::Path,
        host: &SshHostRecord,
    ) -> Result<Self, PtyError> {
        let ssh = root.join("ssh");
        let shell = root.join("private-shell");
        for (path, body) in [
            (
                &ssh,
                r#"#!/bin/bash
printf 'AUTH_ONLY_2929\n'
remote=${!#}
exec /bin/sh -c "$remote"
"#,
            ),
            (
                &shell,
                r#"#!/bin/sh
PROMPT_COMMAND="$PROMPT_COMMAND
PS1='fixture-prompt> '"
export PROMPT_COMMAND
exec /bin/bash --noprofile --norc -i
"#,
            ),
        ] {
            // Written by a child process, so a parallel test's fork cannot
            // make running it fail with "Text file busy".
            enzui_test_support::fake_executable::write_executable_with_mode(path, body, 0o700);
        }
        let (mut cmd, secret) =
            build_ssh_command_with(host, None, |name| (name == "ssh").then(|| ssh.clone()))?;
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.env("HOME", root);
        cmd.env("HOSTNAME", "agent.invalid");
        cmd.env("INPUTRC", "/dev/null");
        cmd.env("SHELL", &shell);
        cmd.env("HISTFILE", "/dev/null");
        cmd.env("TERM", "xterm-256color");
        for (key, name) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            cmd.env(key, root.join(name));
        }
        cmd.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/no-bus", root.display()),
        );
        cmd.cwd(root);
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 4,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::OpenPty(error.to_string()))?;
        Self::finish_spawn_configured(
            id,
            pair,
            cmd,
            wake_bus(),
            default_reader_spawner,
            secret,
            500,
            PtySpawnTestControl {
                capability_path: Some(root.join("capability.fifo")),
                ..PtySpawnTestControl::default()
            },
        )
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn spawn_control_probe(
        id: &str,
        root: &std::path::Path,
        rows: u16,
        cols: u16,
        cap: usize,
    ) -> Result<Self, PtyError> {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "stty -echo -onlcr || exit 1; : > ready; while IFS= read -r payload; do printf '%b' \"$payload\"; done"]);
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.env("HOME", root);
        cmd.cwd(root);
        for (key, path) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            cmd.env(key, root.join(path));
        }
        cmd.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/no-bus", root.display()),
        );
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::OpenPty(error.to_string()))?;
        Self::finish_spawn_configured(
            id,
            pair,
            cmd,
            Box::leak(Box::new(WakeBus::default())),
            default_reader_spawner,
            None,
            cap,
            PtySpawnTestControl {
                capability_path: Some(root.join("capability.fifo")),
                ..PtySpawnTestControl::default()
            },
        )
    }
}
