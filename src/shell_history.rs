//! Accepted-command metadata from shell integration, never from terminal input.
//!
//! Shells without this signal leave in-app history empty. Each UTF-8 command is
//! hex-encoded so quotes, newlines and terminal controls cannot delimit the OSC.

pub(crate) const BASH_PROMPT_COMMAND: &str = include_str!("shell_history.bash");
pub(crate) const MAX_COMMAND_BYTES: usize = 4096;
pub(crate) const HISTORY_DISABLED: &str = "enzui-history-disabled";

#[derive(Clone, Eq, PartialEq)]
pub(crate) enum ShellEvent {
    Started { key: u64, command: String },
    Finished { key: Option<u64>, status: i32 },
    Unavailable,
}

impl std::fmt::Debug for ShellEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Started { key, command } => f
                .debug_struct("Started")
                .field("key", key)
                .field("command_bytes", &command.len())
                .finish(),
            Self::Finished { key, status } => f
                .debug_struct("Finished")
                .field("key", key)
                .field("status", status)
                .finish(),
            Self::Unavailable => f.write_str("Unavailable"),
        }
    }
}

pub(crate) fn next_command_key() -> Option<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |key| {
        key.checked_add(1)
    })
    .ok()
}

pub(crate) fn command_row_id(key: u64) -> String {
    format!("shell-command-{key}")
}

pub(crate) fn is_command_row_id(id: &str) -> bool {
    id.strip_prefix("shell-command-")
        .is_some_and(|key| key.parse::<u64>().is_ok())
}

pub(crate) fn history_disabled_from_osc(fields: &[String]) -> bool {
    matches!(fields, [name] if name == HISTORY_DISABLED)
}

pub(crate) fn command_from_osc(fields: &[String]) -> Option<String> {
    let [name, hex] = fields else {
        return None;
    };
    if name != "enzui-command"
        || hex.is_empty()
        || hex.len() > MAX_COMMAND_BYTES * 2
        || !hex.len().is_multiple_of(2)
    {
        return None;
    }
    let bytes: Option<Vec<u8>> = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect();
    let command = String::from_utf8(bytes?).ok()?;
    (!command.trim().is_empty()).then_some(command)
}

#[path = "shell_cwd.rs"]
pub(crate) mod cwd;

#[cfg(unix)]
#[path = "shell_startup.rs"]
pub(crate) mod local;

#[cfg(test)]
mod tests;
