//! Terminal SSH host-row helpers.

use crate::{SshHostRecord, ssh_config};

/// A saved host's accessible name — its own name when that is unique,
/// otherwise progressively more specific until it is.
///
/// `SaveSshConnection` accepts any non-blank name, so two saved hosts may be
/// called `prod`. Their action buttons would then both be `Connect to SSH host
/// prod`, which is exactly the ambiguity this issue exists to remove — just
/// moved from "every row says Connect" to "two rows say the same thing". The
/// destination is appended first (it is what the user reads on the row's own
/// second line), and the record id last, which is unique by construction.
pub(crate) fn saved_host_at_name(hosts: &[SshHostRecord], index: usize) -> String {
    let host = &hosts[index];
    let unique_by = |form: &dyn Fn(&SshHostRecord) -> String| {
        let mine = form(host);
        hosts
            .iter()
            .enumerate()
            .all(|(j, other)| j == index || form(other) != mine)
    };
    if unique_by(&|h: &SshHostRecord| h.name.clone()) {
        return host.name.clone();
    }
    let with_destination = |h: &SshHostRecord| format!("{} · {}", h.name, h.destination);
    if unique_by(&with_destination) {
        return with_destination(host);
    }
    format!("{} · {}", with_destination(host), host.id)
}

/// An OpenSSH row's accessible name, disambiguated the same way.
///
/// A literal `Host a b` line yields one row per alias, and an `Include` can
/// re-declare an alias, so duplicates are reachable here too. There is no
/// synthetic id to fall back on — two rows with the same alias AND the same
/// destination are the same connection twice — so the final fallback is the
/// occurrence ordinal, which is stable for a given config text.
pub(crate) fn openssh_host_at_name(hosts: &[ssh_config::OpenSshHost], index: usize) -> String {
    let host = &hosts[index];
    let destination = |h: &ssh_config::OpenSshHost| {
        if h.user.is_empty() {
            format!("{}:{}", h.hostname, h.port)
        } else {
            format!("{}@{}:{}", h.user, h.hostname, h.port)
        }
    };
    let unique_by = |form: &dyn Fn(&ssh_config::OpenSshHost) -> String| {
        let mine = form(host);
        hosts
            .iter()
            .enumerate()
            .all(|(j, other)| j == index || form(other) != mine)
    };
    if unique_by(&|h: &ssh_config::OpenSshHost| h.alias.clone()) {
        return host.alias.clone();
    }
    let with_destination =
        |h: &ssh_config::OpenSshHost| format!("{} · {}", h.alias, destination(h));
    if unique_by(&with_destination) {
        return with_destination(host);
    }
    let ordinal = hosts
        .iter()
        .take(index + 1)
        .filter(|h| with_destination(h) == with_destination(host))
        .count();
    format!("{} · #{ordinal}", with_destination(host))
}
