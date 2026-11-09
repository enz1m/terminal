use enzui_core::portal::open_uri::OpenUri;
use std::path::{Path, PathBuf};

pub(crate) const MAX_PATH_BYTES: usize = 4096;
const NAME: &str = "enzui-cwd";
const HOST: &str = "localhost";

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ReportedCwd(PathBuf);

impl std::fmt::Debug for ReportedCwd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReportedCwd([redacted])")
    }
}

impl ReportedCwd {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn location(&self, kind: crate::model::SessionKind, remote_hint: &str) -> String {
        match kind {
            crate::model::SessionKind::Local => path_label(self.path()),
            crate::model::SessionKind::Ssh => {
                let host = remote_hint.rsplit('@').next().unwrap_or(remote_hint);
                match plain_path(self.path()) {
                    Some(path) => format!("{host}:{path}"),
                    None => format!("{host}:[directory unavailable for SSH reuse]"),
                }
            }
        }
    }
}

pub(crate) fn from_osc(fields: &[String]) -> Option<ReportedCwd> {
    let [name, host, hex] = fields else {
        return None;
    };
    if name != NAME
        || host != HOST
        || hex.is_empty()
        || hex.len() > MAX_PATH_BYTES * 2
        || !hex.len().is_multiple_of(2)
    {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    };
    let bytes: Option<Vec<u8>> = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Some(nibble(pair[0])? * 16 + nibble(pair[1])?))
        .collect();
    let bytes = bytes?;
    if bytes.first() != Some(&b'/') || bytes.contains(&0) {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(ReportedCwd(PathBuf::from(std::ffi::OsString::from_vec(
            bytes,
        ))))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes)
            .ok()
            .map(|path| ReportedCwd(PathBuf::from(path)))
    }
}

fn plain_path(path: &Path) -> Option<&str> {
    path.to_str()
        .filter(|text| text.trim() == *text && crate::pinned_output::plain_text(text))
}

// Encoded labels also act as lossless local launch locators; literal '%' paths remain distinct.
pub(crate) fn path_label(path: &Path) -> String {
    if let Some(text) = plain_path(path) {
        return text.to_owned();
    }
    OpenUri::from_file_path(path).map_or_else(
        |_| "[unavailable directory]".into(),
        |uri| uri.as_str().to_owned(),
    )
}

pub(crate) fn local_path(locator: &str) -> Option<PathBuf> {
    if !locator.starts_with("file://") || locator.len() > MAX_PATH_BYTES * 3 + 32 {
        return None;
    }
    let path = OpenUri::parse(locator).ok()?.local_file_path()?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if path.as_os_str().as_bytes().len() > MAX_PATH_BYTES {
            return None;
        }
    }
    Some(path)
}

#[cfg(all(test, target_os = "linux"))]
#[path = "shell_cwd_tests.rs"]
mod tests;
