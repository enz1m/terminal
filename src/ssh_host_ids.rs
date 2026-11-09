use std::collections::HashSet;

use crate::model::SshHostRecord;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Refusal {
    Duplicate,
    Invalid,
    Exhausted,
    Occupied,
    MissingEdit,
    TooMany,
}

impl Refusal {
    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Duplicate => {
                "Saved connection IDs are duplicated. No connection was changed. Repair the saved profiles and restart before retrying."
            }
            Self::Invalid => {
                "A saved connection ID is invalid. No connection was changed. Repair the saved profiles and restart before retrying."
            }
            Self::Exhausted => {
                "Saved connection IDs are exhausted. No connection was changed. Cancel and restore valid saved profiles before retrying."
            }
            Self::Occupied => {
                "The new connection ID is already in use. No connection was changed. Retry saving the connection."
            }
            Self::MissingEdit => {
                "The connection being edited is no longer current. No connection was changed. Reopen the connection and retry."
            }
            Self::TooMany => {
                "The saved connection limit has been reached. Remove a connection before retrying."
            }
        }
    }
}

pub(crate) fn numeric_id(id: &str) -> Result<Option<u64>, Refusal> {
    if id.is_empty() {
        return Err(Refusal::Invalid);
    }
    let Some(suffix) = id.strip_prefix("ssh-host-") else {
        return Ok(None);
    };
    // Legacy nonnumeric IDs remain opaque; malformed decimal owners must not
    // hide above the allocator's high-water mark.
    if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        suffix.parse().map(Some).map_err(|_| Refusal::Invalid)
    } else {
        Ok(None)
    }
}

pub(crate) fn validate(hosts: &[SshHostRecord]) -> Result<u64, Refusal> {
    if hosts.len() > crate::settings_store::MAX_HOSTS {
        return Err(Refusal::TooMany);
    }
    let mut ids = HashSet::with_capacity(hosts.len());
    let mut sequence = 0;
    for host in hosts {
        if host.id.len() > crate::settings_store::MAX_HOST_STRING_BYTES {
            return Err(Refusal::Invalid);
        }
        if !ids.insert(host.id.as_str()) {
            return Err(Refusal::Duplicate);
        }
        sequence = sequence.max(numeric_id(&host.id)?.unwrap_or(0));
    }
    Ok(sequence)
}

pub(crate) struct Reservation {
    pub(crate) id: String,
    pub(crate) sequence: u64,
    pub(crate) edit_index: Option<usize>,
}

pub(crate) fn reserve(
    hosts: &[SshHostRecord],
    sequence: u64,
    editing_id: Option<&str>,
) -> Result<Reservation, Refusal> {
    let sequence = sequence.max(validate(hosts)?);
    if let Some(id) = editing_id {
        let index = hosts
            .iter()
            .position(|host| host.id == id)
            .ok_or(Refusal::MissingEdit)?;
        return Ok(Reservation {
            id: id.to_owned(),
            sequence,
            edit_index: Some(index),
        });
    }
    if hosts.len() == crate::settings_store::MAX_HOSTS {
        return Err(Refusal::TooMany);
    }
    let sequence = sequence.checked_add(1).ok_or(Refusal::Exhausted)?;
    reserve_candidate(hosts, sequence)
}

pub(crate) fn reserve_candidate(
    hosts: &[SshHostRecord],
    sequence: u64,
) -> Result<Reservation, Refusal> {
    let id = format!("ssh-host-{sequence}");
    if hosts.iter().any(|host| host.id == id) {
        return Err(Refusal::Occupied);
    }
    Ok(Reservation {
        id,
        sequence,
        edit_index: None,
    })
}
