use crate::model::{SessionKind, SshAuthMode, SshHostRecord};
use crate::session_store::{hex, unhex};
use crate::ssh_config::OpenSshHost;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneEndpoint {
    Local,
    Profile(SshHostRecord),
    OpenSsh(OpenSshHost),
    LegacySsh(Option<String>),
}

impl PaneEndpoint {
    pub(crate) fn profile(host: &SshHostRecord) -> Self {
        let mut snapshot = host.clone();
        snapshot.notes.clear();
        Self::Profile(snapshot)
    }

    pub(crate) fn kind(&self) -> SessionKind {
        match self {
            Self::Local => SessionKind::Local,
            _ => SessionKind::Ssh,
        }
    }

    pub(crate) fn profile_id(&self) -> Option<&str> {
        match self {
            Self::Profile(host) => Some(&host.id),
            Self::LegacySsh(id) => id.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn remote_hint(&self) -> &str {
        match self {
            Self::Profile(host) => &host.destination,
            Self::OpenSsh(host) => &host.hostname,
            Self::LegacySsh(Some(id)) => id,
            _ => "remote",
        }
    }

    pub(crate) fn resolve(
        &self,
        profiles: &[SshHostRecord],
        aliases: &[OpenSshHost],
    ) -> Result<SshHostRecord, &'static str> {
        match self {
            Self::Local => Err("This pane is a local shell, not an SSH endpoint."),
            Self::LegacySsh(_) => Err(
                "This workspace has no per-pane SSH snapshot. Open Remote and connect explicitly; the old endpoint cannot be inferred safely.",
            ),
            Self::Profile(saved) => {
                let current = profiles.iter().find(|host| host.id == saved.id).ok_or(
                    "The saved SSH profile was deleted. Open Remote to choose a new connection.",
                )?;
                if current.destination != saved.destination
                    || current.port != saved.port
                    || current.username != saved.username
                    || current.auth != saved.auth
                    || current.path != saved.path
                {
                    return Err(
                        "The saved SSH endpoint or authentication changed. Open Remote to connect explicitly; this pane was not retargeted.",
                    );
                }
                Ok(saved.clone())
            }
            Self::OpenSsh(saved) => {
                let current = aliases.iter().find(|host| host.alias == saved.alias)
                    .ok_or("The OpenSSH alias is missing or its config could not be read. This pane remains disconnected.")?;
                if current != saved {
                    return Err(
                        "The OpenSSH config changed. Open Remote to connect explicitly; this pane was not retargeted.",
                    );
                }
                Ok(SshHostRecord {
                    id: format!("openssh:{}", saved.alias),
                    name: saved.alias.clone(),
                    destination: saved.alias.clone(),
                    port: saved.port,
                    username: saved.user.clone(),
                    path: String::new(),
                    auth: saved
                        .identity_file
                        .as_ref()
                        .map_or(SshAuthMode::Agent, |key_path| SshAuthMode::KeyFile {
                            key_path: key_path.clone(),
                        }),
                    notes: String::new(),
                })
            }
        }
    }

    pub(crate) fn remote_directory(&self, cwd: &str, fallback: &str) -> String {
        if cwd.starts_with('/') {
            return cwd.to_owned();
        }
        // The first :/ separates an OSC 7 host (including IPv6) from its path;
        // subsequent colons belong to the path, not another endpoint.
        cwd.split_once(":/")
            .map_or_else(|| fallback.to_owned(), |(_, path)| format!("/{path}"))
    }

    pub(crate) fn encode(&self) -> String {
        let fields = match self {
            Self::Local => vec!["local".to_owned()],
            Self::LegacySsh(id) => vec![
                "legacy-ssh".into(),
                hex(id.as_deref().unwrap_or("").as_bytes()),
            ],
            Self::Profile(host) => {
                let (auth, key) = match &host.auth {
                    SshAuthMode::Password => ("password", ""),
                    SshAuthMode::Agent => ("agent", ""),
                    SshAuthMode::KeyFile { key_path } => ("key", key_path.as_str()),
                };
                vec![
                    "profile-1".into(),
                    hex(host.id.as_bytes()),
                    hex(host.name.as_bytes()),
                    hex(host.destination.as_bytes()),
                    host.port.to_string(),
                    hex(host.username.as_bytes()),
                    hex(host.path.as_bytes()),
                    auth.into(),
                    hex(key.as_bytes()),
                ]
            }
            Self::OpenSsh(host) => vec![
                "openssh-1".into(),
                hex(host.alias.as_bytes()),
                hex(host.hostname.as_bytes()),
                hex(host.user.as_bytes()),
                host.port.to_string(),
                hex(host.identity_file.as_deref().unwrap_or("").as_bytes()),
                host.config_fingerprint.to_string(),
            ],
        };
        fields.join("\t")
    }

    pub(crate) fn decode(raw: &str) -> Option<Self> {
        let fields: Vec<_> = raw.split('\t').collect();
        match fields.as_slice() {
            ["local"] => Some(Self::Local),
            ["legacy-ssh", id] => Some(Self::LegacySsh({
                let id = unhex(id)?;
                (!id.is_empty()).then_some(id)
            })),
            [
                "profile-1",
                id,
                name,
                destination,
                port,
                username,
                path,
                auth,
                key,
            ] => {
                let key_path = unhex(key)?;
                let auth = match *auth {
                    "password" if key_path.is_empty() => SshAuthMode::Password,
                    "agent" if key_path.is_empty() => SshAuthMode::Agent,
                    "key" if !key_path.is_empty() => SshAuthMode::KeyFile { key_path },
                    _ => return None,
                };
                let host = SshHostRecord {
                    id: unhex(id)?,
                    name: unhex(name)?,
                    destination: unhex(destination)?,
                    port: port.parse().ok().filter(|port| *port != 0)?,
                    username: unhex(username)?,
                    path: unhex(path)?,
                    auth,
                    notes: String::new(),
                };
                if host.id.is_empty() || host.destination.is_empty() {
                    return None;
                }
                Some(Self::Profile(host))
            }
            [
                "openssh-1",
                alias,
                hostname,
                user,
                port,
                identity,
                fingerprint,
            ] => {
                let host = OpenSshHost {
                    alias: unhex(alias)?,
                    hostname: unhex(hostname)?,
                    user: unhex(user)?,
                    port: port.parse().ok().filter(|port| *port != 0)?,
                    identity_file: {
                        let key = unhex(identity)?;
                        (!key.is_empty()).then_some(key)
                    },
                    config_fingerprint: fingerprint.parse().ok()?,
                };
                if host.alias.is_empty() || host.hostname.is_empty() {
                    return None;
                }
                Some(Self::OpenSsh(host))
            }
            _ => None,
        }
    }
}
