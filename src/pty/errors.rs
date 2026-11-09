//! PTY failure classes and errors.

use super::*;

/// Stable failure classes at the PTY/process boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PtyFailureKind {
    PtyUnavailable,
    SpawnRefused,
    ReaderFailed,
    Other,
}

impl PtyFailureKind {
    /// Machine-readable value used by the diagnostic channel.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PtyUnavailable => "pty-unavailable",
            Self::SpawnRefused => "spawn-refused",
            Self::ReaderFailed => "reader-failed",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum PtyError {
    OpenPty(String),
    Spawn(String),
    TakeWriter(String),
    CloneReader(String),
    #[cfg(test)]
    WriterPoisoned,
    SessionClosed,
    /// No unique input generation was left, so no session was created: no
    /// child, writer thread or queue was reached. Permanent for the process,
    /// which keeps running the sessions it already has.
    InputAuthorityExhausted,
    /// The bounded input queue would not take this chunk. Nothing was written.
    InputRefused(InputRefusal),
    #[cfg(test)]
    Write(String),
    Resize(String),
    /// The reader thread could not be started. Typically EAGAIN from
    /// `thread::Builder::spawn` under memory pressure — transient, and the
    /// caller's job to report rather than the process's job to die on.
    SpawnReader(String),
    /// No teardown-custody slot was free, so no child was started.
    TeardownCapacity,
    Other(String),
}

impl PtyError {
    /// Compact category for UI translation and structured diagnostics.
    pub(crate) const fn kind(&self) -> PtyFailureKind {
        match self {
            Self::OpenPty(_) => PtyFailureKind::PtyUnavailable,
            Self::Spawn(_) | Self::TeardownCapacity | Self::InputAuthorityExhausted => {
                PtyFailureKind::SpawnRefused
            }
            Self::CloneReader(_) | Self::SpawnReader(_) => PtyFailureKind::ReaderFailed,
            #[cfg(test)]
            Self::WriterPoisoned | Self::Write(_) => PtyFailureKind::Other,
            Self::TakeWriter(_)
            | Self::SessionClosed
            | Self::InputRefused(_)
            | Self::Resize(_)
            | Self::Other(_) => PtyFailureKind::Other,
        }
    }

    /// The refusal a visible input banner should name, if this is one.
    pub(crate) const fn input_refusal(&self) -> Option<InputRefusal> {
        match self {
            Self::InputRefused(refusal) => Some(*refusal),
            _ => None,
        }
    }

    /// Bounded single-line prose safe for session rows, banners and panes.
    pub(crate) const fn user_summary(&self) -> &'static str {
        match self {
            Self::OpenPty(_) => "A terminal session is unavailable.",
            Self::Spawn(_) => "The system refused to start the terminal process.",
            Self::TeardownCapacity => {
                "Too many terminal sessions are still shutting down. Close one and retry."
            }
            Self::InputAuthorityExhausted => {
                "The terminal cannot start another session until the application is restarted."
            }
            Self::CloneReader(_) | Self::SpawnReader(_) => {
                "The terminal output reader could not start."
            }
            Self::SessionClosed => "The terminal session is closed.",
            Self::InputRefused(refusal) => refusal.user_summary(),
            #[cfg(test)]
            Self::WriterPoisoned | Self::Write(_) => "The terminal session encountered an error.",
            Self::TakeWriter(_) | Self::Resize(_) | Self::Other(_) => {
                "The terminal session encountered an error."
            }
        }
    }

    /// Full backend detail for an escaped diagnostic channel, never UI copy.
    pub(crate) fn diagnostic(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtyError::OpenPty(m) => write!(f, "openpty failed: {m}"),
            PtyError::Spawn(m) => write!(f, "spawn shell failed: {m}"),
            PtyError::TakeWriter(m) => write!(f, "take_writer failed: {m}"),
            PtyError::CloneReader(m) => write!(f, "try_clone_reader failed: {m}"),
            #[cfg(test)]
            PtyError::WriterPoisoned => write!(f, "pty writer lock poisoned"),
            PtyError::SessionClosed => write!(f, "pty session is closed"),
            PtyError::InputAuthorityExhausted => {
                f.write_str("pty input authority is exhausted: no unique generation is left")
            }
            PtyError::InputRefused(refusal) => write!(f, "pty input refused: {refusal:?}"),
            #[cfg(test)]
            PtyError::Write(m) => write!(f, "pty write failed: {m}"),
            PtyError::Resize(m) => write!(f, "pty resize failed: {m}"),
            PtyError::SpawnReader(m) => write!(f, "spawn PTY reader thread failed: {m}"),
            PtyError::TeardownCapacity => f.write_str("PTY teardown custody is at capacity"),
            PtyError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for PtyError {}
