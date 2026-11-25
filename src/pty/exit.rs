//! Child exits and working directory resolution.

use super::*;

/// How one child terminated: prose for the pane's exit line, plus the numeric
/// status callers branch on.
///
/// `code` is `None` for a signal death and for an unavailable status, so a
/// terminating signal is never read as the numerically equal exit status.
/// `detail` carries no native stderr, path or host bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChildExit {
    pub detail: String,
    pub code: Option<i32>,
}

#[cfg(target_os = "linux")]
pub(crate) fn describe_child_exit(status: &rustix::process::WaitIdStatus) -> ChildExit {
    if let Some(code) = status.exit_status() {
        return ChildExit {
            detail: format!("exit code {code}"),
            code: Some(code),
        };
    }
    if let Some(signal) = status.terminating_signal() {
        let core = if status.dumped() { ", core dumped" } else { "" };
        return ChildExit {
            detail: format!("terminated by signal {signal}{core}"),
            code: None,
        };
    }
    ChildExit {
        detail: "process status unavailable: neither an exit code nor a terminating signal".into(),
        code: None,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn open_pidfd(pid: u32) -> std::io::Result<rustix::fd::OwnedFd> {
    let raw_pid = i32::try_from(pid)
        .map_err(|_| std::io::Error::other("PTY process id does not fit pid_t"))?;
    let pid = rustix::process::Pid::from_raw(raw_pid)
        .ok_or_else(|| std::io::Error::other("PTY process id cannot be zero"))?;
    rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
        .map_err(std::io::Error::other)
}

#[cfg(target_os = "linux")]
pub(crate) fn parse_process_stat(stat: &str) -> std::io::Result<LinuxProcStat> {
    let close = stat
        .rfind(')')
        .ok_or_else(|| std::io::Error::other("malformed process stat"))?;
    let fields = stat[close + 1..].split_whitespace().collect::<Vec<_>>();
    let field = |index: usize, name: &str| {
        fields
            .get(index)
            .copied()
            .ok_or_else(|| std::io::Error::other(format!("process stat has no {name}")))
    };
    Ok(LinuxProcStat {
        state: field(0, "state")?.as_bytes()[0],
        parent: field(1, "parent")?.parse().map_err(std::io::Error::other)?,
        process_group: field(2, "process group")?
            .parse()
            .map_err(std::io::Error::other)?,
        session: field(3, "session")?
            .parse()
            .map_err(std::io::Error::other)?,
        start_ticks: field(19, "start time")?
            .parse()
            .map_err(std::io::Error::other)?,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn read_process_stat(pid: u32) -> std::io::Result<LinuxProcStat> {
    parse_process_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat"))?)
}

#[cfg(target_os = "linux")]
pub(crate) fn process_start_ticks(pid: u32) -> std::io::Result<u64> {
    Ok(read_process_stat(pid)?.start_ticks)
}

/// Whether closing the PTY ends `pid`: the leader, and members of its session
/// in the leader's or the foreground process group that do not ignore SIGHUP.
///
/// Background jobs follow the shell's own hangup rules, so disowned jobs
/// survive, as do `nohup` commands and processes that left with `setsid()`.
#[cfg(target_os = "linux")]
pub(super) fn closes_with_pty(
    pid: u32,
    leader: u32,
    foreground: Option<u32>,
    snapshot: &HashMap<u32, LinuxProcStat>,
) -> bool {
    let Some(candidate) = snapshot.get(&pid) else {
        return false;
    };
    pid == leader
        || (candidate.session == leader
            && (candidate.process_group == leader || Some(candidate.process_group) == foreground)
            && !ignores_hangup(pid))
}

/// An unreadable mask counts as not ignoring, so the process still closes.
#[cfg(target_os = "linux")]
fn ignores_hangup(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/status")).is_ok_and(|status| {
        status
            .lines()
            .find_map(|line| line.strip_prefix("SigIgn:"))
            .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
            .is_some_and(|mask| mask & (1 << (rustix::process::Signal::HUP.as_raw() - 1)) != 0)
    })
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn process_has_open_path(pid: u32, wanted: &std::path::Path) -> bool {
    std::fs::read_dir(format!("/proc/{pid}/fd")).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|entry| std::fs::read_link(entry.path()).is_ok_and(|path| path == wanted))
    })
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct WorkerWitnesses {
    pub(crate) reader_done: Arc<AtomicBool>,
    pub(crate) exit_done: Arc<AtomicBool>,
    pub(crate) reader_joined: Arc<AtomicBool>,
    pub(crate) exit_joined: Arc<AtomicBool>,
    pub(crate) writer_joined: Arc<AtomicBool>,
    pub(crate) final_freeze_scans: Arc<AtomicU64>,
    pub(crate) final_freeze_stable: Arc<AtomicBool>,
}

#[cfg(test)]
impl WorkerWitnesses {
    pub(crate) fn reader_completed(&self) -> bool {
        self.reader_done.load(Ordering::Acquire)
    }

    pub(crate) fn exit_waiter_completed(&self) -> bool {
        self.exit_done.load(Ordering::Acquire)
    }

    pub(crate) fn reader_joined(&self) -> bool {
        self.reader_joined.load(Ordering::Acquire)
    }

    pub(crate) fn exit_waiter_joined(&self) -> bool {
        self.exit_joined.load(Ordering::Acquire)
    }

    pub(crate) fn input_writer_joined(&self) -> bool {
        self.writer_joined.load(Ordering::Acquire)
    }

    pub(crate) fn final_freeze_scans(&self) -> u64 {
        self.final_freeze_scans.load(Ordering::Acquire)
    }

    /// `true` only when the final pre-KILL sweep observed two consecutive empty
    /// snapshots. A sweep that merely ran out of deadline reports `false`.
    pub(crate) fn final_freeze_reached_stability(&self) -> bool {
        self.final_freeze_stable.load(Ordering::Acquire)
    }
}

/// Result of resolving a requested local-shell cwd.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalCwdResolution {
    pub requested: Option<PathBuf>,
    pub effective: PathBuf,
    pub warning: Option<String>,
}

// File-URI locators preserve path bytes that cannot be displayed safely.
pub(crate) fn resolve_local_cwd(requested: Option<&str>) -> LocalCwdResolution {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let raw = requested.map(str::trim).filter(|s| !s.is_empty());
    let requested_path = raw.and_then(|value| {
        if value.starts_with("file://") {
            return crate::shell_history::cwd::local_path(value);
        }
        Some(if value == "~" {
            home.clone().unwrap_or_else(|| PathBuf::from("/"))
        } else if let Some(rest) = value.strip_prefix("~/") {
            home.clone()
                .unwrap_or_else(|| PathBuf::from("/"))
                .join(rest)
        } else {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                home.clone()
                    .unwrap_or_else(|| PathBuf::from("/"))
                    .join(path)
            }
        })
    });
    let fallback = home
        .filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok().filter(|p| p.is_dir()))
        .unwrap_or_else(|| PathBuf::from("/"));
    match requested_path {
        Some(path) if path.is_dir() => LocalCwdResolution {
            requested: Some(path.clone()),
            effective: path,
            warning: None,
        },
        Some(path) => LocalCwdResolution {
            requested: Some(path.clone()),
            effective: fallback.clone(),
            warning: Some(crate::i18n::format(
                crate::i18n::localizer(),
                "Saved working directory {} is unavailable; started in {}",
                &[
                    ("arg0", &(crate::shell_history::cwd::path_label(&path))),
                    ("arg1", &(crate::shell_history::cwd::path_label(&fallback))),
                ],
            )),
        },
        None => LocalCwdResolution {
            requested: None,
            effective: fallback,
            warning: raw.filter(|value| value.starts_with("file://")).map(|_| {
                "Saved working directory is invalid; started in the default directory.".into()
            }),
        },
    }
}
