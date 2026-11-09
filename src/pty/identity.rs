//! Linux process identity and presence.

use super::*;

#[cfg(target_os = "linux")]
pub(crate) struct LinuxProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) start_ticks: u64,
    #[cfg(test)]
    pub(crate) tty_name: Option<PathBuf>,
    pub(super) capability: Option<PtyCapability>,
    // The waiter deliberately leaves this leader waitable until shutdown has
    // finished enumerating its session. That pins the numeric SID while every
    // member is converted into its own stable pidfd.
    pub(crate) pidfd: Arc<rustix::fd::OwnedFd>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LeaderIdentityState {
    Original,
    Gone,
    Replaced,
    /// The leader's `/proc` entry could not be read. This proves nothing: it is
    /// neither exit nor recycling, so it stays debt and its exact pidfd keeps
    /// receiving signals.
    Unknown,
}

/// What a `/proc` read proved about one exact process.
///
/// A read failure is never evidence of exit. `Unknown` remains owed, keeps its
/// exact pidfd signalled, and can never be counted as an empty scan.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Presence {
    Present,
    Gone,
    Unknown,
}

#[cfg(target_os = "linux")]
impl Presence {
    /// Whether this process is still owed: present, or not proved gone.
    pub(crate) const fn is_owed(self) -> bool {
        !matches!(self, Self::Gone)
    }
}

/// Whether one process is provably parked or gone, which is the only thing a
/// freeze scan may count toward stability.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FreezeState {
    Settled,
    Running,
    Unknown,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LinuxProcStat {
    pub(crate) state: u8,
    pub(crate) parent: u32,
    pub(crate) process_group: u32,
    pub(crate) session: u32,
    pub(crate) start_ticks: u64,
}

#[cfg(target_os = "linux")]
pub(crate) struct OwnedSessionMember {
    pub(crate) pid: u32,
    pub(crate) start_ticks: u64,
    pub(crate) pidfd: rustix::fd::OwnedFd,
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PtySetupFailure {
    Mkfifo,
    Open,
    Metadata,
    MasterDup,
    CancelPipe,
    Spawn,
    /// Injected AFTER the child is spawned, at `take_writer`.
    TakeWriter,
    /// Injected AFTER the child is spawned, at `try_clone_reader`.
    PostSpawnCloneReader,
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Default)]
pub(crate) struct PtySpawnTestControl {
    pub(crate) capability_path: Option<PathBuf>,
    pub(crate) path_was_visible: Option<Arc<AtomicBool>>,
    pub(crate) setup_failure: Option<PtySetupFailure>,
    pub(crate) fail_capability_handshake: bool,
    pub(crate) delay_capability_handshake: bool,
    /// Records the pid of the child actually spawned.
    pub(crate) spawned_child_pid: Option<Arc<Mutex<Option<u32>>>>,
    /// Attached to the exact password object before spawn. Its Drop event runs
    /// while that exact memfd is still open and observes whether the exact
    /// spawned child identity has already disappeared.
    pub(crate) secret_release_probe: Option<SecretReleaseProbe>,
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MemfdIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SecretReleaseObservation {
    pub(crate) secret: MemfdIdentity,
    pub(crate) child: Option<(u32, u64)>,
    pub(crate) exact_child_gone: bool,
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Clone, Debug, Default)]
pub(crate) struct SecretReleaseProbe {
    pub(crate) child: Arc<Mutex<Option<(u32, u64)>>>,
    pub(crate) observations: Arc<Mutex<Vec<SecretReleaseObservation>>>,
}

#[cfg(all(test, target_os = "linux"))]
impl SecretReleaseProbe {
    pub(crate) fn set_child(&self, pid: u32, start_ticks: u64) {
        *self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((pid, start_ticks));
    }

    pub(crate) fn observations(&self) -> Vec<SecretReleaseObservation> {
        self.observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[cfg(target_os = "linux")]
pub(crate) struct LinuxReaderControl {
    /// A `dup` of the EXACT pty master open file description. Re-opening the
    /// master's `/proc/self/fd` magic link is NOT a duplicate: that path names
    /// `/dev/ptmx`, so opening it allocates a fresh, unrelated pty whose
    /// readiness has nothing to do with this session's output.
    pub(crate) master: filedescriptor::FileDescriptor,
    pub(crate) cancel: filedescriptor::FileDescriptor,
}

/// A pty master file descriptor owned elsewhere (by `PtyPair`), borrowed only
/// to duplicate it. `AsRawFd` is a safe trait, so this needs no `unsafe` — and
/// this crate denies `unsafe_code`.
#[cfg(target_os = "linux")]
pub(crate) struct BorrowedMasterFd(pub(crate) RawFd);

#[cfg(target_os = "linux")]
impl AsRawFd for BorrowedMasterFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

#[cfg(target_os = "linux")]
impl OwnedSessionMember {
    pub(crate) fn presence(&self) -> Presence {
        presence_of(self.pid, self.start_ticks)
    }

    pub(crate) fn signal(&self, signal: rustix::process::Signal) {
        let _ = rustix::process::pidfd_send_signal(&self.pidfd, signal);
    }

    pub(crate) fn freeze_state(&self) -> FreezeState {
        freeze_state_of(self.pid, self.start_ticks)
    }
}

/// Only an exact `NotFound`, a replaced start time or a reaped-pending zombie
/// is gone. Every other read failure is [`Presence::Unknown`].
#[cfg(target_os = "linux")]
pub(crate) fn presence_of(pid: u32, start_ticks: u64) -> Presence {
    match read_process_stat(pid) {
        Ok(stat) if stat.start_ticks != start_ticks || stat.state == b'Z' => Presence::Gone,
        Ok(_) => Presence::Present,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Presence::Gone,
        Err(_) => Presence::Unknown,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn freeze_state_of(pid: u32, start_ticks: u64) -> FreezeState {
    match read_process_stat(pid) {
        Ok(stat) => {
            if stat.start_ticks != start_ticks || matches!(stat.state, b'T' | b't' | b'Z') {
                FreezeState::Settled
            } else {
                FreezeState::Running
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => FreezeState::Settled,
        Err(_) => FreezeState::Unknown,
    }
}

#[cfg(target_os = "linux")]
impl LinuxProcessIdentity {
    pub(super) fn capture(
        pid: u32,
        #[cfg(test)] tty_name: Option<PathBuf>,
        capability: Option<PtyCapability>,
    ) -> std::io::Result<Self> {
        let before = read_process_stat(pid)?;
        let pidfd = open_pidfd(pid)?;
        let after = read_process_stat(pid)?;
        if before.start_ticks != after.start_ticks {
            return Err(std::io::Error::other(
                "PTY process changed while its identity was captured",
            ));
        }
        Ok(Self {
            pid,
            start_ticks: after.start_ticks,
            #[cfg(test)]
            tty_name,
            capability,
            pidfd: Arc::new(pidfd),
        })
    }

    pub(crate) fn state(&self) -> LeaderIdentityState {
        self.state_with(process_start_ticks)
    }

    pub(crate) fn state_with<F>(&self, read_start_ticks: F) -> LeaderIdentityState
    where
        F: FnOnce(u32) -> std::io::Result<u64>,
    {
        match read_start_ticks(self.pid) {
            Ok(current) if current == self.start_ticks => LeaderIdentityState::Original,
            Ok(_) => LeaderIdentityState::Replaced,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => LeaderIdentityState::Gone,
            Err(_) => LeaderIdentityState::Unknown,
        }
    }

    /// Whether this exact leader is provably no longer running under its
    /// captured identity. An unreadable leader is not.
    pub(crate) fn leader_departed(&self) -> bool {
        matches!(
            self.state(),
            LeaderIdentityState::Gone | LeaderIdentityState::Replaced
        )
    }

    pub(crate) fn leader_freeze_state(&self) -> FreezeState {
        freeze_state_of(self.pid, self.start_ticks)
    }

    /// Move this identity into a new value, leaving the original inert.
    ///
    /// The pidfd is already shared, so the exact leader handle survives in both
    /// values and is not closed by the drained one. The capability descriptor
    /// moves, because only one holder may use it to discover members.
    pub(crate) fn transfer(&mut self) -> Self {
        Self {
            pid: self.pid,
            start_ticks: self.start_ticks,
            #[cfg(test)]
            tty_name: self.tty_name.take(),
            capability: self.capability.take(),
            pidfd: Arc::clone(&self.pidfd),
        }
    }

    /// Add every live process still provably owned by this PTY. Ordinary jobs
    /// are owned by the pinned session id or a parent chain rooted at the
    /// retained leader. A reparented `setsid()` descendant — including one that
    /// scrubbed its environment with `env -i` — is owned when it still holds the
    /// inherited capability descriptor. Every candidate is read, opened as a
    /// pidfd, and read again before its stable handle is retained across later
    /// rescans.
    pub(crate) fn refresh_owned_processes(
        &self,
        owned: &mut Vec<OwnedSessionMember>,
    ) -> std::io::Result<usize> {
        self.admit_discovery()?;
        let mut snapshot = HashMap::new();
        for entry in std::fs::read_dir("/proc")?.filter_map(Result::ok) {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if let Ok(stat) = read_process_stat(pid) {
                snapshot.insert(pid, stat);
            }
        }
        let mut discovered = Vec::new();
        for (&pid, before) in &snapshot {
            if before.state == b'Z'
                || owned
                    .iter()
                    .any(|member| member.pid == pid && member.start_ticks == before.start_ticks)
                || !process_belongs_to_pty(pid, self.pid, self.capability.as_ref(), &snapshot)
            {
                continue;
            }
            let Ok(pidfd) = open_pidfd(pid) else {
                continue;
            };
            let Ok(after) = read_process_stat(pid) else {
                continue;
            };
            if before.start_ticks == after.start_ticks
                && after.state != b'Z'
                && process_belongs_to_pty(pid, self.pid, self.capability.as_ref(), &snapshot)
            {
                discovered.push(OwnedSessionMember {
                    pid,
                    start_ticks: after.start_ticks,
                    pidfd,
                });
            }
        }
        // Never grow the ownership set after an unexpected leader reap frees
        // its numeric SID. Already captured pidfds remain exact and safe.
        self.admit_discovery()?;
        let added = discovered.len();
        owned.extend(discovered);
        owned.sort_by_key(|member| member.pid == self.pid);
        Ok(added)
    }

    pub(crate) fn signal_session_with<F>(
        &self,
        initial_state: LeaderIdentityState,
        include_leader: bool,
        owned: &mut Vec<OwnedSessionMember>,
        mut signal: F,
    ) -> std::io::Result<usize>
    where
        F: FnMut(&OwnedSessionMember) -> bool,
    {
        // Discovery needs an intact numeric SID, but signalling a retained
        // pidfd is exact whatever the leader now reads as. An unknown or
        // replaced leader therefore still gets every captured member signalled.
        if initial_state == LeaderIdentityState::Original {
            let _ = self.refresh_owned_processes(owned);
        }
        let mut signalled = 0;
        for member in owned.iter().filter(|member| member.presence().is_owed()) {
            if (include_leader || member.pid != self.pid) && signal(member) {
                signalled += 1;
            }
        }
        Ok(signalled)
    }

    /// Whether new members may be adopted into the ownership set.
    ///
    /// A recycled numeric SID would adopt strangers, so only the exact original
    /// leader admits discovery. An unreadable leader proves nothing, so the
    /// scan reports failure rather than an authoritative empty result.
    pub(crate) fn admit_discovery(&self) -> std::io::Result<()> {
        match self.state() {
            LeaderIdentityState::Original => Ok(()),
            LeaderIdentityState::Gone | LeaderIdentityState::Replaced => {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            }
            LeaderIdentityState::Unknown => Err(std::io::Error::other(
                "PTY leader identity could not be read",
            )),
        }
    }

    pub(crate) fn signal_owned_processes(
        &self,
        owned: &mut Vec<OwnedSessionMember>,
        signal: rustix::process::Signal,
        include_leader: bool,
    ) -> std::io::Result<usize> {
        self.signal_session_with(self.state(), include_leader, owned, |member| {
            rustix::process::pidfd_send_signal(&member.pidfd, signal).is_ok()
        })
    }

    /// Stop every captured member and rescan until two consecutive snapshots
    /// discover nothing new. A leader or child may already be crossing `fork`
    /// when its STOP is sent; the repeated scan closes that final fork window
    /// before callers start the uncatchable KILL phase.
    pub(crate) fn freeze_and_capture_until_stable(
        &self,
        owned: &mut Vec<OwnedSessionMember>,
        deadline: Instant,
    ) -> RescanOutcome {
        let _ = self.signal_leader(rustix::process::Signal::STOP);
        rescan_until_stable(
            deadline,
            || {
                // A scan that could not enumerate `/proc`, or that cannot read
                // some member's state, has proved nothing. It is reported as
                // unstable so it can never increment stability. A leader that
                // is provably gone or replaced ends discovery legitimately.
                let added = match self.refresh_owned_processes(owned) {
                    Ok(added) => added,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(_) => return 1,
                };
                for member in owned.iter().filter(|member| member.presence().is_owed()) {
                    member.signal(rustix::process::Signal::STOP);
                }
                let all_settled = self.leader_freeze_state() == FreezeState::Settled
                    && owned
                        .iter()
                        .all(|member| member.freeze_state() == FreezeState::Settled);
                if all_settled { added } else { added.max(1) }
            },
            || thread::sleep(Duration::from_millis(10)),
        )
    }

    /// Members still owed, which includes any member whose state is unknown.
    pub(crate) fn owed_processes(&self, owned: &[OwnedSessionMember]) -> usize {
        owned
            .iter()
            .filter(|member| member.presence().is_owed())
            .count()
    }

    pub(crate) fn owed_descendants(&self, owned: &[OwnedSessionMember]) -> usize {
        owned
            .iter()
            .filter(|member| member.pid != self.pid && member.presence().is_owed())
            .count()
    }

    pub(crate) fn signal_leader(&self, signal: rustix::process::Signal) -> std::io::Result<()> {
        rustix::process::pidfd_send_signal(&*self.pidfd, signal).map_err(std::io::Error::other)
    }
}

/// Outcome of a stop-and-rescan sweep.
///
/// `reached_stability` is the load-bearing part: a sweep that merely ran out of
/// deadline has NOT proved the fork window closed, and it must not be reported
/// as the same thing as two consecutive empty snapshots.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RescanOutcome {
    pub(crate) scans: u64,
    pub(crate) reached_stability: bool,
}

#[cfg(target_os = "linux")]
pub(crate) fn rescan_until_stable<F, P>(
    deadline: Instant,
    mut scan: F,
    mut pause: P,
) -> RescanOutcome
where
    F: FnMut() -> usize,
    P: FnMut(),
{
    let mut stable_scans = 0;
    let mut scans = 0;
    while Instant::now() < deadline && stable_scans < 2 {
        let added = scan();
        scans += 1;
        stable_scans = if added == 0 { stable_scans + 1 } else { 0 };
        pause();
    }
    RescanOutcome {
        scans,
        reached_stability: stable_scans >= 2,
    }
}
