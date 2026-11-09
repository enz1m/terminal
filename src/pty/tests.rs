use super::*;
// The wake-stream guards poll a `Stream` by hand, with a no-op waker
// and no executor, so what they assert is the stream's OWN readiness and
// not an executor's scheduling.

/// A real pty pair plus a child command carrying `marker` in its argv, or
/// `None` when the sandbox has no `/dev/ptmx`.
///
/// A REAL pair on purpose: the claim is about `finish_spawn`, which spawns
/// the child and takes the writer/reader before it ever reaches the reader
/// thread. A fake pair would skip all of that.
///
/// The child **ignores SIGHUP** and sleeps, and that detail is load-bearing
/// for `a_failing_reader_thread_spawn_leaves_no_orphan_child`. A plain `cat`
/// exits on its own the moment the last master fd is dropped, so it makes
/// the orphan guard vacuous — it passes with the cleanup deleted (measured).
/// A HUP-ignoring child survives the hangup and is only reaped if the error
/// path really kills it, which is the whole claim.
fn real_pair_and_cmd(marker: &str) -> Option<(PtyPair, CommandBuilder)> {
    // The shared builder preserves the fixture's load-bearing properties:
    // ignored HUP, a bounded shell wait loop, and a marker in argv rather
    // than only in output. The successful-drop variant asks that same builder
    // for a marked descendant which also ignores TERM.
    real_hup_ignoring_pair_and_cmd(marker, false)
}

/// A spawner that fails the way a thread-starved host does.
fn eagain_spawner(_body: ReaderBody) -> std::io::Result<thread::JoinHandle<()>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        "Resource temporarily unavailable",
    ))
}

/// `eagain_spawner`, but it LEAKS the reader body instead of dropping it.
///
/// This is what makes `a_failing_reader_thread_spawn_leaves_no_orphan_child`
/// able to fail at all, and it needs justifying. The reader inside the body
/// is a `dup` of the pty master. When every master fd closes, the kernel
/// sends SIGHUP to the child's session — and the child dies from THAT,
/// microseconds after being forked and before it can install any handler.
/// Measured: with the body dropped, the child is never once visible in /proc
/// and the orphan guard passes with `child.kill()` deleted, which is exactly
/// the "gate on the wrong thing" trap.
///
/// Holding one master fd open removes the hangup, so the ONLY thing that can
/// reap the child is the error path's own `kill`/`wait`. That is precisely
/// the claim under test, and it is also the real hazard: a spawn failure can
/// leave the reader alive (another `Arc` clone, a future retry holding it) and
/// then nothing at all cleans up the shell.
fn eagain_spawner_retaining_body(body: ReaderBody) -> std::io::Result<thread::JoinHandle<()>> {
    // One deliberately leaked fd for the lifetime of the test binary.
    Box::leak(body);
    Err(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        "Resource temporarily unavailable",
    ))
}

#[cfg(target_os = "linux")]
fn all_marked_pids(marker: &str) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.chars().all(|c| c.is_ascii_digit()))
        .filter(|name| {
            std::fs::read(format!("/proc/{name}/cmdline"))
                .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(marker))
        })
        .filter_map(|name| name.parse().ok())
        .collect()
}

#[cfg(target_os = "linux")]
fn capability_test_path(label: &str) -> (capability::tests::RuntimeFixture, PathBuf) {
    let fixture = capability::tests::RuntimeFixture::new();
    let path = fixture.runtime().join(label);
    (fixture, path)
}

#[cfg(target_os = "linux")]
fn process_group_of(pid: u32) -> Option<u32> {
    read_process_stat(pid).ok().map(|stat| stat.process_group)
}

#[cfg(target_os = "linux")]
fn process_parent_of(pid: u32) -> Option<u32> {
    read_process_stat(pid).ok().map(|stat| stat.parent)
}

#[cfg(target_os = "linux")]
fn process_session_of(pid: u32) -> Option<u32> {
    read_process_stat(pid).ok().map(|stat| stat.session)
}

#[cfg(target_os = "linux")]
struct UnrelatedPtyHolder(std::process::Child);

#[cfg(target_os = "linux")]
impl UnrelatedPtyHolder {
    fn spawn(tty: &std::path::Path, marker: &str) -> std::io::Result<Self> {
        std::process::Command::new("/bin/bash")
            .args([
                "-c",
                "exec 3<>\"$1\"; exec -a \"$2-unrelated\" /bin/sleep 20",
                "unrelated-holder",
            ])
            .arg(tty)
            .arg(marker)
            .spawn()
            .map(Self)
            .and_then(|holder| {
                if holder.settled(marker) {
                    Ok(holder)
                } else {
                    Err(std::io::Error::other("the holder never became its sleep"))
                }
            })
    }

    /// Wait until the holder is `sleep`, holding only what it opened itself.
    ///
    /// `posix_spawn` returns once the child's exec has replaced its memory,
    /// before the kernel closes its close-on-exec descriptors. Until then the
    /// child's `/proc/<pid>/fd` still lists this process's descriptors,
    /// including the session's slave and its retained capability FIFO, so a
    /// check made then calls an unrelated process a capability holder. On
    /// one CPU that window is the common case.
    fn settled(&self, marker: &str) -> bool {
        let wanted = format!("{marker}-unrelated");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let argv0 = std::fs::read(format!("/proc/{}/cmdline", self.pid()))
                .ok()
                .and_then(|cmdline| {
                    cmdline
                        .split(|byte| *byte == 0)
                        .next()
                        .map(|arg| String::from_utf8_lossy(arg).into_owned())
                });
            if argv0.as_deref() == Some(wanted.as_str()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }

    /// Every descriptor the holder has open, for a failure message.
    fn descriptors(&self) -> Vec<String> {
        use std::os::unix::fs::FileTypeExt as _;
        let Ok(entries) = std::fs::read_dir(format!("/proc/{}/fd", self.pid())) else {
            return vec!["<no fd directory>".into()];
        };
        let mut found: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|entry| {
                let target = std::fs::read_link(entry.path())
                    .map_or_else(|err| format!("<{err}>"), |path| path.display().to_string());
                let identity = std::fs::metadata(entry.path()).map_or_else(
                    |err| format!("<{err}>"),
                    |meta| {
                        format!(
                            "dev={} ino={} fifo={}",
                            meta.dev(),
                            meta.ino(),
                            meta.file_type().is_fifo()
                        )
                    },
                );
                format!(
                    "{} -> {target} ({identity})",
                    entry.file_name().to_string_lossy()
                )
            })
            .collect();
        found.sort();
        found
    }

    fn is_alive(&mut self) -> bool {
        self.0.try_wait().is_ok_and(|status| status.is_none())
    }
}

fn argv_strings(cmd: &CommandBuilder) -> Vec<String> {
    cmd.get_argv()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn assert_double_dash_isolates_destination(cmd: &CommandBuilder, destination: &str) {
    let argv = argv_strings(cmd);
    let dd = argv
        .iter()
        .position(|a| a == "--")
        .unwrap_or_else(|| panic!("argv must contain a literal `--`; argv={argv:?}"));
    assert_eq!(
        argv.get(dd + 1).map(String::as_str),
        Some(destination),
        "the destination must be the single argument immediately after `--`; argv={argv:?}"
    );
    for before in &argv[..dd] {
        assert!(
            !before.contains("ProxyCommand"),
            "a destination option leaked before `--`: {before:?} in {argv:?}"
        );
    }
}

// =====================================================================
// SSH launch hardening. Each mechanism is routed through the
// production `build_ssh_command_with` / `build_remote_command` /
// `resolve_executable_in` / `PtySession::spawn_ssh_with` seams and has its
// OWN sabotage control (recorded in the issue): the argv `--`, POSIX cwd
// quoting, the anonymous sealed-`memfd` password channel, and absolute
// executable-identity resolution.

fn ssh_2511_host(destination: &str, path: &str) -> crate::model::SshHostRecord {
    crate::model::SshHostRecord {
        id: "t".into(),
        name: "t".into(),
        destination: destination.into(),
        port: 22,
        username: String::new(),
        path: path.into(),
        auth: crate::model::SshAuthMode::Password,
        notes: String::new(),
    }
}

/// A resolver over a fixed name->absolute-path map, so a test can inject the
/// exact executables `build_ssh_command_with` will launch without touching
/// process env. Any unlisted name resolves to `None` (missing executable).
fn ssh_2511_resolver(entries: Vec<(&'static str, PathBuf)>) -> impl Fn(&str) -> Option<PathBuf> {
    move |bin| {
        entries
            .iter()
            .find(|(name, _)| *name == bin)
            .map(|(_, path)| path.clone())
    }
}

/// A stub sshpass that records its argv to `argv_out` AND copies the bytes of
/// the file named by its `-f` argument (`$2`) to `secret_out`, proving a
/// same-uid child launched THROUGH portable-pty (which closes every inherited
/// fd >= 3 before exec) can still open the `/proc/<pid>/fd/<n>` magic path and
/// read the exact secret bytes.
#[cfg(target_os = "linux")]
fn ssh_2511_secret_capture_stub(
    name: &str,
    argv_out: &std::path::Path,
    secret_out: &std::path::Path,
) -> PathBuf {
    let dir = ssh_2511_tmp_dir(name);
    std::fs::create_dir_all(&dir).expect("stub dir");
    let bin = dir.join(name);
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{argv}'\ncat \"$2\" > '{secret}' 2>/dev/null\nexit 0\n",
        argv = argv_out.display(),
        secret = secret_out.display(),
    );
    enzui_test_support::fake_executable::write_executable(&bin, &script);
    bin
}

fn ssh_2511_stub_executable(name: &str) -> PathBuf {
    let dir = ssh_2511_tmp_dir(name);
    std::fs::create_dir_all(&dir).expect("stub dir");
    let bin = dir.join(name);
    enzui_test_support::fake_executable::write_executable(&bin, "#!/bin/sh\nexit 0\n");
    bin
}

fn ssh_2511_tmp_dir(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "enzui-2511-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Fresh loop plumbing (screen + the three atomics the real thread owns +
/// a PRIVATE wake bus).
///
/// The bus is per-test, not `wake_bus()`: the global one is shared by every
/// test in the binary and by any real PTY a fixture spawned, so counting
/// wakes against it would be counting other tests' output.
type LoopParts = (Arc<Mutex<emulator::Screen>>, Arc<AtomicU64>, WakeBus);

/// Best-effort cleanup so a red run leaves nothing behind.
#[cfg(target_os = "linux")]
fn kill_marked_children(marker: &str) {
    for pid in all_marked_pids(marker) {
        let Some(pid) = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
        else {
            continue;
        };
        if let Ok(pidfd) = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()) {
            let _ = rustix::process::pidfd_send_signal(&pidfd, rustix::process::Signal::KILL);
        }
    }
}

#[cfg(target_os = "linux")]
struct MarkedProcessCleanup(String);

#[cfg(target_os = "linux")]
impl Drop for MarkedProcessCleanup {
    fn drop(&mut self) {
        kill_marked_children(&self.0);
    }
}

#[path = "tests/capability_tests.rs"]
mod capability_tests;

#[path = "tests/identity_tests.rs"]
mod identity_tests;
#[path = "tests/projection_tests.rs"]
mod projection_tests;
#[path = "tests/reader_tests.rs"]
mod reader_tests;

#[path = "tests/session_tests.rs"]
mod session_tests;
fn ssh_2511_secret_identity_set() -> std::collections::HashSet<MemfdIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        panic!("/proc/self/fd must be readable for the descriptor inventory");
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            std::fs::read_link(entry.path())
                .is_ok_and(|target| target.to_string_lossy().contains("memfd:enzui-pty-data"))
        })
        .filter_map(|entry| std::fs::metadata(entry.path()).ok())
        .map(|meta| MemfdIdentity {
            device: meta.dev(),
            inode: meta.ino(),
        })
        .collect()
}

fn ssh_2511_no_new_secret_identity(baseline: &std::collections::HashSet<MemfdIdentity>) -> bool {
    for _ in 0..200 {
        if ssh_2511_secret_identity_set().is_subset(baseline) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// The actual shutdown-order probe, run only inside a disposable child-test
/// process. If sabotage wedges `shutdown`, the parent test can kill the entire
/// owner process instead of panicking while a Rust thread (and its PTY child)
/// remains detached in this test process.
#[cfg(target_os = "linux")]
fn ssh_2511_shutdown_probe_body() {
    // 0 = reader body never ran, 1 = still resolvable at reader exit,
    // 2 = ALREADY released at reader exit, 3 = path not published in time.
    let observation = Arc::new(AtomicU64::new(0));
    let slot: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

    let obs = Arc::clone(&observation);
    let path_slot = Arc::clone(&slot);
    let spawner = move |body: ReaderBody| {
        thread::Builder::new()
            .name("terminal-emulator-pty-reader".into())
            .spawn(move || {
                // The REAL production reader body; it returns when `shutdown`
                // cancels it, i.e. during teardown, before the joins finish.
                body();
                let published = path_slot.lock().unwrap().clone();
                let verdict = match published {
                    Some(path) => {
                        if std::fs::File::open(&path).is_ok() {
                            1
                        } else {
                            2
                        }
                    }
                    None => 3,
                };
                obs.store(verdict, Ordering::Release);
            })
    };

    let ssh_bin = ssh_2511_stub_executable("order-ssh");
    let sshpass_bin = ssh_2511_sleeper_stub("order-sshpass");
    let host = ssh_2511_host("example.net", "");
    let resolve = ssh_2511_resolver(vec![("ssh", ssh_bin), ("sshpass", sshpass_bin)]);

    let mut session =
        match PtySession::spawn_ssh_with("order-route", &host, Some("S3NT-INEL"), resolve, spawner)
        {
            Ok(session) => session,
            Err(PtyError::OpenPty(err)) => panic!(
                "this test REQUIRES a real PTY; openpty failed: {err}. \
             The capability preflight `ssh_2511_openpty_capability_is_available` \
             reports whether the environment can support these targets at all \
             — a routed test must never report green by skipping"
            ),
            Err(other) => panic!("spawn_ssh_with failed unexpectedly: {other:?}"),
        };

    let proc_path = session
        ._password_secret
        .as_ref()
        .expect("spawn_ssh must retain the password memfd")
        .proc_path()
        .to_path_buf();
    *slot.lock().unwrap() = Some(proc_path.clone());
    assert!(
        std::fs::File::open(&proc_path).is_ok(),
        "the memfd resolves while the session is live"
    );

    let release_probe = SecretReleaseProbe::default();
    let secret_identity = session
        ._password_secret
        .as_ref()
        .expect("session retains exact secret")
        .identity;
    release_probe.set_child(
        session.process_identity.pid,
        session.process_identity.start_ticks,
    );
    session
        ._password_secret
        .as_mut()
        .expect("session retains exact secret")
        .release_probe = Some(release_probe.clone());

    session.shutdown();

    match observation.load(Ordering::Acquire) {
        1 => {}
        2 => panic!(
            "shutdown released the password memfd BEFORE teardown/joins completed; \
             a lazily-opening sshpass could then read a recycled fd number"
        ),
        3 => panic!("the published path was not visible to the observer"),
        _ => panic!("the production reader body never ran, so ordering was not observed"),
    }

    // Released once teardown is complete; session deliberately still in scope.
    assert!(
        session._password_secret.is_none(),
        "shutdown must take the secret out of the session once teardown completes"
    );
    assert!(
        std::fs::File::open(&proc_path).is_err(),
        "shutdown must release the memfd after teardown completes"
    );
    let releases = release_probe.observations();
    assert_eq!(
        releases.len(),
        1,
        "exactly one secret release: {releases:?}"
    );
    assert_eq!(releases[0].secret, secret_identity);
    assert!(
        releases[0].exact_child_gone,
        "the exact memfd inode was released before the exact PTY child was reaped: {:?}",
        releases[0]
    );
}

/// A long-lived stub so the reader thread stays parked in the kernel until
/// `shutdown` cancels it, making the teardown ordering observable.
#[cfg(target_os = "linux")]
fn ssh_2511_sleeper_stub(name: &str) -> PathBuf {
    let dir = ssh_2511_tmp_dir(name);
    std::fs::create_dir_all(&dir).expect("stub dir");
    let bin = dir.join(name);
    enzui_test_support::fake_executable::write_executable(&bin, "#!/bin/sh\nsleep 30\n");
    bin
}

#[path = "tests/spawn_tests.rs"]
mod spawn_tests;

#[path = "tests/ssh_confirm_tests.rs"]
mod ssh_confirm_tests;
#[path = "tests/ssh_hostile_tests.rs"]
mod ssh_hostile_tests;
#[path = "tests/ssh_memfd_tests.rs"]
mod ssh_memfd_tests;
#[path = "tests/ssh_preflight_tests.rs"]
mod ssh_preflight_tests;
#[path = "tests/ssh_route_tests.rs"]
mod ssh_route_tests;
#[path = "tests/ssh_secret_tests.rs"]
mod ssh_secret_tests;
#[path = "tests/ssh_shutdown_tests.rs"]
mod ssh_shutdown_tests;
#[path = "tests/ssh_spawn_preflight_tests.rs"]
mod ssh_spawn_preflight_tests;
#[path = "tests/ssh_spawn_tests.rs"]
mod ssh_spawn_tests;
