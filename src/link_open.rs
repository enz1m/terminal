//! Detect and open terminal hyperlinks.
//!
//! The framework search for `hyperlink`, `URL click to open`, `open URL`,
//! `OpenURI`, and `path detection` found pointer reporting in `text_grid` and a
//! styled prose `link`, but no terminal detector. This app keeps terminal-
//! specific token detection here, routes URI strings through core's cancellable
//! OpenURI owner, and opens retained local paths through the same shared desktop route.
//! Tests inject both transports and never contact the host desktop.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use enzui_core::activation::ActivationContext;
use enzui_core::io_class::IoFailureCategory;
use enzui_core::portal::open_uri::{OpenUri, OpenUriBackend, OpenUriRequest, PendingOpenUri};
use enzui_core::portal::{CancelReason, PortalOutcome, PortalParent};

use crate::model::TranscriptLine;

/// Something Ctrl+click can ask the desktop to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenTarget {
    Uri(String),
    Path(PathBuf),
}

/// Authored categories that may cross from the opener into visible state.
/// Neither this enum nor its summaries retain target or backend text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkOpenFailureKind {
    InvalidLink,
    LinkCancelled,
    LinkDenied,
    LinkNotDeclared,
    LinkUnsupported,
    LinkUnavailable,
    LinkParentUnavailable,
    LinkFailed,
    FileMissing,
    FilePermission,
    FileUnavailable,
}

impl LinkOpenFailureKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InvalidLink => "invalid-link",
            Self::LinkCancelled => "link-cancelled",
            Self::LinkDenied => "link-denied",
            Self::LinkNotDeclared => "link-not-declared",
            Self::LinkUnsupported => "link-unsupported",
            Self::LinkUnavailable => "link-unavailable",
            Self::LinkParentUnavailable => "link-parent-unavailable",
            Self::LinkFailed => "link-failed",
            Self::FileMissing => "file-missing",
            Self::FilePermission => "file-permission",
            Self::FileUnavailable => "file-unavailable",
        }
    }

    fn user_summary(self) -> &'static str {
        match self {
            Self::InvalidLink => "This link is not a valid address.",
            Self::LinkCancelled => "Opening the link was cancelled.",
            Self::LinkDenied => "Permission to open the link was denied.",
            Self::LinkNotDeclared => "Opening links is not enabled for this build.",
            Self::LinkUnsupported => "This desktop does not support opening links.",
            Self::LinkUnavailable => "No desktop link opener is available.",
            Self::LinkParentUnavailable => "The link opener lost its parent window.",
            Self::LinkFailed => "The link could not be opened.",
            Self::FileMissing => "The selected file could not be found.",
            Self::FilePermission => {
                "The selected file could not be opened because permission was denied."
            }
            Self::FileUnavailable => "The selected file could not be opened.",
        }
    }
}

/// A service-boundary failure with separate user and diagnostic projections.
///
/// `diagnostic` is escaped when the error is constructed. Custom `Debug`
/// deliberately exposes only the authored kind, preventing an incidental
/// `{:?}` from turning hostile target/backend text into semantic copy.
#[derive(Clone)]
pub(crate) struct LinkOpenError {
    kind: LinkOpenFailureKind,
    diagnostic: String,
}

impl LinkOpenError {
    fn new(kind: LinkOpenFailureKind, target: &OpenTarget, error: &dyn fmt::Display) -> Self {
        let target = match target {
            OpenTarget::Uri(uri) => uri.clone(),
            OpenTarget::Path(path) => path.display().to_string(),
        };
        let detail = format!("target=\"{target}\" error=\"{error}\"");
        Self {
            kind,
            diagnostic: crate::failure_translation::escaped_diagnostic(&detail),
        }
    }

    fn invalid_uri(uri: String, error: &dyn fmt::Display) -> Self {
        Self::new(
            LinkOpenFailureKind::InvalidLink,
            &OpenTarget::Uri(uri),
            error,
        )
    }

    fn file_io(path: &Path, error: &std::io::Error) -> Self {
        let kind = match IoFailureCategory::classify(error) {
            IoFailureCategory::NotFound | IoFailureCategory::NotADirectory => {
                LinkOpenFailureKind::FileMissing
            }
            IoFailureCategory::Permission => LinkOpenFailureKind::FilePermission,
            IoFailureCategory::StorageExhausted | IoFailureCategory::Unknown => {
                LinkOpenFailureKind::FileUnavailable
            }
        };
        Self::new(kind, &OpenTarget::Path(path.to_path_buf()), error)
    }

    pub(crate) fn opener(target: &OpenTarget, error: &dyn fmt::Display) -> Self {
        let kind = match target {
            OpenTarget::Uri(_) => LinkOpenFailureKind::LinkFailed,
            OpenTarget::Path(_) => LinkOpenFailureKind::FileUnavailable,
        };
        Self::new(kind, target, error)
    }

    fn uri_outcome(uri: &str, outcome: PortalOutcome<()>) -> Result<(), Self> {
        let kind = match outcome {
            PortalOutcome::Completed(()) => return Ok(()),
            PortalOutcome::Cancelled(_) => LinkOpenFailureKind::LinkCancelled,
            PortalOutcome::Denied => LinkOpenFailureKind::LinkDenied,
            PortalOutcome::NotDeclared => LinkOpenFailureKind::LinkNotDeclared,
            PortalOutcome::Unsupported => LinkOpenFailureKind::LinkUnsupported,
            PortalOutcome::Unavailable => LinkOpenFailureKind::LinkUnavailable,
            PortalOutcome::ParentUnavailable => LinkOpenFailureKind::LinkParentUnavailable,
            PortalOutcome::Failed => LinkOpenFailureKind::LinkFailed,
        };
        Err(Self::new(
            kind,
            &OpenTarget::Uri(uri.to_owned()),
            &kind.as_str(),
        ))
    }

    pub(crate) fn kind(&self) -> LinkOpenFailureKind {
        self.kind
    }

    pub(crate) fn user_summary(&self) -> &'static str {
        self.kind.user_summary()
    }

    pub(crate) fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}

impl fmt::Debug for LinkOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinkOpenError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Validate a URL without normalising its bytes. Both production and injected
/// opener tests use this boundary, so malformed/control-bearing URLs never
/// reach a desktop backend.
pub(crate) fn validated_uri(uri: String) -> Result<OpenUri, LinkOpenError> {
    OpenUri::parse(uri.clone()).map_err(|error| LinkOpenError::invalid_uri(uri, &error))
}

/// The descriptor-owning local-path seam. Implementations must not run until
/// the returned future is polled by `Task::perform`. URI requests deliberately
/// bypass this seam and use [`UriPortal`] so dropping an iced task cannot lose
/// the portal's explicit close obligation.
pub(crate) trait LinkOpener: Send + Sync {
    fn open_path(
        &self,
        path: PathBuf,
    ) -> Pin<Box<dyn Future<Output = Result<(), LinkOpenError>> + Send + '_>>;
}

/// Retain a local file or directory on the bounded blocking pool, then let
/// the framework select the native desktop or sandboxed portal route.
pub(crate) struct PortalLinkOpener {
    blocking: Arc<crate::blocking_dispatch::Dispatch>,
}

impl PortalLinkOpener {
    /// Opens local paths through `blocking` and the desktop portal.
    pub(crate) fn new(blocking: Arc<crate::blocking_dispatch::Dispatch>) -> Self {
        Self { blocking }
    }
}

impl LinkOpener for PortalLinkOpener {
    fn open_path(
        &self,
        path: PathBuf,
    ) -> Pin<Box<dyn Future<Output = Result<(), LinkOpenError>> + Send + '_>> {
        let blocking = Arc::clone(&self.blocking);
        Box::pin(async move {
            use enzui_core::portal::open_uri::{
                OpenDirectoryRequest, OpenFileRequest, RetainedDirectory, RetainedFile,
                desktop_launcher::host_open_uri_backend,
            };

            let target = OpenTarget::Path(path.clone());
            enum RetainedPath {
                Directory(RetainedDirectory),
                File(RetainedFile),
            }
            let worker_target = target.clone();
            let opened = blocking.submit_fifo(move |_| {
                let file = std::fs::File::open(&path)
                    .map_err(|error| LinkOpenError::file_io(&path, &error))?;
                let metadata = file
                    .metadata()
                    .map_err(|error| LinkOpenError::file_io(&path, &error))?;
                if metadata.is_dir() {
                    RetainedDirectory::new(file)
                        .map(RetainedPath::Directory)
                        .map_err(|error| LinkOpenError::opener(&worker_target, &error))
                } else {
                    RetainedFile::new(file)
                        .map(RetainedPath::File)
                        .map_err(|error| LinkOpenError::opener(&worker_target, &error))
                }
            });
            let retained = match opened.await.into_result() {
                Ok(opened) => opened?,
                Err(refusal) => return Err(LinkOpenError::opener(&target, &refusal)),
            };
            let mut backend = host_open_uri_backend();
            // Snapshot before starting: a fast completion must not be lost.
            let signal = backend
                .open_uri_completion_signal()
                .ok_or_else(|| LinkOpenError::opener(&target, &"missing completion signal"))?;
            let mut seen = signal.generation();
            let started = match retained {
                RetainedPath::Directory(directory) => {
                    OpenDirectoryRequest::new(directory, PortalParent::None)
                        .start(&mut *backend, |_| true)
                }
                RetainedPath::File(file) => {
                    OpenFileRequest::new(file, PortalParent::None).start(&mut *backend, |_| true)
                }
            };
            let pending = started
                .map_err(|outcome| LinkOpenError::opener(&target, &format!("{outcome:?}")))?;
            let mut request = PathRequest { backend, pending };
            loop {
                if let Some(outcome) = request.pending.poll(&mut *request.backend) {
                    return match outcome {
                        PortalOutcome::Completed(()) => Ok(()),
                        other => Err(LinkOpenError::opener(&target, &format!("{other:?}"))),
                    };
                }
                seen = signal.wait_after(seen).await;
            }
        })
    }
}

// Dropping an aborted app task must settle the framework's close obligation
// before its backend and retained descriptor disappear.
struct PathRequest {
    backend: Box<dyn OpenUriBackend + Send>,
    pending: PendingOpenUri,
}

impl Drop for PathRequest {
    fn drop(&mut self) {
        let _ = self.pending.cancel(CancelReason::Application);
        let _ = self.pending.settle(&mut *self.backend);
    }
}

/// Stable ownership of one URI request. Pane identity is globally unique in
/// the app today, but retaining the session too makes teardown intent explicit
/// and prevents a future restored pane id from aliasing another session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct UriOwner {
    pub(crate) session: String,
    pub(crate) pane: String,
}

/// One completion edge tagged with the exact app-side request generation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct UriWake {
    owner: UriOwner,
    generation: u64,
}

impl UriWake {
    pub(crate) fn owner(&self) -> &UriOwner {
        &self.owner
    }
}

#[derive(Debug)]
pub(crate) enum UriWakeVerdict {
    Stale,
    Pending,
    Finished(Result<(), LinkOpenError>),
}

struct LiveUri {
    uri: String,
    generation: u64,
    pending: PendingOpenUri,
}

/// Keyed ownership for every live framework OpenURI request.
///
/// A new link supersedes only the same pane. Closing a pane or session removes
/// and settles only matching entries, while [`Drop`] settles all remaining
/// entries before the shared backend is released.
pub(crate) struct UriPortal {
    backend: Option<Box<dyn OpenUriBackend>>,
    live: HashMap<UriOwner, LiveUri>,
    next_generation: u64,
}

impl Default for UriPortal {
    fn default() -> Self {
        Self::new()
    }
}

impl UriPortal {
    pub(crate) fn new() -> Self {
        Self {
            backend: None,
            live: HashMap::new(),
            next_generation: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_backend(backend: Box<dyn OpenUriBackend>) -> Self {
        Self {
            backend: Some(backend),
            live: HashMap::new(),
            next_generation: 0,
        }
    }

    /// Starts one byte-preserving URI request with an explicitly unparented
    /// portal policy. Terminal owns pane/session lifetime rather than a single
    /// window: a pane can move between the main and detached windows, so
    /// pretending one current window is its permanent parent would be stale.
    #[cfg(test)]
    pub(crate) fn start(
        &mut self,
        owner: UriOwner,
        raw_uri: String,
    ) -> Result<UriWake, LinkOpenError> {
        self.start_with_activation(owner, raw_uri, None)
    }

    pub(crate) fn start_with_activation(
        &mut self,
        owner: UriOwner,
        raw_uri: String,
        activation: Option<ActivationContext>,
    ) -> Result<UriWake, LinkOpenError> {
        let uri = validated_uri(raw_uri.clone())?;
        self.cancel_owner(&owner, CancelReason::Application);
        let Some(backend) = self.backend_mut() else {
            return Err(
                LinkOpenError::uri_outcome(&raw_uri, PortalOutcome::Unavailable)
                    .expect_err("unavailable is a failure"),
            );
        };
        let request = match activation {
            Some(activation) => {
                OpenUriRequest::new(uri, PortalParent::None).with_activation(activation)
            }
            None => OpenUriRequest::new(uri, PortalParent::None),
        };
        let pending = match request.start(&mut **backend, |_| true) {
            Ok(pending) => pending,
            Err(outcome) => {
                return Err(LinkOpenError::uri_outcome(&raw_uri, outcome)
                    .expect_err("a start refusal cannot be successful"));
            }
        };
        self.next_generation = self.next_generation.wrapping_add(1);
        let wake = UriWake {
            owner: owner.clone(),
            generation: self.next_generation,
        };
        self.live.insert(
            owner,
            LiveUri {
                uri: raw_uri,
                generation: wake.generation,
                pending,
            },
        );
        Ok(wake)
    }

    pub(crate) fn pump(&mut self, wake: &UriWake) -> UriWakeVerdict {
        let Some(live) = self.live.get_mut(&wake.owner) else {
            return UriWakeVerdict::Stale;
        };
        if live.generation != wake.generation {
            return UriWakeVerdict::Stale;
        }
        let backend = self
            .backend
            .as_deref_mut()
            .expect("a live URI request always owns a backend");
        let outcome = live
            .pending
            .poll(backend)
            .or_else(|| live.pending.cancel_reason().map(PortalOutcome::Cancelled));
        let _ = live.pending.settle(backend);
        let Some(outcome) = outcome else {
            return UriWakeVerdict::Pending;
        };
        let uri = live.uri.clone();
        self.live.remove(&wake.owner);
        UriWakeVerdict::Finished(LinkOpenError::uri_outcome(&uri, outcome))
    }

    pub(crate) fn completion_subscription(&self) -> iced::Subscription<UriWake> {
        let Some(signal) = self
            .backend
            .as_deref()
            .and_then(OpenUriBackend::open_uri_completion_signal)
        else {
            return iced::Subscription::none();
        };
        iced::Subscription::batch(
            self.live
                .iter()
                .filter(|&(_owner, live)| live.pending.is_unsettled())
                .map(|(owner, live)| {
                    let wake = UriWake {
                        owner: owner.clone(),
                        generation: live.generation,
                    };
                    enzui_widgets::file_chooser::completion_signal_subscription(
                        live.generation,
                        signal.clone(),
                    )
                    .with(wake)
                    .map(|(wake, _)| wake)
                }),
        )
    }

    pub(crate) fn cancel_owner(&mut self, owner: &UriOwner, reason: CancelReason) {
        let Some(mut live) = self.live.remove(owner) else {
            return;
        };
        let _ = live.pending.cancel(reason);
        if let Some(backend) = self.backend.as_deref_mut() {
            let _ = live.pending.settle(backend);
        }
    }

    pub(crate) fn cancel_session(&mut self, session: &str, reason: CancelReason) {
        let owners: Vec<UriOwner> = self
            .live
            .keys()
            .filter(|owner| owner.session == session)
            .cloned()
            .collect();
        for owner in owners {
            self.cancel_owner(&owner, reason);
        }
    }

    fn cancel_all(&mut self, reason: CancelReason) {
        let owners: Vec<UriOwner> = self.live.keys().cloned().collect();
        for owner in owners {
            self.cancel_owner(&owner, reason);
        }
    }

    fn backend_mut(&mut self) -> Option<&mut Box<dyn OpenUriBackend>> {
        if self.backend.is_none() {
            self.backend =
                Some(enzui_core::portal::open_uri::desktop_launcher::host_open_uri_backend());
        }
        self.backend.as_mut()
    }

    #[cfg(test)]
    pub(crate) fn live_wake(&self, owner: &UriOwner) -> Option<UriWake> {
        self.live.get(owner).map(|live| UriWake {
            owner: owner.clone(),
            generation: live.generation,
        })
    }
}

impl Drop for UriPortal {
    fn drop(&mut self) {
        self.cancel_all(CancelReason::Application);
    }
}

/// Tests default to a loud refusal. A gate must explicitly inject a recorder;
/// accidentally exercising the production portal from `cargo test` is a bug.
#[cfg(test)]
pub(crate) struct UnavailableLinkOpener;

#[cfg(test)]
impl LinkOpener for UnavailableLinkOpener {
    fn open_path(
        &self,
        path: PathBuf,
    ) -> Pin<Box<dyn Future<Output = Result<(), LinkOpenError>> + Send + '_>> {
        Box::pin(async move {
            Err(LinkOpenError::opener(
                &OpenTarget::Path(path),
                &"link opener unavailable in tests",
            ))
        })
    }
}

/// Resolve the link under a terminal *column*. Explicit OSC 8 metadata wins;
/// otherwise the whitespace-delimited token is classified as HTTP(S), `www`,
/// `file://`, or a local path. Remote-pane paths are deliberately not opened
/// against the local filesystem, while web URIs remain useful over SSH.
///
/// `link_generation` is the pane's live screen authority. An OSC 8 range is
/// only honoured while it matches, so a row retained across a screen reset,
/// an alternate-screen swap or a replacement session keeps its visible text
/// and yields no target. Detected tokens are visible text and stay resolvable.
pub(crate) fn target_at(
    line: &TranscriptLine,
    column: usize,
    cwd: &Path,
    home: Option<&Path>,
    allow_local_path: bool,
    link_generation: u64,
) -> Option<OpenTarget> {
    if let Some(link) = current_link_at(line, column, link_generation) {
        return classify_token(&link.uri, cwd, home, allow_local_path);
    }

    let token = token_at(line, column)?;
    classify_token(token, cwd, home, allow_local_path)
}

/// Whether hover should use a link pointer.
///
/// Deliberately cheaper than [`target_at`]: it needs no cwd, no filesystem
/// access and no link authority, so a retired OSC 8 range still reads as a
/// pointer while activation refuses it. Activation performs the authority and
/// local/remote decisions.
#[cfg(test)]
pub(crate) fn candidate_at(line: &TranscriptLine, column: usize) -> bool {
    line.hyperlinks
        .iter()
        .any(|link| column >= link.start_col && column < link.end_col)
        || token_at(line, column).is_some_and(looks_like_target)
}

fn current_link_at(
    line: &TranscriptLine,
    column: usize,
    link_generation: u64,
) -> Option<&crate::model::TranscriptHyperlink> {
    // A zero generation is "no live authority": a pane with no running screen
    // never opens retained link metadata.
    if link_generation == 0 {
        return None;
    }
    line.hyperlinks.iter().find(|link| {
        link.generation == link_generation && column >= link.start_col && column < link.end_col
    })
}

fn token_at(line: &TranscriptLine, column: usize) -> Option<&str> {
    if column >= line.cells.len() || line.cells[column].blank {
        return None;
    }

    // Plain detected links are ASCII-shaped in practice. Mapping through the
    // cell vector (rather than byte offsets in `line.text`) keeps the pointer's
    // terminal-column coordinate authoritative.
    let boundary = |col: usize| {
        line.cells[col]
            .text
            .as_str()
            .chars()
            .next()
            .is_none_or(|ch| {
                ch.is_whitespace()
                    || matches!(
                        ch,
                        '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}'
                    )
            })
    };
    let mut start = column;
    while start > 0 && !boundary(start - 1) {
        start -= 1;
    }
    let mut end = column + 1;
    while end < line.cells.len() && !boundary(end) {
        end += 1;
    }

    // Cell columns and UTF-8 byte offsets differ, so reconstruct the token's
    // byte range from cells. Plain links are ASCII; reject anything where that
    // one-byte-per-column invariant does not hold rather than opening the wrong
    // substring.
    let before: usize = line.cells[..start].iter().map(|cell| cell.text.len()).sum();
    let through: usize = line.cells[start..end]
        .iter()
        .map(|cell| cell.text.len())
        .sum();
    let raw = line.text.get(before..before + through)?;
    let trimmed = raw.trim_end_matches([',', '.', ';', '!', '?']);
    (!trimmed.is_empty()).then_some(trimmed)
}

fn looks_like_target(token: &str) -> bool {
    token.starts_with("https://")
        || token.starts_with("http://")
        || token.starts_with("file://")
        || token.starts_with("www.")
        || token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/")
        || token.contains('/')
        || token.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
}

fn classify_token(
    token: &str,
    cwd: &Path,
    home: Option<&Path>,
    allow_local_path: bool,
) -> Option<OpenTarget> {
    if token.starts_with("https://") || token.starts_with("http://") {
        return Some(OpenTarget::Uri(token.to_owned()));
    }
    if token.starts_with("www.") {
        return Some(OpenTarget::Uri(format!("https://{token}")));
    }
    if let Some(encoded) = token.strip_prefix("file://") {
        let path = percent_decode(encoded)?;
        return allow_local_path.then(|| OpenTarget::Path(PathBuf::from(path)));
    }
    if !allow_local_path || !looks_like_target(token) {
        return None;
    }

    // Compiler/test output commonly appends `:line` or `:line:column`.
    let token = strip_line_suffix(token);
    let path = if let Some(rest) = token.strip_prefix("~/") {
        home?.join(rest)
    } else {
        let path = Path::new(token);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        }
    };
    Some(OpenTarget::Path(path))
}

fn strip_line_suffix(mut token: &str) -> &str {
    for _ in 0..2 {
        let Some((prefix, suffix)) = token.rsplit_once(':') else {
            break;
        };
        if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
            break;
        }
        token = prefix;
    }
    token
}

fn percent_decode(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = bytes.get(i + 1..i + 3)?;
            let text = std::str::from_utf8(pair).ok()?;
            out.push(u8::from_str_radix(text, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
