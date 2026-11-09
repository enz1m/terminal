// Corrupt appearance fields are tolerated; identity conflicts and budget
// violations refuse the document rather than choosing a profile's credential owner.

use enzui_core::editable_shortcuts::ShortcutOverrides;
use enzui_core::persist::{InMemoryBackend, Persistable, RawByteLimit, Store, XdgResolver};

use crate::model::{Settings, SshAuthMode, SshHostRecord};

pub(crate) const FORMAT_ID: &str = "terminal-emulator-settings";
pub(crate) const VERSION: u32 = 1;
pub(crate) const RAW_BYTE_LIMIT: RawByteLimit = RawByteLimit::new(1024 * 1024);
const FRAME_BYTES: usize = match RawByteLimit::for_body(FORMAT_ID, VERSION, 0) {
    Ok(frame) => frame.bytes() as usize,
    Err(_) => panic!("settings framing must fit the framework frame ceiling"),
};
pub(crate) const MAX_DOCUMENT_BYTES: usize = RAW_BYTE_LIMIT.bytes() as usize - FRAME_BYTES;
pub(crate) const MAX_HOSTS: usize = 128;
pub(crate) const MAX_HOST_STRING_BYTES: usize = 4096;
pub(crate) const MAX_HOST_TEXT_BYTES: usize = 448 * 1024;
pub(crate) const MAX_SETTING_ID_BYTES: usize = 128;
pub(crate) const MAX_OVERRIDES: usize = 128;
pub(crate) const MAX_ACTION_ID_BYTES: usize = 128;
pub(crate) const MAX_SHORTCUT_KEY_BYTES: usize = 32;
pub(crate) const MAX_OVERRIDE_BODY_BYTES: usize = 64 * 1024;
// Host text is hex-expanded; each record adds eight separators, a newline,
// a five-digit port and at most eight authentication-tag bytes.
const MAX_CANONICAL_BODY_BYTES: usize = b"enzui-terminal-settings-v1\n".len()
    + b"font_size_px\t\ncursor_style\t\nscrollback_lines\t\ncolor_scheme\t\n".len()
    + 2 * 11
    + b"ssh_host_seq\t\n".len()
    + 20
    + 4 * MAX_SETTING_ID_BYTES
    + MAX_OVERRIDE_BODY_BYTES
    + 2 * MAX_HOST_TEXT_BYTES
    + MAX_HOSTS * (b"ssh_host\t".len() + 8 + 1 + 5 + 8);
const _: () = assert!(MAX_CANONICAL_BODY_BYTES <= MAX_DOCUMENT_BYTES);

#[derive(Clone, Debug, Default)]
pub(crate) struct AppPreferences {
    pub settings: Settings,
    pub shortcut_overrides: ShortcutOverrides,
    /// Saved connection metadata only. Password bytes are held by
    /// `credential_store` and are structurally absent from this value.
    pub ssh_hosts: Vec<SshHostRecord>,
    // Retain issued numeric owners after profile deletion; live rows alone
    // cannot prevent a later restart from reissuing a retired credential key.
    pub(crate) ssh_host_seq: u64,
}

impl AppPreferences {
    pub(crate) fn snapshot(
        settings: &Settings,
        shortcut_overrides: &ShortcutOverrides,
        ssh_hosts: &[SshHostRecord],
    ) -> Self {
        Self {
            settings: settings.clone(),
            shortcut_overrides: shortcut_overrides.clone(),
            ssh_hosts: ssh_hosts.to_vec(),
            ssh_host_seq: crate::ssh_host_ids::validate(ssh_hosts).unwrap_or(0),
        }
    }
}

impl AppPreferences {
    fn admitted_overrides(&self) -> Result<String, String> {
        if self.ssh_hosts.len() > MAX_HOSTS
            || self.settings.cursor_style.len() > MAX_SETTING_ID_BYTES
            || self.settings.color_scheme.len() > MAX_SETTING_ID_BYTES
            || self.shortcut_overrides.len() > MAX_OVERRIDES
        {
            return Err("preferences exceed count or setting-id budget".into());
        }
        crate::ssh_host_ids::validate(&self.ssh_hosts)
            .map_err(|error| error.summary().to_owned())?;
        let mut host_bytes = 0_usize;
        for host in &self.ssh_hosts {
            let key = match &host.auth {
                SshAuthMode::KeyFile { key_path } => key_path.as_str(),
                _ => "",
            };
            for text in [
                host.id.as_str(),
                &host.name,
                &host.destination,
                &host.username,
                &host.path,
                key,
                &host.notes,
            ] {
                if text.len() > MAX_HOST_STRING_BYTES {
                    return Err("connection metadata exceeds string budget".into());
                }
                host_bytes = host_bytes
                    .checked_add(text.len())
                    .filter(|bytes| *bytes <= MAX_HOST_TEXT_BYTES)
                    .ok_or("connection metadata exceeds aggregate budget")?;
            }
        }
        for (id, chord) in self.shortcut_overrides.iter() {
            if id.len() > MAX_ACTION_ID_BYTES
                || matches!(chord.key(), enzui_core::ShortcutKey::Character(key) if key.len() > MAX_SHORTCUT_KEY_BYTES)
            {
                return Err("shortcut override exceeds string budget".into());
            }
        }
        let overrides = self.shortcut_overrides.persist_serialize()?;
        if overrides.len() > MAX_OVERRIDE_BODY_BYTES {
            return Err("shortcut overrides exceed aggregate budget".into());
        }
        Ok(overrides)
    }
}

impl Persistable for AppPreferences {
    fn persist_serialize(&self) -> Result<String, String> {
        let overrides = self.admitted_overrides()?;
        let mut body = String::from("enzui-terminal-settings-v1\n");
        body.push_str(&format!(
            "font_size_px\t{}\ncursor_style\t{}\nscrollback_lines\t{}\ncolor_scheme\t{}\n",
            self.settings.clamped_font_size_px(),
            hex(self.settings.cursor_style.as_bytes()),
            self.settings.clamped_scrollback(),
            hex(self.settings.color_scheme.as_bytes()),
        ));
        let sequence = self.ssh_host_seq.max(
            crate::ssh_host_ids::validate(&self.ssh_hosts)
                .map_err(|error| error.summary().to_owned())?,
        );
        body.push_str(&format!("ssh_host_seq\t{sequence}\n"));
        body.push_str(&overrides);
        for host in &self.ssh_hosts {
            let (auth, key_path) = match &host.auth {
                SshAuthMode::Password => ("password", ""),
                SshAuthMode::KeyFile { key_path } => ("key", key_path.as_str()),
                SshAuthMode::Agent => ("agent", ""),
            };
            body.push_str("ssh_host\t");
            body.push_str(
                &[
                    hex(host.id.as_bytes()),
                    hex(host.name.as_bytes()),
                    hex(host.destination.as_bytes()),
                    host.port.to_string(),
                    hex(host.username.as_bytes()),
                    hex(host.path.as_bytes()),
                    auth.to_owned(),
                    hex(key_path.as_bytes()),
                    hex(host.notes.as_bytes()),
                ]
                .join("\t"),
            );
            body.push('\n');
        }
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err("preferences exceed document budget".into());
        }
        Ok(body)
    }

    fn persist_deserialize(body: &str) -> Result<Self, String> {
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err("preferences exceed document budget".into());
        }
        let mut value = Self::default();
        let mut override_lines = String::new();
        let mut host_ids = std::collections::HashSet::new();
        let mut host_records = 0_usize;
        let mut sequence = None;
        for line in body.lines() {
            let Some((field, raw)) = line.split_once('\t') else {
                continue;
            };
            match field {
                "ssh_host_seq" => {
                    if sequence.is_some() {
                        return Err("duplicate saved connection sequence".into());
                    }
                    sequence = Some(
                        raw.parse::<u64>()
                            .map_err(|_| "invalid saved connection sequence")?,
                    );
                }
                "font_size_px" => {
                    if let Ok(px) = raw.parse::<i32>() {
                        value.settings.font_size_px = px;
                    }
                }
                "cursor_style" => {
                    if let Some(style) = unhex(raw) {
                        value.settings.cursor_style = style;
                    }
                }
                "scrollback_lines" => {
                    if let Ok(lines) = raw.parse::<i32>() {
                        value.settings.scrollback_lines = lines;
                    }
                }
                "color_scheme" => {
                    if let Some(id) = unhex(raw) {
                        value.settings.color_scheme = id;
                    }
                }
                "override" => {
                    override_lines.push_str(line);
                    override_lines.push('\n');
                }
                "ssh_host" => {
                    host_records += 1;
                    if host_records > MAX_HOSTS {
                        return Err("preferences exceed connection count budget".into());
                    }
                    let fields: Vec<&str> = raw.splitn(10, '\t').collect();
                    let id = fields
                        .first()
                        .filter(|id| id.len() <= MAX_HOST_STRING_BYTES * 2)
                        .and_then(|id| unhex(id))
                        .ok_or("invalid saved connection ID")?;
                    crate::ssh_host_ids::numeric_id(&id)
                        .map_err(|error| error.summary().to_owned())?;
                    if !host_ids.insert(id.clone()) {
                        return Err("duplicate saved connection ID".into());
                    }
                    if fields.len() == 9
                        && let (
                            Some(name),
                            Some(destination),
                            Ok(port),
                            Some(username),
                            Some(path),
                            Some(key_path),
                            Some(notes),
                        ) = (
                            unhex(fields[1]),
                            unhex(fields[2]),
                            fields[3].parse::<u16>(),
                            unhex(fields[4]),
                            unhex(fields[5]),
                            unhex(fields[7]),
                            unhex(fields[8]),
                        )
                    {
                        let auth = match fields[6] {
                            "password" => SshAuthMode::Password,
                            "key" => SshAuthMode::KeyFile { key_path },
                            "agent" => SshAuthMode::Agent,
                            _ => return Err("invalid saved connection authentication mode".into()),
                        };
                        value.ssh_hosts.push(SshHostRecord {
                            id,
                            name,
                            destination,
                            port,
                            username,
                            path,
                            auth,
                            notes,
                        });
                    } else {
                        return Err("invalid saved connection record".into());
                    }
                }
                _ => {}
            }
        }
        value.shortcut_overrides = ShortcutOverrides::persist_deserialize(&override_lines)?;
        value.admitted_overrides()?;
        let minimum = crate::ssh_host_ids::validate(&value.ssh_hosts)
            .map_err(|error| error.summary().to_owned())?;
        if sequence.is_some_and(|sequence| sequence < minimum) {
            return Err("saved connection sequence precedes a live owner".into());
        }
        value.ssh_host_seq = sequence.unwrap_or(minimum);
        Ok(value)
    }

    fn persist_default() -> Self {
        Self::default()
    }
}

pub(crate) fn store() -> Store<AppPreferences> {
    #[cfg(test)]
    {
        Store::new(InMemoryBackend::new(), FORMAT_ID, VERSION)
    }
    #[cfg(not(test))]
    {
        store_with_resolver(&XdgResolver::from_env())
    }
}

pub(crate) fn store_with_resolver(resolver: &XdgResolver) -> Store<AppPreferences> {
    use enzui_core::persist::{FileBackend, PersistenceKind};
    match FileBackend::xdg_with_resolver(
        resolver,
        PersistenceKind::Config,
        "terminal-emulator",
        "settings.json",
        RAW_BYTE_LIMIT,
    )
    .expect("the static settings XDG descendants are valid")
    {
        Some(backend) => Store::new(backend, FORMAT_ID, VERSION),
        None => Store::new(InMemoryBackend::new(), FORMAT_ID, VERSION),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Option<String> {
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
mod tests {

    use enzui_core::persist::{BackendLoad, PersistBackend};

    struct FailingBackend;
    impl PersistBackend for FailingBackend {
        fn load_raw(&self, _: &str, _: u32) -> BackendLoad {
            BackendLoad::Absent
        }
        fn save_raw(
            &self,
            _: &str,
            _: u32,
            _: &str,
        ) -> Result<(), enzui_core::persist::PersistError> {
            Err(enzui_core::persist::PersistError::new("disk full"))
        }
    }
}
