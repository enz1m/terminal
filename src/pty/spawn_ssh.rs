//! PtySession SSH spawning and finish paths.

use super::*;

impl PtySession {
    #[cfg(test)]
    pub(crate) fn spawn_ssh_with<R, S>(
        session_id: impl Into<String>,
        host: &SshHostRecord,
        password: Option<&str>,
        resolve: R,
        spawner: S,
    ) -> Result<Self, PtyError>
    where
        R: Fn(&str) -> Option<PathBuf>,
        S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
    {
        Self::spawn_ssh_configured(
            session_id,
            host,
            password,
            None,
            resolve,
            spawner,
            emulator::MAX_SCROLLBACK,
        )
    }

    pub(crate) fn spawn_ssh_with_scrollback(
        session_id: impl Into<String>,
        host: &SshHostRecord,
        password: Option<&str>,
        scrollback_lines: usize,
    ) -> Result<Self, PtyError> {
        Self::spawn_ssh_configured(
            session_id,
            host,
            password,
            None,
            resolve_executable,
            default_reader_spawner,
            scrollback_lines,
        )
    }

    pub(crate) fn spawn_ssh_alias_with_scrollback(
        session_id: impl Into<String>,
        host: &SshHostRecord,
        hostname: &str,
        scrollback_lines: usize,
    ) -> Result<Self, PtyError> {
        Self::spawn_ssh_configured(
            session_id,
            host,
            None,
            Some(hostname),
            resolve_executable,
            default_reader_spawner,
            scrollback_lines,
        )
    }

    pub(crate) fn spawn_ssh_configured<R, S>(
        session_id: impl Into<String>,
        host: &SshHostRecord,
        password: Option<&str>,
        hostname: Option<&str>,
        resolve: R,
        spawner: S,
        scrollback_lines: usize,
    ) -> Result<Self, PtyError>
    where
        R: Fn(&str) -> Option<PathBuf>,
        S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
    {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| PtyError::OpenPty(err.to_string()))?;

        let (mut cmd, password_channel) = build_ssh_command_with(host, password, resolve)?;
        if let Some(hostname) = hostname {
            pin_ssh_hostname(&mut cmd, hostname)?;
        }

        cmd.env("TERM", "xterm-256color");
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", &home);
            cmd.cwd(home);
        }

        // Hand the sealed `memfd` to the spawn sequence by value. sshpass opens
        // its `-f` path lazily after portable-pty's `close_random_fds`, so the
        // object AND its descriptor NUMBER must stay reserved for as long as a
        // child could still perform that open. Ownership transfers here rather
        // than being assigned after the fact precisely because the failures
        // that occur AFTER `spawn_command` (take_writer, try_clone_reader, the
        // worker-thread spawns) must kill and reap that child before the number
        // is released; a caller that merely dropped the channel on error would
        // free the number underneath a live sshpass.
        Self::finish_spawn_configured(
            session_id,
            pair,
            cmd,
            wake_bus(),
            spawner,
            password_channel,
            scrollback_lines,
            #[cfg(all(test, target_os = "linux"))]
            PtySpawnTestControl::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn finish_spawn(
        session_id: impl Into<String>,
        pair: PtyPair,
        cmd: CommandBuilder,
    ) -> Result<Self, PtyError> {
        Self::finish_spawn_with(session_id, pair, cmd, default_reader_spawner)
    }

    /// Test compatibility path with the reader-thread spawner injected.
    ///
    /// `thread::Builder::spawn` cannot be made to fail on demand, so tests hand
    /// this function a spawner that returns the error. It delegates to the same
    /// common spawn path used by production.
    #[cfg(test)]
    pub(crate) fn finish_spawn_with<S>(
        session_id: impl Into<String>,
        pair: PtyPair,
        cmd: CommandBuilder,
        spawner: S,
    ) -> Result<Self, PtyError>
    where
        S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
    {
        Self::finish_spawn_with_secret(session_id, pair, cmd, spawner, None)
    }

    /// [`finish_spawn_with`] taking ownership of the password channel.
    ///
    /// The secret is handed over BEFORE the child is spawned so that every
    /// post-spawn failure branch inside the spawn sequence can kill and reap
    /// that child before the descriptor number is released. A caller that kept
    /// the secret and merely dropped it on error would free the number while a
    /// live sshpass could still open `/proc/<pid>/fd/<n>`.
    #[cfg(test)]
    pub(crate) fn finish_spawn_with_secret<S>(
        session_id: impl Into<String>,
        pair: PtyPair,
        cmd: CommandBuilder,
        spawner: S,
        secret: PasswordChannel,
    ) -> Result<Self, PtyError>
    where
        S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
    {
        Self::finish_spawn_with_wake_bus_controlled(
            session_id,
            pair,
            cmd,
            wake_bus(),
            spawner,
            secret,
            #[cfg(all(test, target_os = "linux"))]
            PtySpawnTestControl::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn finish_spawn_with_wake_bus<S>(
        session_id: impl Into<String>,
        pair: PtyPair,
        cmd: CommandBuilder,
        wake: &'static WakeBus,
        spawner: S,
    ) -> Result<Self, PtyError>
    where
        S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
    {
        Self::finish_spawn_with_wake_bus_controlled(
            session_id,
            pair,
            cmd,
            wake,
            spawner,
            None,
            #[cfg(all(test, target_os = "linux"))]
            PtySpawnTestControl::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn finish_spawn_with_wake_bus_controlled<S>(
        session_id: impl Into<String>,
        pair: PtyPair,
        cmd: CommandBuilder,
        wake: &'static WakeBus,
        spawner: S,
        secret: PasswordChannel,
        #[cfg(all(test, target_os = "linux"))] control: PtySpawnTestControl,
    ) -> Result<Self, PtyError>
    where
        S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
    {
        Self::finish_spawn_configured(
            session_id,
            pair,
            cmd,
            wake,
            spawner,
            secret,
            emulator::MAX_SCROLLBACK,
            #[cfg(all(test, target_os = "linux"))]
            control,
        )
    }

    #[cfg_attr(
        all(test, target_os = "linux"),
        expect(
            clippy::too_many_arguments,
            reason = "The test-only control adds one argument to the production spawn boundary"
        )
    )]
    pub(crate) fn finish_spawn_configured<S>(
        session_id: impl Into<String>,
        pair: PtyPair,
        cmd: CommandBuilder,
        wake: &'static WakeBus,
        spawner: S,
        secret: PasswordChannel,
        scrollback_lines: usize,
        #[cfg(all(test, target_os = "linux"))] control: PtySpawnTestControl,
    ) -> Result<Self, PtyError>
    where
        S: FnOnce(ReaderBody) -> std::io::Result<thread::JoinHandle<()>>,
    {
        // Claimed first because it is the one step no later unwinding can
        // undo: refusal must leave no child, no writer thread, no queue and no
        // teardown slot, and it is permanent, since nothing resets the
        // allocator. Sessions already running keep their own generations.
        let generation = INPUT_GENERATIONS
            .allocate()
            .ok_or(PtyError::InputAuthorityExhausted)?;

        #[cfg(unix)]
        let (cmd, shell_startup, startup_warning) = {
            let mut cmd = cmd;
            let local_request = cmd.get_env("TERM_PROGRAM")
                == Some(std::ffi::OsStr::new("enzui-terminal"))
                && cmd.get_argv().len() == 1;
            let (startup, warning) = match crate::shell_history::local::prepare(&mut cmd) {
                Ok(Some(startup)) => (Some(startup), None),
                Ok(None) => (
                    None,
                    local_request.then_some(crate::shell_history::local::UNSUPPORTED),
                ),
                Err(_) => (None, Some(crate::shell_history::local::UNAVAILABLE)),
            };
            (cmd, startup, warning)
        };
        let program = cmd
            .get_argv()
            .first()
            .and_then(|program| std::path::Path::new(program).file_name());
        let local_protocol = program == Some(std::ffi::OsStr::new("bash"))
            && cmd
                .get_env("PROMPT_COMMAND")
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.contains(BASH_PROMPT_COMMAND));
        let remote_protocol = program == Some(std::ffi::OsStr::new("ssh"))
            && cmd
                .get_argv()
                .last()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.contains(&shell_single_quote(BASH_PROMPT_COMMAND)));
        #[cfg(unix)]
        let local_protocol = local_protocol || shell_startup.is_some();
        let history_source = (local_protocol || remote_protocol) && secret.is_none();
        #[cfg(unix)]
        let history_source = history_source && startup_warning.is_none();
        let secret_delivery = secret.is_some();
        #[cfg(all(test, target_os = "linux"))]
        let mut secret = secret;
        #[cfg(not(all(test, target_os = "linux")))]
        let secret = secret;

        #[cfg(all(test, target_os = "linux"))]
        if let (Some(secret), Some(probe)) =
            (secret.as_mut(), control.secret_release_probe.as_ref())
        {
            secret.release_probe = Some(probe.clone());
        }

        // Capability failure must leave the shell usable with leader/SID ownership,
        // not invent descriptor inheritance.
        #[cfg(target_os = "linux")]
        let mut capability = None;
        #[cfg(target_os = "linux")]
        let created_capability = {
            #[cfg(test)]
            {
                match control.capability_path.clone() {
                    Some(path) => PtyCapability::create_for_test(
                        path,
                        control.setup_failure,
                        control.path_was_visible.as_deref(),
                    ),
                    None => PtyCapability::create(&cmd),
                }
            }
            #[cfg(not(test))]
            {
                PtyCapability::create(&cmd)
            }
        };
        #[cfg(target_os = "linux")]
        let cmd = match created_capability {
            Ok(created) => {
                #[cfg(test)]
                let handshake_path = if control.fail_capability_handshake {
                    created.handoff_path().with_extension("missing")
                } else {
                    created.handoff_path()
                };
                #[cfg(not(test))]
                let handshake_path = created.handoff_path();
                let wrapped = wrap_with_capability(cmd, &handshake_path);
                #[cfg(test)]
                let wrapped = if control.delay_capability_handshake {
                    let mut wrapped = wrapped;
                    let script = wrapped.get_argv()[2]
                        .to_str()
                        .expect("capability script")
                        .to_owned();
                    wrapped.get_argv_mut()[2] = format!(
                        "/bin/bash --noprofile --norc -c 'IFS= read -r -t 5 value'; {script}"
                    )
                    .into();
                    wrapped
                } else {
                    wrapped
                };
                capability = Some(created);
                wrapped
            }
            Err(_) => cmd,
        };

        // The reader must be cancellable even when an unrelated process has
        // independently opened the slave and therefore prevents master EOF.
        // Poll a `dup` of THIS master alongside a private cancellation pipe;
        // both pty descriptors stay blocking. The test-only errors are placed
        // immediately before the corresponding fallible production operation,
        // after capability ownership has begun.
        #[cfg(all(test, target_os = "linux"))]
        if control.setup_failure == Some(PtySetupFailure::MasterDup) {
            return Err(PtyError::CloneReader("injected master dup failure".into()));
        }
        #[cfg(target_os = "linux")]
        let reader_poll_master = pair
            .master
            .as_raw_fd()
            .ok_or_else(|| PtyError::CloneReader("PTY master has no file descriptor".into()))
            .and_then(|fd| {
                filedescriptor::FileDescriptor::dup(&BorrowedMasterFd(fd))
                    .map_err(|err| PtyError::CloneReader(err.to_string()))
            })?;
        #[cfg(all(test, target_os = "linux"))]
        if control.setup_failure == Some(PtySetupFailure::CancelPipe) {
            return Err(PtyError::CloneReader(
                "injected cancellation pipe failure".into(),
            ));
        }
        #[cfg(target_os = "linux")]
        let mut reader_cancel =
            filedescriptor::Pipe::new().map_err(|err| PtyError::CloneReader(err.to_string()))?;

        // Teardown custody is reserved before a child exists: an unresolved
        // teardown must always be charged, and must never need a fallback slot.
        let reaper_slot = teardown_supervisor()
            .reserve()
            .ok_or(PtyError::TeardownCapacity)?;

        #[cfg(all(test, target_os = "linux"))]
        if control.setup_failure == Some(PtySetupFailure::Spawn) {
            return Err(PtyError::Spawn("injected child spawn failure".into()));
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|err| PtyError::Spawn(err.to_string()))?;
        // A live child can open either /proc handoff lazily; every failure below
        // must kill and reap it before releasing either descriptor number.
        #[cfg(all(test, target_os = "linux"))]
        if let Some(slot) = control.spawned_child_pid.as_ref()
            && let Ok(mut slot) = slot.lock()
        {
            *slot = child.process_id();
        }
        #[cfg(target_os = "linux")]
        let capability_handoff = capability.as_ref().map(PtyCapability::retain_handoff);
        #[cfg(target_os = "linux")]
        let capability = capability.and_then(|capability| capability.seal(child.process_id()));
        // portable-pty makes the child a session leader; pin its pidfd before a
        // waiter can reap it and allow its PID/SID to be recycled.
        #[cfg(target_os = "linux")]
        let process_identity = match child.process_id().and_then(|pid| {
            LinuxProcessIdentity::capture(
                pid,
                #[cfg(test)]
                pair.master.tty_name(),
                capability,
            )
            .ok()
        }) {
            Some(identity) => identity,
            None => {
                return Err(kill_reap_then_release(
                    child,
                    secret,
                    PtyError::Spawn("could not retain the spawned process identity".into()),
                ));
            }
        };
        #[cfg(all(test, target_os = "linux"))]
        if let Some(probe) = control.secret_release_probe.as_ref() {
            probe.set_child(process_identity.pid, process_identity.start_ticks);
        }
        #[cfg(not(target_os = "linux"))]
        let child_killer = child.clone_killer();

        #[cfg(all(test, target_os = "linux"))]
        let take_writer_result = if control.setup_failure == Some(PtySetupFailure::TakeWriter) {
            Err(PtyError::TakeWriter("injected take_writer failure".into()))
        } else {
            pair.master
                .take_writer()
                .map_err(|err| PtyError::TakeWriter(err.to_string()))
        };
        #[cfg(not(all(test, target_os = "linux")))]
        let take_writer_result = pair
            .master
            .take_writer()
            .map_err(|err| PtyError::TakeWriter(err.to_string()));
        let writer = match take_writer_result {
            Ok(writer) => writer,
            Err(err) => return Err(kill_reap_then_release(child, secret, err)),
        };

        #[cfg(all(test, target_os = "linux"))]
        let clone_reader_result =
            if control.setup_failure == Some(PtySetupFailure::PostSpawnCloneReader) {
                Err(PtyError::CloneReader(
                    "injected post-spawn clone_reader failure".into(),
                ))
            } else {
                pair.master
                    .try_clone_reader()
                    .map_err(|err| PtyError::CloneReader(err.to_string()))
            };
        #[cfg(not(all(test, target_os = "linux")))]
        let clone_reader_result = pair
            .master
            .try_clone_reader()
            .map_err(|err| PtyError::CloneReader(err.to_string()));
        let reader = match clone_reader_result {
            Ok(reader) => reader,
            Err(err) => return Err(kill_reap_then_release(child, secret, err)),
        };

        let dirty = Arc::new(AtomicU64::new(0));
        // The first reader burst must already obey the application limit.
        let mut initial_screen = emulator::Screen::new(Arc::clone(&dirty));
        initial_screen.set_max_scrollback(scrollback_lines);
        initial_screen.pinned_capture.supported(history_source);
        #[cfg(unix)]
        // Without the integration the shell is just a shell: history is
        // off, and nothing is written into the terminal about it.
        if startup_warning.is_some() {
            initial_screen.disable_shell_integration();
        }
        let screen = Arc::new(Mutex::new(initial_screen));

        // A failed thread spawn is a TRANSIENT condition (EAGAIN under
        // memory pressure), not a bug, so it must travel as a `PtyError` like
        // every other step in this sequence. It used to be
        // `.expect("spawn PTY reader thread")`, which took the whole process —
        // every other tab in the window — down with the one tab that failed to
        // open.
        let child = Arc::new(Mutex::new(child));
        let child_exit = Arc::new(Mutex::new(None));
        #[cfg(test)]
        let reader_done = Arc::new(AtomicBool::new(false));
        #[cfg(any(test, not(target_os = "linux")))]
        let exit_done = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "linux")]
        let reap_request = Arc::new((Mutex::new(false), Condvar::new()));
        let reader_joined = Arc::new(AtomicBool::new(false));
        let exit_joined = Arc::new(AtomicBool::new(false));
        let leader_reaped = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let final_freeze_scans = Arc::new(AtomicU64::new(0));
        #[cfg(test)]
        let final_freeze_stable = Arc::new(AtomicBool::new(false));
        let reader_thread = match spawn_reader_thread_with(
            spawner,
            reader,
            #[cfg(target_os = "linux")]
            LinuxReaderControl {
                master: reader_poll_master,
                cancel: reader_cancel.read,
            },
            Arc::clone(&screen),
            Arc::clone(&dirty),
            #[cfg(test)]
            Arc::clone(&reader_done),
            wake,
        ) {
            Ok(handle) => handle,
            Err(err) => {
                // The child is already running but nothing will ever read its
                // output, so leaving it alive would be an invisible orphan
                // shell holding the pty slave open. Kill AND reap it, then
                // release the password channel: the descriptor number must stay
                // bound to the secret until the child that could still open
                // `/proc/<pid>/fd/<n>` is gone.
                if let Ok(mut child) = child.lock() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                drop(secret);
                return Err(PtyError::SpawnReader(err.to_string()));
            }
        };

        // One retained thread owns the PTY writer, so no reducer turn ever
        // blocks on a child that has stopped reading and no two chunks can
        // interleave on the wire. The queue is installed under the generation
        // claimed at the top of this function.
        let (sender, receiver) = sync_channel::<InputChunk>(MAX_INPUT_QUEUE_CHUNKS);
        let charge = Arc::new(InputCharge::default());
        let writer_joined = Arc::new(AtomicBool::new(false));
        let closing = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let gate = Arc::new(WriterGate::default());
        #[cfg(test)]
        let drained = Arc::new(Mutex::new(Vec::new()));
        let writer_thread = {
            let charge = Arc::clone(&charge);
            let closing = Arc::clone(&closing);
            #[cfg(test)]
            let gate = Arc::clone(&gate);
            #[cfg(test)]
            let drained = Arc::clone(&drained);
            thread::Builder::new()
                .name("terminal-emulator-pty-writer".into())
                .spawn(move || {
                    run_input_writer(
                        writer,
                        receiver,
                        generation,
                        charge,
                        closing,
                        #[cfg(test)]
                        gate,
                        #[cfg(test)]
                        drained,
                    );
                })
        };
        let writer_thread = match writer_thread {
            Ok(handle) => handle,
            Err(err) => {
                // A failed spawn drops the closure, and with it the writer
                // handle, so only the child and the reader need unwinding.
                if let Ok(mut child) = child.lock() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                drop(pair);
                #[cfg(target_os = "linux")]
                let _ = reader_cancel.write.write_all(b"x");
                let _ = reader_thread.join();
                drop(secret);
                return Err(PtyError::SpawnReader(format!("pty input writer: {err}")));
            }
        };

        // Waiting for child status is independent of reading its PTY. In
        // particular, the parent intentionally retains the slave handle in
        // `pair`, so reader EOF is not a reliable exit notification. A
        // dedicated waiter blocks off the UI thread and publishes the detail
        // before waking the app. On Linux WNOWAIT leaves the exited leader
        // waitable until shutdown has safely finished signalling the session.
        let waiter_child = Arc::clone(&child);
        let waiter_exit = Arc::clone(&child_exit);
        #[cfg(any(test, not(target_os = "linux")))]
        let waiter_done = Arc::clone(&exit_done);
        #[cfg(target_os = "linux")]
        let waiter_pidfd = Arc::clone(&process_identity.pidfd);
        #[cfg(target_os = "linux")]
        let waiter_reap_request = Arc::clone(&reap_request);
        let waiter_leader_reaped = Arc::clone(&leader_reaped);
        let exit_thread = match thread::Builder::new()
            .name("terminal-emulator-exit-waiter".into())
            .spawn(move || {
                #[cfg(target_os = "linux")]
                let exit = match rustix::process::waitid(
                    rustix::process::WaitId::PidFd(waiter_pidfd.as_ref().as_fd()),
                    rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOWAIT,
                ) {
                    Ok(Some(status)) => describe_child_exit(&status),
                    Ok(None) => ChildExit {
                        detail: "process status unavailable: wait returned no status".into(),
                        code: None,
                    },
                    Err(err) => ChildExit {
                        detail: format!("process status unavailable: {err}"),
                        code: None,
                    },
                };
                #[cfg(not(target_os = "linux"))]
                let exit = match waiter_child.lock() {
                    Ok(mut child) => match child.wait() {
                        Ok(status) => {
                            waiter_leader_reaped.store(true, Ordering::Release);
                            ChildExit {
                                detail: format!("exit code {}", status.exit_code()),
                                code: i32::try_from(status.exit_code()).ok(),
                            }
                        }
                        Err(err) => ChildExit {
                            detail: format!("process status unavailable: {err}"),
                            code: None,
                        },
                    },
                    Err(_) => ChildExit {
                        detail: "process status unavailable: child lock poisoned".into(),
                        code: None,
                    },
                };
                if let Ok(mut slot) = waiter_exit.lock() {
                    *slot = Some(exit);
                }
                wake.notify();

                #[cfg(target_os = "linux")]
                {
                    let (requested, ready) = &*waiter_reap_request;
                    let mut requested = requested
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while !*requested {
                        requested = ready
                            .wait(requested)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    // The reap this waiter actually performed is the proof the
                    // teardown outcome reads; a `/proc` entry disappearing is
                    // not.
                    if let Ok(mut child) = waiter_child.lock()
                        && child.wait().is_ok()
                    {
                        waiter_leader_reaped.store(true, Ordering::Release);
                    }
                }
                #[cfg(any(test, not(target_os = "linux")))]
                waiter_done.store(true, Ordering::Release);
            }) {
            Ok(handle) => handle,
            Err(err) => {
                if let Ok(mut child) = child.lock() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                // Closing the queue is what ends the writer thread; it is
                // blocked on the queue, never on the child.
                drop(sender);
                let _ = writer_thread.join();
                drop(pair);
                // Cancellation, not slave EOF, releases the reader: an
                // unrelated holder of the exact slave would otherwise strand
                // this join with no bound left.
                #[cfg(target_os = "linux")]
                let _ = reader_cancel.write.write_all(b"x");
                let _ = reader_thread.join();
                // Only now, with the child reaped and both workers joined, may
                // the descriptor number be released.
                drop(secret);
                return Err(PtyError::SpawnReader(format!("reader exit watcher: {err}")));
            }
        };

        Ok(PtySession {
            session_id: session_id.into(),
            live: Mutex::new(None),
            input: Some(PtyInput {
                generation,
                sender: Some(sender),
                charge,
                typed_tail: Mutex::new(None),
                closing,
                thread: Some(writer_thread),
                joined: writer_joined,
                panicked: false,
                #[cfg(test)]
                gate,
                #[cfg(test)]
                drained,
            }),
            #[cfg(unix)]
            shell_startup,
            screen,
            dirty,
            last_seen: AtomicU64::new(0),
            #[cfg(test)]
            recorded_input: Mutex::new(Vec::new()),
            master: Some(pair.master),
            slave: Some(pair.slave),
            child_exit,
            secret_delivery,
            #[cfg(target_os = "linux")]
            process_identity,
            #[cfg(target_os = "linux")]
            reap_request,
            #[cfg(target_os = "linux")]
            reader_cancel: Some(reader_cancel.write),
            #[cfg(not(target_os = "linux"))]
            child_killer: Some(child_killer),
            reader_thread: Some(reader_thread),
            exit_thread: Some(exit_thread),
            #[cfg(test)]
            reader_done,
            #[cfg(any(test, not(target_os = "linux")))]
            exit_done,
            reader_joined,
            exit_joined,
            leader_reaped,
            teardown_token: reaper_slot.token,
            _reaper_slot: reaper_slot,
            teardown_outcome: None,
            #[cfg(target_os = "linux")]
            owned: Vec::new(),
            custody: CustodyState::Held,
            terminate_started: false,
            settled_once: false,
            freeze_stable: true,
            freeze_scans: 0,
            reader_panicked: false,
            waiter_panicked: false,
            #[cfg(test)]
            force_incomplete: false,
            #[cfg(test)]
            force_sweep_panic: false,
            #[cfg(target_os = "linux")]
            _password_secret: secret,
            #[cfg(target_os = "linux")]
            _capability_handoff: capability_handoff,
            #[cfg(test)]
            final_freeze_scans,
            #[cfg(test)]
            final_freeze_stable,
        })
    }
}
