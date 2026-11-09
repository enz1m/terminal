//! User-safe projections for backend failures.
//!
//! Full details remain available only on escaped, field-labelled diagnostics;
//! render state receives static category prose.

use crate::pty;

/// Escape every scalar before backend detail reaches a terminal diagnostic.
/// This prevents newline, bidi and control payloads from forging log records.
pub(crate) fn escaped_diagnostic(diagnostic: &str) -> String {
    diagnostic.chars().flat_map(char::escape_default).collect()
}

/// Translate one PTY failure and retain full detail on the diagnostic channel.
fn pty_failure_summary(route: &str, error: &pty::PtyError) -> &'static str {
    let diagnostic = escaped_diagnostic(&error.diagnostic());
    eprintln!(
        "terminal PTY failure: route={route} kind={} diagnostic=\"{}\"",
        error.kind().as_str(),
        diagnostic
    );
    error.user_summary()
}

pub(crate) fn shell_spawn_failure(error: &pty::PtyError) -> String {
    format!(
        "{}{}",
        crate::SHELL_SPAWN_FAILED_PREFIX,
        pty_failure_summary("shell-spawn", error)
    )
}

pub(crate) fn ssh_connection_failure(error: &pty::PtyError) -> String {
    crate::i18n::format(
        crate::i18n::localizer(),
        "SSH connection failed: {}",
        &[("arg0", &(pty_failure_summary("ssh-connect", error)))],
    )
}

impl crate::App {
    /// Apply one config read through the reducer's typed translation boundary.
    pub(crate) fn apply_openssh_config_result(
        &mut self,
        result: Result<Vec<crate::ssh_config::OpenSshHost>, crate::ssh_config::OpenSshConfigError>,
    ) {
        match result {
            Ok(hosts) => {
                self.openssh_hosts = hosts;
                self.openssh_load_error = None;
            }
            Err(error) => {
                let diagnostic = escaped_diagnostic(error.diagnostic());
                eprintln!(
                    "terminal OpenSSH config failure: kind={} diagnostic=\"{}\"",
                    error.kind().as_str(),
                    diagnostic
                );
                self.openssh_hosts.clear();
                self.openssh_load_error = Some(crate::i18n::text(error.user_summary()));
            }
        }
    }
}
