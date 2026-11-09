//! Pinned workspace metadata and Secret Service references; never transcript payloads.

use crate::pinned_secret::{Marker, Reference};
use std::collections::{HashMap, HashSet};

use enzui_core::persist::{InMemoryBackend, Persistable, RawByteLimit, Store, XdgResolver};

use crate::model::{CommandHistory, Session, SessionKind, SessionStatus};
use crate::pane_endpoint::PaneEndpoint;
use crate::pane_group::GroupMember;
use crate::pane_layout::{PaneLayout, SplitDirection};

pub(crate) const FORMAT_ID: &str = "terminal-emulator-sessions";
pub(crate) const VERSION: u32 = 3;
pub(crate) const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const RAW_BYTE_LIMIT: RawByteLimit =
    match RawByteLimit::for_body(FORMAT_ID, VERSION, MAX_DOCUMENT_BYTES as u64) {
        Ok(limit) => limit,
        Err(_) => panic!("session body budget must fit the framework frame ceiling"),
    };

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SessionState {
    pub(crate) pending_deletes: Vec<Reference>,
    pub sessions: Vec<SavedSession>,
    pub active_session_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SavedSession {
    pub(crate) archive: Option<Reference>,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: SessionKind,
    pub ssh_host_id: Option<String>,
    pub pinned: bool,
    pub detached: bool,
    /// An explicit user rename wins over later OSC 0/2 shell titles.
    pub title_overridden: bool,
    pub panes: Vec<SavedPane>,
    pub active_pane_id: String,
    pub layout: PaneLayout,
    /// Restartable identity/topology for sessions merged into this one. This
    /// keeps the ungroup affordance and exact pre-group split trees alive
    /// across a process restart.
    pub group_members: Vec<SavedGroupMember>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SavedGroupMember {
    pub session_id: String,
    pub title: String,
    pub subtitle: String,
    pub icon: String,
    pub kind: SessionKind,
    pub ssh_host_id: Option<String>,
    pub pinned: bool,
    pub detached: bool,
    pub pane_ids: Vec<String>,
    pub layout: PaneLayout,
    pub active_pane_id: String,
    pub members: Vec<SavedGroupMember>,
}

impl SavedGroupMember {
    fn snapshot(member: &GroupMember) -> Self {
        Self {
            session_id: member.session_id.clone(),
            title: member.title.clone(),
            subtitle: member.subtitle.clone(),
            icon: member.icon.clone(),
            kind: member.kind,
            ssh_host_id: member.ssh_host_id.clone(),
            pinned: member.pinned,
            detached: member.detached,
            pane_ids: member.pane_ids.clone(),
            layout: member.layout.clone(),
            active_pane_id: member.active_pane_id.clone(),
            members: member.members.iter().map(Self::snapshot).collect(),
        }
    }

    pub(crate) fn restore(&self) -> GroupMember {
        GroupMember {
            session_id: self.session_id.clone(),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            icon: self.icon.clone(),
            status: SessionStatus::Idle,
            kind: self.kind,
            ssh_host_id: self.ssh_host_id.clone(),
            connection_error: None,
            pinned: self.pinned,
            detached: self.detached,
            pane_ids: self.pane_ids.clone(),
            layout: self.layout.clone(),
            active_pane_id: self.active_pane_id.clone(),
            in_app_history: CommandHistory::new(),
            members: self.members.iter().map(Self::restore).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SavedPane {
    pub endpoint: PaneEndpoint,
    pub id: String,
    pub title: String,
    pub cwd: String,
}

impl SessionState {
    /// Captures pinned topology only; rendered output must come from the PTY privacy filter.
    #[must_use]
    pub(crate) fn snapshot(
        sessions: &[Session],
        active: usize,
        title_overrides: &HashSet<String>,
    ) -> Self {
        Self {
            pending_deletes: Vec::new(),
            sessions: sessions
                .iter()
                .filter(|session| session.pinned)
                .filter_map(|session| {
                    let active_pane_id = session.panes.get(session.active_pane)?.id.clone();
                    Some(SavedSession {
                        archive: None,
                        id: session.id.clone(),
                        title: session.title.clone(),
                        subtitle: session.subtitle.clone(),
                        kind: session.kind,
                        ssh_host_id: session.ssh_host_id.clone(),
                        pinned: session.pinned,
                        detached: session.detached,
                        title_overridden: title_overrides.contains(&session.id),
                        panes: session
                            .panes
                            .iter()
                            .map(|pane| SavedPane {
                                endpoint: pane.endpoint.clone(),
                                id: pane.id.clone(),
                                title: pane.title.clone(),
                                cwd: String::new(),
                            })
                            .collect(),
                        layout: session.layout.clone(),
                        active_pane_id,
                        group_members: session
                            .group_members
                            .iter()
                            .map(SavedGroupMember::snapshot)
                            .collect(),
                    })
                })
                .collect(),
            active_session_id: sessions
                .get(active)
                .filter(|session| session.pinned)
                .or_else(|| sessions.iter().find(|session| session.pinned))
                .map(|session| session.id.clone()),
        }
    }

    fn record_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.sessions.len(),
            self.sessions
                .iter()
                .map(|session| session.panes.len())
                .sum(),
            self.sessions
                .iter()
                .filter(|session| session.archive.is_some())
                .count(),
            self.pending_deletes.len(),
        )
    }

    /// Rejects the whole document when ownership or topology is contradictory.
    #[must_use]
    pub(crate) fn is_sound(&self) -> bool {
        let mut keys = HashSet::new();
        if self
            .pending_deletes
            .iter()
            .any(|reference| !reference.valid() || !keys.insert(reference.key.as_str()))
        {
            return false;
        }
        keys.clear();
        if self
            .sessions
            .iter()
            .filter_map(|session| session.archive.as_ref())
            .any(|reference| !reference.valid() || !keys.insert(reference.key.as_str()))
        {
            return false;
        }
        if self.sessions.is_empty() {
            return self.active_session_id.is_none();
        }
        let mut session_ids = HashSet::new();
        let mut pane_ids = HashSet::new();
        for session in &self.sessions {
            if session.id.is_empty()
                || !session_ids.insert(session.id.as_str())
                || session.panes.is_empty()
                || session.panes.len() > crate::pane_layout::MAX_PANES
                || [&session.id, &session.title, &session.subtitle]
                    .iter()
                    .any(|text| !crate::pinned_output::plain_text(text))
            {
                return false;
            }
            let own_panes: HashSet<&str> =
                session.panes.iter().map(|pane| pane.id.as_str()).collect();
            if own_panes.len() != session.panes.len()
                || !own_panes.contains(session.active_pane_id.as_str())
                || session.panes.iter().any(|pane| {
                    pane.id.is_empty()
                        || !pane_ids.insert(pane.id.as_str())
                        || [&pane.id, &pane.title, &pane.cwd]
                            .iter()
                            .any(|text| !crate::pinned_output::plain_text(text))
                })
            {
                return false;
            }
            let mut leaves = Vec::new();
            collect_leaves(&session.layout, &mut leaves);
            let layout_panes: HashSet<&str> = leaves.iter().copied().collect();
            if layout_panes.len() != leaves.len()
                || layout_panes != own_panes
                || !group_members_are_sound(&session.group_members, &own_panes)
            {
                return false;
            }
        }
        self.active_session_id
            .as_deref()
            .is_some_and(|active| session_ids.contains(active))
    }
}

fn group_members_are_sound(members: &[SavedGroupMember], owner_panes: &HashSet<&str>) -> bool {
    fn member_is_sound<'a>(
        member: &'a SavedGroupMember,
        available: &HashSet<&'a str>,
        session_ids: &mut HashSet<&'a str>,
    ) -> bool {
        if member.session_id.is_empty()
            || !session_ids.insert(member.session_id.as_str())
            || member.pane_ids.is_empty()
            || [
                &member.session_id,
                &member.title,
                &member.subtitle,
                &member.icon,
            ]
            .iter()
            .any(|text| !crate::pinned_output::plain_text(text))
        {
            return false;
        }
        let pane_ids: HashSet<&str> = member.pane_ids.iter().map(String::as_str).collect();
        if pane_ids.len() != member.pane_ids.len()
            || !pane_ids.is_subset(available)
            || !pane_ids.contains(member.active_pane_id.as_str())
        {
            return false;
        }
        let mut leaves = Vec::new();
        collect_leaves(&member.layout, &mut leaves);
        if leaves.into_iter().collect::<HashSet<_>>() != pane_ids {
            return false;
        }
        let mut nested_owned = HashSet::new();
        for nested in &member.members {
            if !member_is_sound(nested, &pane_ids, session_ids)
                || nested
                    .pane_ids
                    .iter()
                    .any(|id| !nested_owned.insert(id.as_str()))
            {
                return false;
            }
        }
        true
    }

    let mut session_ids = HashSet::new();
    let mut claimed = HashSet::new();
    members.iter().all(|member| {
        member_is_sound(member, owner_panes, &mut session_ids)
            && member.pane_ids.iter().all(|id| claimed.insert(id.as_str()))
    })
}

impl Persistable for SessionState {
    fn persist_serialize(&self) -> Result<String, String> {
        let mut body = String::from("enzui-terminal-sessions-v3\n");
        if self.sessions.is_empty()
            && self.active_session_id.is_none()
            && self.pending_deletes.is_empty()
        {
            body.push_str("empty\t1\n");
            return Ok(body);
        }
        if !self.is_sound() || self.sessions.iter().any(|session| !session.pinned) {
            return Err("invalid pinned workspace".into());
        }
        if let Some(active) = &self.active_session_id {
            body.push_str(&format!("active\t{}\n", hex(active.as_bytes())));
        }
        for session in &self.sessions {
            body.push_str("session\t");
            body.push_str(
                &[
                    hex(session.id.as_bytes()),
                    hex(session.title.as_bytes()),
                    hex(session.subtitle.as_bytes()),
                    match session.kind {
                        SessionKind::Local => "local".to_owned(),
                        SessionKind::Ssh => "ssh".to_owned(),
                    },
                    session
                        .ssh_host_id
                        .as_deref()
                        .map_or_else(String::new, |id| hex(id.as_bytes())),
                    u8::from(session.pinned).to_string(),
                    u8::from(session.detached).to_string(),
                    u8::from(session.title_overridden).to_string(),
                    hex(session.active_pane_id.as_bytes()),
                    hex(encode_layout(&session.layout).as_bytes()),
                    hex(encode_group_members(&session.group_members).as_bytes()),
                ]
                .join("\t"),
            );
            body.push('\n');
            for pane in &session.panes {
                body.push_str("pane\t");
                body.push_str(
                    &[
                        hex(session.id.as_bytes()),
                        hex(pane.id.as_bytes()),
                        hex(pane.title.as_bytes()),
                        String::new(),
                        hex(pane.endpoint.encode().as_bytes()),
                    ]
                    .join("\t"),
                );
                body.push('\n');
                if body.len() > MAX_DOCUMENT_BYTES {
                    return Err("pinned workspace exceeds document budget".into());
                }
            }
        }
        for session in &self.sessions {
            if let Some(reference) = &session.archive {
                body.push_str(&format!(
                    "secret\t{}\t{}\t{}\n",
                    hex(session.id.as_bytes()),
                    reference.key,
                    encode_marker(reference.marker.as_ref())
                ));
            }
        }
        for reference in &self.pending_deletes {
            body.push_str(&format!(
                "remove-secret\t{}\t{}\n",
                reference.key,
                encode_marker(reference.marker.as_ref())
            ));
        }
        let counts = self.record_counts();
        body.push_str(&format!(
            "end\t{}\t{}\t{}\t{}\n",
            counts.0, counts.1, counts.2, counts.3
        ));
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err("pinned workspace exceeds document budget".into());
        }
        Ok(body)
    }

    fn persist_deserialize(body: &str) -> Result<Self, String> {
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err("pinned workspace exceeds document budget".into());
        }
        let header = body.lines().next().ok_or("missing session header")?;
        if !matches!(
            header,
            "enzui-terminal-sessions-v1" | "enzui-terminal-sessions-v3"
        ) {
            return Err("unsupported session body version".into());
        }
        let legacy_version = header == "enzui-terminal-sessions-v1";
        if body == format!("{header}\nempty\t1\n") {
            return Ok(Self::default());
        }
        let mut end_counts = None;
        let mut active_session_id = None;
        let mut sessions = Vec::new();
        let mut panes: HashMap<String, Vec<SavedPane>> = HashMap::new();
        let mut legacy = HashSet::new();
        let mut references: HashMap<String, Reference> = HashMap::new();
        let mut pending_deletes = Vec::new();
        for line in body.lines().skip(1) {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.first() == Some(&"session")
                && (fields.len() < 9
                    || fields[6..9]
                        .iter()
                        .any(|value| !matches!(*value, "0" | "1")))
            {
                return Err("invalid session flags".into());
            }
            if end_counts.is_some() {
                return Err("records after session footer".into());
            }
            match fields.as_slice() {
                ["end", sessions, panes, references, pending] if !legacy_version => {
                    let number =
                        |text: &str| text.parse::<usize>().map_err(|_| "invalid session footer");
                    end_counts = Some((
                        number(sessions)?,
                        number(panes)?,
                        number(references)?,
                        number(pending)?,
                    ));
                }
                ["active", active] => {
                    if active_session_id.is_some() {
                        return Err("duplicate active session".into());
                    }
                    active_session_id = Some(unhex(active).ok_or("invalid active session")?);
                }
                ["secret", owner, key, marker] if !legacy_version => {
                    let reference = Reference {
                        key: (*key).to_owned(),
                        marker: decode_marker(marker)?,
                    };
                    if !reference.valid()
                        || references
                            .insert(unhex(owner).ok_or("invalid secret owner")?, reference)
                            .is_some()
                    {
                        return Err("invalid secret reference".into());
                    }
                }
                ["remove-secret", key, marker] if !legacy_version => {
                    let reference = Reference {
                        key: (*key).to_owned(),
                        marker: decode_marker(marker)?,
                    };
                    if !reference.valid() {
                        return Err("invalid deletion reference".into());
                    }
                    pending_deletes.push(reference);
                }
                [
                    "session",
                    id,
                    title,
                    subtitle,
                    kind,
                    ssh_host,
                    pinned,
                    detached,
                    title_overridden,
                    active_pane,
                    layout,
                    group_members,
                ] => {
                    let Some((id, title, subtitle, active_pane_id, layout_text, members_text)) =
                        unhex(id)
                            .zip(unhex(title))
                            .zip(unhex(subtitle))
                            .zip(unhex(active_pane))
                            .zip(unhex(layout))
                            .zip(unhex(group_members))
                            .map(
                                |(((((id, title), subtitle), active_pane), layout), members)| {
                                    (id, title, subtitle, active_pane, layout, members)
                                },
                            )
                    else {
                        return Err("invalid session record".into());
                    };
                    let Some(group_members) = decode_group_members(&members_text) else {
                        return Err("invalid session record".into());
                    };
                    let kind = match *kind {
                        "local" => SessionKind::Local,
                        "ssh" => SessionKind::Ssh,
                        _ => return Err("invalid session kind".into()),
                    };
                    let Some(layout) = decode_layout(&layout_text) else {
                        return Err("invalid session record".into());
                    };
                    sessions.push(SavedSession {
                        archive: None,
                        id,
                        title,
                        subtitle,
                        kind,
                        ssh_host_id: if ssh_host.is_empty() {
                            None
                        } else {
                            Some(unhex(ssh_host).ok_or("invalid SSH owner")?)
                        },
                        pinned: *pinned == "1",
                        detached: *detached == "1",
                        title_overridden: *title_overridden == "1",
                        panes: Vec::new(),
                        active_pane_id,
                        layout,
                        group_members,
                    });
                }
                [
                    "session",
                    id,
                    title,
                    subtitle,
                    kind,
                    ssh_host,
                    pinned,
                    detached,
                    title_overridden,
                    active_pane,
                    layout,
                ] => {
                    let Some((id, title, subtitle, active_pane_id, layout_text)) = unhex(id)
                        .zip(unhex(title))
                        .zip(unhex(subtitle))
                        .zip(unhex(active_pane))
                        .zip(unhex(layout))
                        .map(|((((id, title), subtitle), active_pane), layout)| {
                            (id, title, subtitle, active_pane, layout)
                        })
                    else {
                        return Err("invalid session record".into());
                    };
                    let Some(layout) = decode_layout(&layout_text) else {
                        return Err("invalid session record".into());
                    };
                    let kind = match *kind {
                        "local" => SessionKind::Local,
                        "ssh" => SessionKind::Ssh,
                        _ => return Err("invalid session kind".into()),
                    };
                    sessions.push(SavedSession {
                        archive: None,
                        id,
                        title,
                        subtitle,
                        kind,
                        ssh_host_id: if ssh_host.is_empty() {
                            None
                        } else {
                            Some(unhex(ssh_host).ok_or("invalid SSH owner")?)
                        },
                        pinned: *pinned == "1",
                        detached: *detached == "1",
                        title_overridden: *title_overridden == "1",
                        panes: Vec::new(),
                        active_pane_id,
                        layout,
                        group_members: Vec::new(),
                    });
                }
                ["pane", session_id, id, title, cwd, endpoint] => {
                    if !legacy_version && !cwd.is_empty() {
                        return Err("working directories belong in Secret Service".into());
                    }
                    let session_id = unhex(session_id).ok_or("invalid pane session id")?;
                    let endpoint = unhex(endpoint)
                        .and_then(|raw| PaneEndpoint::decode(&raw))
                        .ok_or("invalid pane endpoint")?;
                    panes.entry(session_id).or_default().push(SavedPane {
                        id: unhex(id).ok_or("invalid pane id")?,
                        title: unhex(title).ok_or("invalid pane title")?,
                        cwd: unhex(cwd).ok_or("invalid pane directory")?,
                        endpoint,
                    });
                }
                ["pane", session_id, id, title, cwd] => {
                    if let (Some(session_id), Some(id), Some(title), Some(cwd)) =
                        (unhex(session_id), unhex(id), unhex(title), unhex(cwd))
                    {
                        legacy.insert(id.clone());
                        panes.entry(session_id).or_default().push(SavedPane {
                            id,
                            title,
                            cwd,
                            endpoint: PaneEndpoint::Local,
                        });
                    } else {
                        return Err("invalid legacy pane".into());
                    }
                }
                _ => return Err("unknown session record".into()),
            }
        }
        for session in &mut sessions {
            session.archive = references.remove(&session.id);
            if !session.pinned && session.archive.is_some() {
                return Err("unpinned secret reference".into());
            }
            session.panes = panes.remove(&session.id).unwrap_or_default();
            for pane in &mut session.panes {
                if legacy.contains(&pane.id) {
                    let (kind, profile) = legacy_owner(&pane.id, &session.group_members)
                        .unwrap_or((session.kind, session.ssh_host_id.as_deref()));
                    pane.endpoint = match kind {
                        SessionKind::Local => PaneEndpoint::Local,
                        SessionKind::Ssh => PaneEndpoint::LegacySsh(profile.map(str::to_owned)),
                    };
                }
            }
        }
        if !panes.is_empty() || !references.is_empty() {
            return Err("orphan session data".into());
        }
        let mut state = Self {
            pending_deletes,
            sessions,
            active_session_id,
        };
        if state.sessions.is_empty() && state.pending_deletes.is_empty() {
            return Err("missing explicit empty marker".into());
        }
        if !legacy_version && end_counts != Some(state.record_counts()) {
            return Err("incomplete pinned workspace".into());
        }
        if state.is_sound() {
            state.sessions.retain(|session| session.pinned);
            if !state
                .sessions
                .iter()
                .any(|session| Some(&session.id) == state.active_session_id.as_ref())
            {
                state.active_session_id = state.sessions.first().map(|session| session.id.clone());
            }
            Ok(state)
        } else {
            Err("session state has inconsistent tabs, panes, or layout".into())
        }
    }

    fn persist_default() -> Self {
        Self::default()
    }
}

fn encode_marker(marker: Option<&Marker>) -> String {
    match marker {
        None => "reserved".into(),
        Some(marker) => format!(
            "stored-v1:{}:{}:{}:{}:{:016x}",
            marker.bound, marker.rows, marker.commands, marker.bytes, marker.checksum
        ),
    }
}
fn decode_marker(text: &str) -> Result<Option<Marker>, String> {
    if text == "reserved" {
        return Ok(None);
    }
    let fields: Vec<_> = text.split(':').collect();
    let ["stored-v1", bound, rows, commands, bytes, checksum] = fields.as_slice() else {
        return Err("invalid marker".into());
    };
    let number = |text: &str| {
        text.parse::<usize>()
            .map_err(|_| "invalid marker".to_owned())
    };
    let marker = Marker {
        bound: number(bound)?,
        rows: number(rows)?,
        commands: number(commands)?,
        bytes: number(bytes)?,
        checksum: u64::from_str_radix(checksum, 16).map_err(|_| "invalid marker")?,
    };
    if !marker.valid() {
        return Err("invalid marker bounds".into());
    }
    Ok(Some(marker))
}

pub(crate) fn private_store(
    backend: impl enzui_core::persist::PersistBackend,
) -> Store<SessionState> {
    Store::new(SessionVersions(backend), FORMAT_ID, VERSION)
}

struct SessionVersions<B>(B);
impl<B: enzui_core::persist::PersistBackend> enzui_core::persist::PersistBackend
    for SessionVersions<B>
{
    fn load_raw(&self, format: &str, version: u32) -> enzui_core::persist::BackendLoad {
        use enzui_core::persist::BackendLoad;
        let current = self.0.load_raw(format, version);
        if matches!(&current, BackendLoad::Refused { reason, .. } if matches!(reason, enzui_core::atomic_state::StateRefusal::UnexpectedHeader))
            && let BackendLoad::Loaded(body) = self.0.load_raw(format, 1)
            && body.starts_with("enzui-terminal-sessions-v1\n")
        {
            return BackendLoad::Loaded(body);
        }
        current
    }
    fn save_raw(
        &self,
        format: &str,
        version: u32,
        body: &str,
    ) -> Result<(), enzui_core::persist::PersistError> {
        self.0.save_raw(format, version, body)
    }

    fn raw_byte_limit(&self) -> RawByteLimit {
        self.0.raw_byte_limit()
    }
}

pub(crate) fn store() -> Store<SessionState> {
    #[cfg(test)]
    {
        Store::new(InMemoryBackend::new(), FORMAT_ID, VERSION)
    }
    #[cfg(not(test))]
    {
        store_with_resolver(&XdgResolver::from_env())
    }
}

pub(crate) fn store_with_resolver(resolver: &XdgResolver) -> Store<SessionState> {
    use enzui_core::persist::{FileBackend, PersistenceKind};
    match FileBackend::xdg_with_resolver(
        resolver,
        PersistenceKind::State,
        "terminal-emulator",
        "sessions.json",
        RAW_BYTE_LIMIT,
    )
    .expect("the static session XDG descendants are valid")
    {
        Some(backend) => private_store(backend),
        None => Store::new(InMemoryBackend::new(), FORMAT_ID, VERSION),
    }
}

fn encode_group_members(members: &[SavedGroupMember]) -> String {
    members
        .iter()
        .map(|member| hex(encode_group_member(member).as_bytes()))
        .collect::<Vec<_>>()
        .join(",")
}

fn encode_group_member(member: &SavedGroupMember) -> String {
    [
        hex(member.session_id.as_bytes()),
        hex(member.title.as_bytes()),
        hex(member.subtitle.as_bytes()),
        hex(member.icon.as_bytes()),
        match member.kind {
            SessionKind::Local => "local".to_owned(),
            SessionKind::Ssh => "ssh".to_owned(),
        },
        member
            .ssh_host_id
            .as_deref()
            .map_or_else(String::new, |id| hex(id.as_bytes())),
        u8::from(member.pinned).to_string(),
        u8::from(member.detached).to_string(),
        member
            .pane_ids
            .iter()
            .map(|id| hex(id.as_bytes()))
            .collect::<Vec<_>>()
            .join(","),
        hex(member.active_pane_id.as_bytes()),
        hex(encode_layout(&member.layout).as_bytes()),
        encode_group_members(&member.members),
    ]
    .join("|")
}

fn decode_group_members(text: &str) -> Option<Vec<SavedGroupMember>> {
    if text.is_empty() {
        return Some(Vec::new());
    }
    text.split(',')
        .map(|encoded| decode_group_member(&unhex(encoded)?))
        .collect()
}

fn decode_group_member(text: &str) -> Option<SavedGroupMember> {
    let fields: Vec<&str> = text.split('|').collect();
    let [
        id,
        title,
        subtitle,
        icon,
        kind,
        ssh_host,
        pinned,
        detached,
        panes,
        active,
        layout,
        members,
    ] = fields.as_slice()
    else {
        return None;
    };
    if !matches!(*pinned, "0" | "1") || !matches!(*detached, "0" | "1") {
        return None;
    }
    Some(SavedGroupMember {
        session_id: unhex(id)?,
        title: unhex(title)?,
        subtitle: unhex(subtitle)?,
        icon: unhex(icon)?,
        kind: match *kind {
            "local" => SessionKind::Local,
            "ssh" => SessionKind::Ssh,
            _ => return None,
        },
        ssh_host_id: if ssh_host.is_empty() {
            None
        } else {
            Some(unhex(ssh_host)?)
        },
        pinned: *pinned == "1",
        detached: *detached == "1",
        pane_ids: if panes.is_empty() {
            Vec::new()
        } else {
            panes.split(',').map(unhex).collect::<Option<Vec<_>>>()?
        },
        active_pane_id: unhex(active)?,
        layout: decode_layout(&unhex(layout)?)?,
        members: decode_group_members(members)?,
    })
}

fn collect_leaves<'a>(layout: &'a PaneLayout, leaves: &mut Vec<&'a str>) {
    match layout {
        PaneLayout::Leaf(id) => leaves.push(id),
        PaneLayout::Split { first, second, .. } => {
            collect_leaves(first, leaves);
            collect_leaves(second, leaves);
        }
    }
}

fn encode_layout(layout: &PaneLayout) -> String {
    match layout {
        PaneLayout::Leaf(id) => format!("L{}:{}", id.len(), id),
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => format!(
            "S{}{:08x}{}{}",
            match direction {
                SplitDirection::Horizontal => 'H',
                SplitDirection::Vertical => 'V',
            },
            ratio.to_bits(),
            encode_layout(first),
            encode_layout(second)
        ),
    }
}

fn decode_layout(text: &str) -> Option<PaneLayout> {
    fn node(text: &str, at: &mut usize, depth: usize) -> Option<PaneLayout> {
        if depth > 64 {
            return None;
        }
        match text.as_bytes().get(*at).copied()? {
            b'L' => {
                *at += 1;
                let len_start = *at;
                while text.as_bytes().get(*at).is_some_and(u8::is_ascii_digit) {
                    *at += 1;
                }
                if text.as_bytes().get(*at) != Some(&b':') {
                    return None;
                }
                let len = text[len_start..*at].parse::<usize>().ok()?;
                *at += 1;
                let end = at.checked_add(len)?;
                let id = text.get(*at..end)?.to_owned();
                *at = end;
                Some(PaneLayout::Leaf(id))
            }
            b'S' => {
                let direction = match text.as_bytes().get(*at + 1) {
                    Some(b'H') => SplitDirection::Horizontal,
                    Some(b'V') => SplitDirection::Vertical,
                    _ => return None,
                };
                let ratio_bits = u32::from_str_radix(text.get(*at + 2..*at + 10)?, 16).ok()?;
                let ratio = f32::from_bits(ratio_bits);
                if !ratio.is_finite() || !(0.0..=1.0).contains(&ratio) {
                    return None;
                }
                *at += 10;
                let first = Box::new(node(text, at, depth + 1)?);
                let second = Box::new(node(text, at, depth + 1)?);
                Some(PaneLayout::Split {
                    direction,
                    ratio,
                    first,
                    second,
                })
            }
            _ => None,
        }
    }
    let mut at = 0;
    let layout = node(text, &mut at, 0)?;
    (at == text.len()).then_some(layout)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn unhex(value: &str) -> Option<String> {
    if !value.is_ascii() || !value.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

#[cfg(test)]
mod tests;

fn legacy_owner<'a>(
    pane: &str,
    members: &'a [SavedGroupMember],
) -> Option<(SessionKind, Option<&'a str>)> {
    let owner = members
        .iter()
        .find(|member| member.pane_ids.iter().any(|id| id == pane))?;
    legacy_owner(pane, &owner.members).or(Some((owner.kind, owner.ssh_host_id.as_deref())))
}
