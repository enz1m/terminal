//! SSH spawn commands and password secrets.

use super::*;

/// Abandon a just-spawned child on a post-spawn setup failure: kill it, REAP
/// it, and only then release the password channel, returning `err` unchanged.
///
/// The order is the point. `sshpass` opens its `-f` path lazily, so while the
/// child is alive the descriptor NUMBER in `/proc/<our-pid>/fd/<n>` must stay
/// bound to the secret; releasing it first would free `n` for immediate reuse
/// by any other thread of this process and a late open would then read that
/// unrelated object. `wait` (not merely `kill`) is required because a zombie
/// has not yet been torn down.
pub(crate) fn kill_reap_then_release(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    secret: PasswordChannel,
    err: PtyError,
) -> PtyError {
    let _ = child.kill();
    let _ = child.wait();
    drop(child);
    drop(secret);
    err
}

/// Build the remote shell command sent to `ssh` for a saved host.
///
/// The persisted cwd is user-controlled and reaches a remote `/bin/sh -c`, so
/// it is POSIX single-quoted with [`shell_single_quote`] — never Rust debug
/// formatting, whose double quotes do not suppress `$()`, backticks, `;` or
/// newlines. An empty or `~` path keeps the login shell's own default cwd.
pub(crate) fn build_remote_command(integration: &str, path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "~" {
        format!("{integration} exec \"$SHELL\" -l")
    } else {
        format!(
            "{integration} cd {} && exec \"$SHELL\" -l",
            shell_single_quote(trimmed)
        )
    }
}

// PTY programs need the same cwd-independent authority as short helpers, but
// their interactive process custody stays with the PTY owner.
pub(crate) fn resolve_executable(bin: &str) -> Option<PathBuf> {
    resolve_executable_in(bin, &std::env::var_os("PATH")?)
}

pub(crate) fn resolve_executable_in(bin: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let program = std::path::Path::new(bin);
    if program.is_absolute() {
        return is_executable_file(program).then(|| program.to_owned());
    }
    if program.components().count() != 1 || bin.is_empty() {
        return None;
    }
    if std::env::split_paths(path).any(|dir| !dir.is_absolute()) {
        return None;
    }
    std::env::split_paths(path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable_file(candidate))
}

/// A regular file (after following symlinks) with an execute bit for us.
#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

/// The substring `sshpass -P` must find in `ssh`'s prompt for the secret this
/// host's auth mode delivers. Key-file auth is answered at the OpenSSH key
/// prompt `Enter passphrase for key '…':`, which contains no `password`, so
/// the default `assword` indicator would never match and the passphrase would
/// never be sent; password auth is answered at `…'s password:`.
fn sshpass_prompt(auth: &SshAuthMode) -> &'static str {
    match auth {
        SshAuthMode::KeyFile { .. } => "passphrase",
        _ => "assword",
    }
}

/// Push the shared `ssh` options (excluding the trailing `-- <destination>
/// <remote_cmd>`) onto `c`. Used by both the direct and sshpass branches so
/// they cannot drift.
fn push_ssh_options(c: &mut CommandBuilder, host: &SshHostRecord) {
    c.arg("-t");
    // Host-key acceptance stays with the user: `ask` on the command line wins
    // over any weaker `~/.ssh/config` value, so an unknown key is shown as a
    // fingerprint on this pty and written to `known_hosts` before the session
    // continues, and a changed key ends the connection. Auto-acceptance would
    // hand a sshpass-delivered secret to whoever answered first.
    c.arg("-o");
    c.arg("StrictHostKeyChecking=ask");
    if let Some(kp) = host.key_path()
        && !kp.is_empty()
    {
        c.arg("-i");
        c.arg(kp);
    }
    if host.port != 22 {
        c.arg("-p");
        c.arg(host.port.to_string());
    } else {
        c.arg("-o");
        c.arg("Port=22");
    }
    if !host.username.is_empty() {
        c.arg("-l");
        c.arg(&host.username);
    }
}

/// Construct the `ssh`/`sshpass` command for a saved host through the pure
/// seams the tests exercise, returning the command and an owned password
/// channel (a retained anonymous sealed `memfd`) that must outlive the child's
/// open of it. The helper `resolve`r is injected so a test can supply a
/// synthetic `PATH` resolution without mutating process env; production passes
/// [`resolve_executable`].
///
/// Hardening applied here: the destination is preceded by a literal `--` so a
/// destination such as `-oProxyCommand=…` cannot be parsed as an option; `ssh`
/// is resolved once to an absolute executable that is both the direct program
/// and the command argument passed to sshpass (a missing `ssh` is a typed
/// failure, never a basename fallback); the secret is delivered by pointing
/// `sshpass -f` at `/proc/<our-pid>/fd/<n>`, the magic path naming this
/// process's retained sealed `memfd` — never `-p`/argv or the environment, and
/// never a filesystem pathname; and `sshpass -P` is set to the prompt this
/// host's auth mode answers so a key-file passphrase is matched at the
/// passphrase prompt rather than silently dropped.
pub(crate) fn build_ssh_command_with<R>(
    host: &SshHostRecord,
    password: Option<&str>,
    resolve: R,
) -> Result<(CommandBuilder, PasswordChannel), PtyError>
where
    R: Fn(&str) -> Option<PathBuf>,
{
    let integration = format!(
        "export PROMPT_COMMAND={}; export TERM_PROGRAM=enzui-terminal;",
        shell_single_quote(BASH_PROMPT_COMMAND)
    );
    let remote_cmd = build_remote_command(&integration, &host.path);

    let password = password.unwrap_or("");

    // Resolve `ssh` to an absolute executable exactly once, before branch
    // selection. This same PathBuf is the direct program and the command
    // argument handed to sshpass, so neither `CommandBuilder` nor sshpass can
    // re-resolve a bare name against a poisoned PATH at spawn time. A missing
    // `ssh` is a typed setup failure — never a basename fallback.
    let ssh_path = resolve("ssh")
        .filter(|path| path.is_absolute())
        .ok_or_else(|| PtyError::Spawn("no executable `ssh` found on PATH".to_owned()))?;

    // Resolve sshpass to an absolute executable exactly once too; the same
    // PathBuf decides availability and is the program launched. The secure
    // anonymous password channel is a Linux `memfd`, so on any other OS sshpass
    // password delivery is unavailable and the direct branch is taken.
    #[cfg(target_os = "linux")]
    let sshpass = (!password.is_empty())
        .then(|| resolve("sshpass"))
        .flatten()
        .filter(|path| path.is_absolute());
    #[cfg(not(target_os = "linux"))]
    let sshpass: Option<PathBuf> = None;

    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut channel: PasswordChannel = None;
    let cmd = if let Some(sshpass_path) = sshpass.as_ref() {
        let mut c = CommandBuilder::new(sshpass_path);
        #[cfg(target_os = "linux")]
        {
            let secret = PasswordSecret::create(password)
                .map_err(|err| PtyError::Spawn(format!("ssh password channel: {err}")))?;
            // A non-secret magic path naming this process's retained sealed
            // `memfd`; sshpass re-opens it read-only after portable-pty closes
            // every inherited fd >= 3, so no descriptor inheritance is claimed.
            c.arg("-f");
            c.arg(secret.proc_path());
            // Match the prompt for this host's auth mode: a key-file profile's
            // secret answers the passphrase prompt, not the password one, so
            // sshpass must search for the right indicator or send nothing.
            c.arg("-P");
            c.arg(sshpass_prompt(&host.auth));
            channel = Some(secret);
        }
        // The resolved absolute `ssh`, not the basename "ssh": sshpass execs
        // its command argument, and a bare name would let it re-resolve PATH.
        c.arg(&ssh_path);
        push_ssh_options(&mut c, host);
        c.arg("--");
        c.arg(&host.destination);
        c.arg(&remote_cmd);
        c
    } else {
        let mut c = CommandBuilder::new(&ssh_path);
        push_ssh_options(&mut c, host);
        c.arg("--");
        c.arg(&host.destination);
        c.arg(&remote_cmd);
        c
    };
    Ok((cmd, channel))
}

/// An anonymous sealed `memfd` used to hand the SSH password to `sshpass -f`
/// without ever placing it in argv, the environment, or any filesystem
/// pathname.
///
/// The secret bytes live in an anonymous kernel memory object created by
/// `memfd_create` — there is no directory entry, no runtime dir, no `mkfifo`,
/// and therefore no stat/unlink race or on-disk cleanup to get wrong. The fd
/// is created `CLOEXEC | ALLOW_SEALING | NOEXEC_SEAL` where supported;
/// `"<password>\n"` is written, the offset rewound, and the object sealed
/// `WRITE | GROW | SHRINK | SEAL` (plus the kernel's `EXEC` seal on the
/// NOEXEC path). The kernel-visible object name is deliberately neutral and
/// carries neither a credential nor its purpose.
///
/// `sshpass` cannot receive the descriptor by inheritance: portable-pty's
/// `close_random_fds` closes every inherited fd >= 3 in the child before exec,
/// so an inherited `sshpass -d <fd>` would be unusable. Instead this process
/// RETAINS the descriptor and hands sshpass the magic path
/// `/proc/<our-pid>/fd/<n>`, which the child opens to reach the same sealed
/// object. That path names no filesystem object and carries no secret bytes.
///
/// ## Descriptor-number lifetime
///
/// `n` is an ordinary descriptor number in this process, and sshpass opens the
/// path lazily — potentially long after exec. The number must therefore stay
/// bound to this memfd for as long as any process that might open it exists;
/// otherwise `n` is freed for reuse and a late open resolves to whatever
/// unrelated file or socket next claimed it. `PtySession::shutdown` releases
/// this value only after every owned process is killed and both workers have
/// joined (a session with no worker threads has no such child and may release
/// immediately).
///
/// ## What `/proc/<pid>/fd` does and does not protect
///
/// Access to another process's `/proc/<pid>/fd` is governed by a ptrace-mode
/// access check, not by file permissions alone:
///
/// * Running as the same uid is NOT sufficient on its own. The kernel applies
///   `PTRACE_MODE_READ` credential checks plus the target's dumpability, and an
///   LSM (SELinux, AppArmor, Yama) may deny an otherwise-permitted same-uid
///   peer. A process that has dropped dumpability, or is confined by policy,
///   can be refused. This is why the exact path is preflighted below rather
///   than assumed to work: an unreadable path would otherwise surface as an
///   sshpass hang.
/// * Conversely it is NOT a confidentiality boundary against an observer that
///   satisfies the ptrace-mode check: a process with `CAP_SYS_PTRACE` in the
///   relevant user namespace, and in the usual configuration a same-uid peer,
///   can open the path and read the secret repeatedly for as long as this value
///   is retained. The window is bounded by that retention, not closed by it.
///   Whether any particular privileged or same-uid peer succeeds depends on the
///   ptrace-mode check, dumpability and LSM policy, so no blanket claim is made
///   either that such access always succeeds or that it is always denied.
/// * `hidepid` is a mount option on `/proc` that restricts visibility of other
///   users' process directories; it is not what grants sshpass access here, and
///   no claim about a specific `hidepid` value is made.
/// * The backing store is shmem, so its pages may be swapped to disk under
///   memory pressure like any other anonymous memory; a swap device without
///   encryption can therefore retain the bytes outside this process's control.
/// * If this process exits abruptly the descriptor is closed by the kernel and
///   the object freed, but the pid may later be reused by an unrelated process;
///   a path captured earlier then names a different process's descriptor table.
///   Nothing here should be persisted or reused across process lifetimes.
///
/// ## Sealing
///
/// `F_SEAL_WRITE` freezes the contents: the bytes written before sealing (the
/// payload) remain, and no further write can modify them. It does not make the
/// magic path unopenable for writing — a re-open can still succeed; it is the
/// subsequent write that fails with `EPERM`. Adding the seal is accepted here
/// because no shared writable mapping exists: `F_SEAL_WRITE` fails `EBUSY` only
/// while such a mapping is outstanding, not merely because this process still
/// holds a read-write descriptor. `F_SEAL_SEAL` then prevents any further seal
/// from being added.
///
/// ## `vm.memfd_noexec`
///
/// Under the strict setting (`vm.memfd_noexec=2`) the kernel rejects a
/// `memfd_create` that omits `MFD_NOEXEC_SEAL`. Linux before 6.3 does not know
/// that flag and returns `EINVAL`. Creation therefore tries NOEXEC first and
/// falls back to the base flags ONLY for that exact `EINVAL`; every other first
/// error is returned unchanged, and any fallback error (including strict-mode
/// refusal) remains a typed failure. This supports old kernels without turning
/// policy or resource failures into a weaker channel.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub(crate) struct PasswordSecret {
    /// The retained sealed `memfd`. Held for the session lifetime so both the
    /// object AND its descriptor NUMBER stay reserved until sshpass can no
    /// longer open the path; its `Drop` (an `OwnedFd` close) releases both.
    pub(crate) _memfd: std::os::fd::OwnedFd,
    /// The non-secret magic path handed to `sshpass -f`: this process's own
    /// descriptor named by number under its own `/proc` entry. Contains no
    /// secret bytes and names no filesystem object.
    proc_path: PathBuf,
    /// Whether creation succeeded with `MFD_NOEXEC_SEAL` rather than the
    /// EINVAL-only compatibility fallback for pre-6.3 kernels.
    pub(crate) _used_noexec_seal: bool,
    #[cfg(test)]
    pub(crate) identity: MemfdIdentity,
    #[cfg(test)]
    pub(crate) release_probe: Option<SecretReleaseProbe>,
}

#[cfg(target_os = "linux")]
impl Drop for PasswordSecret {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(probe) = self.release_probe.as_ref() {
            let child = *probe
                .child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let exact_child_gone = child.is_none_or(|(pid, start_ticks)| {
                // A zombie still has the exact identity and has NOT been
                // reaped, so only absence or pid reuse counts as gone.
                !read_process_stat(pid).is_ok_and(|stat| stat.start_ticks == start_ticks)
            });
            probe
                .observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(SecretReleaseObservation {
                    secret: self.identity,
                    child,
                    exact_child_gone,
                });
        }
        // `_memfd` is still open while the observation above is made; Rust
        // drops fields only after this method returns. Thus the event means
        // "the exact inode is ABOUT TO be released", not merely "both facts
        // were absent at some later point".
    }
}

#[cfg(target_os = "linux")]
impl PasswordSecret {
    /// The largest password accepted into the channel. A bound applied BEFORE
    /// the object is created, so an unbounded secret is never written; well
    /// under any resource concern for an anonymous in-memory object.
    pub(crate) const MAX_PASSWORD_LEN: usize = 4096;

    /// Create the sealed `memfd`, write `"<password>\n"`, seal it, and record
    /// the `/proc/<pid>/fd/<n>` path that names it.
    pub(crate) fn create(password: &str) -> std::io::Result<Self> {
        Self::create_with(
            password,
            |flags| rustix::fs::memfd_create("enzui-pty-data", flags).map_err(std::io::Error::from),
            Self::proc_path_for,
        )
    }

    /// The production derivation of the magic path: this process's own pid and
    /// the retained descriptor's own number.
    pub(crate) fn proc_path_for(memfd: &std::os::fd::OwnedFd) -> PathBuf {
        PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            memfd.as_raw_fd()
        ))
    }

    /// `create` with the `memfd_create` and path-derivation seams injected, so
    /// a fixture can force a setup failure or hand over a path that does NOT
    /// name the retained object and prove the preflight refuses it. Production
    /// passes the real `memfd_create` and `proc_path_for`.
    pub(crate) fn create_with<M, P>(
        password: &str,
        make_memfd: M,
        derive_proc_path: P,
    ) -> std::io::Result<Self>
    where
        M: FnMut(rustix::fs::MemfdFlags) -> std::io::Result<std::os::fd::OwnedFd>,
        P: FnOnce(&std::os::fd::OwnedFd) -> PathBuf,
    {
        use std::io::{Seek, Write as _};
        // Bound the secret BEFORE creating or writing anything: an oversized
        // secret is a typed refusal, not a silently truncated or blocking
        // write.
        if password.len() >= Self::MAX_PASSWORD_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "ssh password exceeds the delivery limit",
            ));
        }
        // CLOEXEC so neither the child nor any unrelated exec inherits the
        // descriptor; ALLOW_SEALING so the contents/length can be frozen below.
        // Prefer NOEXEC_SEAL. Only EINVAL means "this kernel predates that
        // flag" and permits one compatibility retry; policy/resource errors
        // never trigger a weaker retry.
        let base_flags = rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING;
        let preferred_flags = base_flags | rustix::fs::MemfdFlags::NOEXEC_SEAL;
        let mut make_memfd = make_memfd;
        let (memfd, used_noexec_seal) = match make_memfd(preferred_flags) {
            Ok(memfd) => (memfd, true),
            Err(err)
                if err.raw_os_error()
                    == std::io::Error::from(rustix::io::Errno::INVAL).raw_os_error() =>
            {
                (make_memfd(base_flags)?, false)
            }
            Err(err) => return Err(err),
        };
        let file = std::fs::File::from(memfd);
        // Write "<password>\n" then rewind to the first byte.
        //
        // Every fallible step from here to the preflight runs inside the block
        // below so that `payload.fill(0)` executes on EVERY exit path — write,
        // seek, seal, and preflight errors included — before the error is
        // propagated. The clear is best effort for this local temporary only:
        // it is not guaranteed secure zeroization (the optimiser may elide it),
        // and the caller's persisted password owner is a separate copy left
        // intact by design.
        let mut payload = Vec::with_capacity(password.len() + 1);
        payload.extend_from_slice(password.as_bytes());
        payload.push(b'\n');
        let staged = {
            let bytes: &[u8] = &payload;
            (move || -> std::io::Result<(std::os::fd::OwnedFd, PathBuf)> {
                let mut file = file;
                file.write_all(bytes)?;
                file.seek(std::io::SeekFrom::Start(0))?;
                // Freeze the object: no future write, grow or shrink, and no
                // further change to the seal set itself.
                rustix::fs::fcntl_add_seals(
                    &file,
                    rustix::fs::SealFlags::WRITE
                        | rustix::fs::SealFlags::GROW
                        | rustix::fs::SealFlags::SHRINK
                        | rustix::fs::SealFlags::SEAL,
                )
                .map_err(std::io::Error::from)?;
                let memfd = std::os::fd::OwnedFd::from(file);
                let proc_path = derive_proc_path(&memfd);
                // PREFLIGHT the exact path that will be handed to sshpass,
                // before the child is ever spawned. Everything that would
                // otherwise turn into an sshpass hang or a wrong-object read
                // becomes a typed setup failure here.
                Self::preflight(&proc_path, &memfd, bytes)?;
                Ok((memfd, proc_path))
            })()
        };
        payload.fill(0);
        let (memfd, proc_path) = staged?;
        #[cfg(test)]
        let identity = {
            let stat = rustix::fs::fstat(&memfd).map_err(std::io::Error::from)?;
            MemfdIdentity {
                device: stat.st_dev as u64,
                inode: stat.st_ino as u64,
            }
        };

        Ok(Self {
            _memfd: memfd,
            proc_path,
            _used_noexec_seal: used_noexec_seal,
            #[cfg(test)]
            identity,
            #[cfg(test)]
            release_probe: None,
        })
    }

    /// Independently open `proc_path`, prove it names the very same object as
    /// the retained `memfd`, and prove it yields exactly `expected`.
    ///
    /// SCOPE, deliberately narrow: this open is performed by THIS process, so
    /// it proves only that the path, the object identity and the contents are
    /// correct as seen from the parent. It is NOT a proof that sshpass will be
    /// able to open the path. The child crosses an `exec`, which can change
    /// credentials (set-user-ID/set-group-ID binaries) and can trigger an LSM
    /// domain transition; either can make the same path unreadable to the child
    /// even though the parent just read it. What this check does buy is that
    /// the common, deterministic failures — a missing or masked `/proc`, a
    /// wrong pid, a path that names some other object, or wrong contents —
    /// become typed setup errors here instead of an sshpass that hangs waiting
    /// on a password it can never read.
    pub(crate) fn preflight(
        proc_path: &std::path::Path,
        memfd: &std::os::fd::OwnedFd,
        expected: &[u8],
    ) -> std::io::Result<()> {
        use std::io::Read as _;
        // A fresh open through the magic path: this is the operation sshpass
        // will perform, so its failure modes are surfaced now.
        let mut opened = std::fs::File::open(proc_path).map_err(|err| {
            std::io::Error::new(
                err.kind(),
                format!(
                    "ssh password channel path {} is not openable by this process \
                     (missing or masked /proc, or a ptrace-mode/LSM denial): {err}",
                    proc_path.display()
                ),
            )
        })?;
        // Same object, not merely a readable one: compare the (device, inode)
        // of the freshly opened description against the retained descriptor.
        let via_path = rustix::fs::fstat(&opened).map_err(std::io::Error::from)?;
        let retained = rustix::fs::fstat(memfd).map_err(std::io::Error::from)?;
        if via_path.st_dev != retained.st_dev || via_path.st_ino != retained.st_ino {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "ssh password channel path {} does not name the retained memfd",
                    proc_path.display()
                ),
            ));
        }
        // Exactly the payload, and nothing beyond it. The read is bounded by
        // one byte more than expected so a longer object is detected without
        // reading an unbounded amount. `seen` holds the secret, so it is
        // cleared (best effort) BEFORE either the read error or the mismatch is
        // propagated.
        let mut seen = Vec::with_capacity(expected.len() + 1);
        // `File` implements both `Read` and `Write`, so `by_ref` is ambiguous
        // here and must name the trait explicitly.
        let read_result = std::io::Read::by_ref(&mut opened)
            .take(expected.len() as u64 + 1)
            .read_to_end(&mut seen);
        let matches = read_result.is_ok() && seen == expected;
        seen.fill(0);
        read_result?;
        if !matches {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "ssh password channel path {} did not yield the expected payload",
                    proc_path.display()
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn proc_path(&self) -> &std::path::Path {
        &self.proc_path
    }
}
