use crate::{
    pane_endpoint::PaneEndpoint,
    pinned_output::{MAX_COMMANDS, MAX_ROW_BYTES, MAX_ROWS, PinnedOutput},
    session_store::SavedSession,
};
use enzui_core::form::SecretText;
use enzui_services::credential_store::{CredentialKey, Secret};
use std::{collections::HashMap, sync::Arc};

const MAGIC: &[u8] = b"enzui-pinned-output-1\0";
pub(crate) const MAX_SECRET_BYTES: usize = 1024 * 1024;
pub(crate) const NOT_SAVED: &str = "Pinned, output not saved securely";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Marker {
    pub(crate) bound: usize,
    pub(crate) rows: usize,
    pub(crate) commands: usize,
    pub(crate) bytes: usize,
    pub(crate) checksum: u64,
}
impl Marker {
    pub(crate) fn valid(&self) -> bool {
        self.bound <= MAX_ROWS
            && self.rows <= self.bound * crate::pane_layout::MAX_PANES
            && self.commands <= self.bound.min(MAX_COMMANDS) * crate::pane_layout::MAX_PANES
            && self.rows <= MAX_ROWS * crate::pane_layout::MAX_PANES
            && self.commands <= MAX_COMMANDS * crate::pane_layout::MAX_PANES
            && self.bytes <= MAX_SECRET_BYTES
            && self.bytes >= MAGIC.len()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    pub(crate) key: String,
    pub(crate) marker: Option<Marker>,
}
impl Reference {
    pub(crate) fn new() -> Option<Self> {
        enzui_core::random::random_array::<16>()
            .ok()
            .map(|bytes| Self {
                key: crate::session_store::hex(&bytes),
                marker: None,
            })
    }
    pub(crate) fn valid(&self) -> bool {
        self.key.len() == 32
            && self.key.bytes().all(|byte| byte.is_ascii_hexdigit())
            && self.marker.as_ref().is_none_or(Marker::valid)
    }
    pub(crate) fn credential_key(&self) -> CredentialKey {
        CredentialKey::new(
            crate::ssh_credentials::APP_ID,
            &self.key,
            "pinned-output-v1",
        )
    }
}

pub(crate) struct PanePayload {
    pub(crate) id: String,
    pub(crate) endpoint: PaneEndpoint,
    pub(crate) cwd: SecretText,
    pub(crate) output: Arc<PinnedOutput>,
}
impl std::fmt::Debug for PanePayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PanePayload([redacted])")
    }
}
#[derive(Clone)]
pub(crate) struct Restored {
    pub(crate) panes: HashMap<String, (Arc<PinnedOutput>, SecretText)>,
}
impl std::fmt::Debug for Restored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Restored([redacted])")
    }
}

// This checksum detects stale or corrupt records; confidentiality is the credential backend's responsibility.
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
struct Buffer(Vec<u8>);
impl Drop for Buffer {
    fn drop(&mut self) {
        drop(Secret::new(std::mem::take(&mut self.0)));
    }
}
impl Buffer {
    fn append(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|len| len > MAX_SECRET_BYTES)
        {
            return Err(());
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    fn number(&mut self, value: usize) -> Result<(), ()> {
        self.append(&u32::try_from(value).map_err(|_| ())?.to_le_bytes())
    }
    fn text(&mut self, text: &str) -> Result<(), ()> {
        self.number(text.len())?;
        self.append(text.as_bytes())
    }
    fn finish(mut self) -> Secret {
        Secret::new(std::mem::take(&mut self.0))
    }
}
pub(crate) fn seal(
    session: &str,
    reference: &Reference,
    bound: usize,
    panes: &[PanePayload],
) -> Result<(Secret, Marker), ()> {
    if !reference.valid() || bound > MAX_ROWS || panes.len() > crate::pane_layout::MAX_PANES {
        return Err(());
    }
    // Fixed capacity prevents reallocations from leaving previous plaintext buffers behind.
    let mut bytes = Buffer(Vec::with_capacity(MAX_SECRET_BYTES));
    bytes.append(MAGIC)?;
    bytes.text(&reference.key)?;
    bytes.text(session)?;
    bytes.number(bound)?;
    bytes.number(panes.len())?;
    let mut rows = 0;
    let mut commands = 0;
    for pane in panes {
        bytes.text(&pane.id)?;
        bytes.text(&pane.endpoint.encode())?;
        if pane.cwd.expose_secret().len() > MAX_ROW_BYTES
            || !crate::pinned_output::plain_text(pane.cwd.expose_secret())
        {
            return Err(());
        }
        bytes.text(pane.cwd.expose_secret())?;
        let output = pane.output.bounded(bound);
        bytes.number(output.rows().count())?;
        for row in output.rows() {
            bytes.number(row.columns)?;
            bytes.text(row.text.expose_secret())?;
            rows += 1;
        }
        bytes.number(output.commands().count())?;
        for command in output.commands() {
            bytes.append(&command.exit.to_le_bytes())?;
            bytes.text(command.text.expose_secret())?;
            commands += 1;
        }
    }
    let marker = Marker {
        bound,
        rows,
        commands,
        bytes: bytes.0.len(),
        checksum: checksum(&bytes.0),
    };
    Ok((bytes.finish(), marker))
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], ()> {
        let bytes = self.0.get(..len).ok_or(())?;
        self.0 = &self.0[len..];
        Ok(bytes)
    }
    fn number(&mut self) -> Result<usize, ()> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ())?) as usize)
    }
    fn text(&mut self, max: usize) -> Result<&'a str, ()> {
        let len = self.number()?;
        if len > max {
            return Err(());
        }
        std::str::from_utf8(self.take(len)?).map_err(|_| ())
    }
}
pub(crate) fn open(
    secret: &Secret,
    session: &SavedSession,
    reference: &Reference,
) -> Result<Restored, ()> {
    if session.panes.len() > crate::pane_layout::MAX_PANES {
        return Err(());
    }
    let marker = reference
        .marker
        .as_ref()
        .filter(|marker| marker.valid())
        .ok_or(())?;
    if secret.len() != marker.bytes
        || secret.len() > MAX_SECRET_BYTES
        || checksum(secret.expose_secret()) != marker.checksum
    {
        return Err(());
    }
    let mut reader = Reader(secret.expose_secret());
    if reader.take(MAGIC.len())? != MAGIC
        || reader.text(32)? != reference.key
        || reader.text(8192)? != session.id
        || reader.number()? != marker.bound
        || reader.number()? != session.panes.len()
    {
        return Err(());
    }
    let mut panes = HashMap::new();
    let mut rows = 0;
    let mut commands = 0;
    for pane in &session.panes {
        if reader.text(8192)? != pane.id || reader.text(64 * 1024)? != pane.endpoint.encode() {
            return Err(());
        }
        let cwd = reader.text(MAX_ROW_BYTES)?;
        if !crate::pinned_output::plain_text(cwd) {
            return Err(());
        }
        let cwd = SecretText::new(cwd.to_owned());
        let count = reader.number()?;
        if count > marker.bound {
            return Err(());
        }
        let mut output = PinnedOutput::default();
        for _ in 0..count {
            let columns = reader.number()?;
            let text = reader.text(MAX_ROW_BYTES)?;
            output.read_row(columns, text.to_owned()).map_err(|_| ())?;
            rows += 1;
        }
        let count = reader.number()?;
        if count > marker.bound.min(MAX_COMMANDS) {
            return Err(());
        }
        for _ in 0..count {
            let exit = i32::from_le_bytes(reader.take(4)?.try_into().map_err(|_| ())?);
            let text = reader.text(crate::shell_history::MAX_COMMAND_BYTES)?;
            output.read_command(text.to_owned(), exit).map_err(|_| ())?;
            commands += 1;
        }
        if panes
            .insert(pane.id.clone(), (Arc::new(output), cwd))
            .is_some()
        {
            return Err(());
        }
    }
    if !reader.0.is_empty() || rows != marker.rows || commands != marker.commands {
        return Err(());
    }
    Ok(Restored { panes })
}
