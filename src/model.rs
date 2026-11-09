// No module-level `#![allow(dead_code)]`. Each genuinely-unused item
// below carries a targeted marker and a reason.
//! Domain data model for the terminal emulator.
//!
//! The reducer lives in the app's `update` fn and the projection is the widget
//! `view`, so this module keeps only the pure state records that
//! `emulator.rs` and the app share.

use enzui_core::form::{ChoiceOption, FieldValue, FormField, FormModel, FormSubmission};
use enzui_core::grapheme_cell::InlineGrapheme;
use enzui_widgets::find_surface::FindSurfaceEvent;
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use termwiz::cell::unicode_column_width;
use unicode_segmentation::UnicodeSegmentation;

use crate::pane_layout::PaneLayout;

/// Instrumentation counters for the performance contract.
///
/// A resize reflow (`resize_active_pty`) is expensive by nature: it re-sizes
/// the PTY surface and forces a reprojection. The `ContentResized` guard is
/// supposed to collapse a drag — a storm of pixel-by-pixel window events — into
/// **one reflow per genuinely new cell size**. This counter makes that
/// observable: the perf gate feeds two identical resize events and asserts the
/// reflow counter moved once, with the positive control that a different size
/// moves it again. A clock would flake on a shared build box; a counter does
/// not. Thread-local, because the perf tests run in parallel in the same
/// binary and each drives its own reflows on its own thread.
pub(crate) mod perf {
    use std::cell::Cell;

    thread_local! {
        static PTY_REFLOWS: Cell<u64> = const { Cell::new(0) };
        #[cfg(test)]
        static TERMINAL_SURFACES: Cell<u64> = const { Cell::new(0) };
    }

    pub(crate) fn note_pty_reflow() {
        let _ = PTY_REFLOWS.try_with(|c| c.set(c.get() + 1));
    }

    #[cfg(test)]
    pub(crate) fn note_terminal_surface() {
        let _ = TERMINAL_SURFACES.try_with(|c| c.set(c.get() + 1));
    }
}

// ---------------------------------------------------------------------------
// Font size — real logical pixels
// ---------------------------------------------------------------------------

/// Inclusive font-size bounds shown and enforced by the Appearance stepper.
pub(crate) const FONT_SIZE_MIN_PX: i32 = 8;
pub(crate) const FONT_SIZE_MAX_PX: i32 = 32;

// ---------------------------------------------------------------------------
#[path = "model/find.rs"]
mod find;
pub(crate) use find::{FindState, visible_window};

// Sessions and panes
// ---------------------------------------------------------------------------

/// One terminal session — a sidebar row / logical tab. Owns one or more panes;
/// the active pane fills the content area.
#[derive(Clone)]
pub(crate) struct Session {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// Never read; kept deliberately. `session_row` draws a tone-carrying
    /// `enz_status_dot` instead of a glyph, so nothing renders this. Every
    /// construction site sets it deliberately ("terminal", "server") and it is
    /// the field an icon-per-session design would use.
    #[allow(
        dead_code,
        reason = "Session construction carries icon metadata while sidebar rows render status dots."
    )]
    pub icon: String,
    pub status: SessionStatus,
    /// Sidebar classification; mixed groups can contain panes of either kind.
    pub kind: SessionKind,
    /// Legacy session/group metadata; reconnecting uses each pane's endpoint snapshot.
    #[allow(
        dead_code,
        reason = "Legacy session metadata coexists with per-pane reconnect endpoints."
    )]
    pub ssh_host_id: Option<String>,
    /// Explicit connection failure; renders the session with Error tone.
    pub connection_error: Option<String>,
    pub pinned: bool,
    /// In-app tear-out flag (multi-window emulation).
    pub detached: bool,
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    /// Tmux-style split tree; each leaf references a `pane_id` in `panes`.
    pub layout: PaneLayout,
    /// Durable framework renderer state for `layout`.
    ///
    /// Structural app edits replace its tree; divider events update it first
    /// and copy the resulting ratios back into `layout` for persistence.
    pub pane_grid: enzui_widgets::EnzPaneGridState<String>,
    /// Commands typed inside the app (distinct from the shell's own history),
    /// bounded to the newest [`IN_APP_HISTORY_MAX`].
    pub in_app_history: CommandHistory,
    /// Sessions merged into this one, newest last.
    ///
    /// Empty for an ordinary session, which is every session until one is
    /// grouped, so this is what "is this a group?" means. Each entry is the
    /// whole record needed to give that session back
    /// ([`crate::pane_group::GroupMember`]): its id, its title, its kind
    /// and the panes it contributed. A plain `Split pane right` adds a pane
    /// and NOT a member, which is the distinction that keeps an ordinary
    /// split from offering a meaningless "ungroup".
    pub group_members: Vec<crate::pane_group::GroupMember>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionStatus {
    Idle,
    Running,
    Ok,
    Warning,
    Error,
}

// `SessionStatus`, `SessionKind` and `LineTone` have no string tokens: the
// view paths take the enum straight (`session_row::status_kind` ->
// `StatusKind`, `App::session_group` matches `SessionKind`, `grid_view` maps
// `LineTone` -> `TextGridCellTone`). `FontSize::as_token` exists because
// `Settings::font_size` is a persisted string.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionKind {
    Local,
    Ssh,
}

/// Convert OSC 7's `file://host/path` into the row's live location. Remote
/// paths stay host-qualified; local paths remain absolute so they can be fed
/// straight back to `spawn_shell_at` when a pane/tab is opened.
pub(crate) fn display_reported_cwd(raw: &str, kind: SessionKind, remote_hint: &str) -> String {
    let value = raw.trim();
    let (reported_host, path) = value
        .strip_prefix("file://")
        .and_then(|rest| rest.split_once('/'))
        .map(|(host, path)| (host, format!("/{path}")))
        .unwrap_or(("", value.to_owned()));
    match kind {
        SessionKind::Local if reported_host.is_empty() || reported_host == "localhost" => path,
        // A directory reported on another host is not a path this process can
        // resolve, so it is labelled rather than shown as a local location.
        SessionKind::Local => format!("{reported_host}:{path}"),
        SessionKind::Ssh => {
            let fallback = remote_hint.rsplit('@').next().unwrap_or(remote_hint);
            let host = if reported_host.is_empty() || reported_host == "localhost" {
                fallback
            } else {
                reported_host
            };
            format!("{host}:{path}")
        }
    }
}

#[path = "model/transcript.rs"]
mod transcript;
pub(crate) use transcript::{
    CellInk, DeferredLine, LineTone, Pane, TranscriptCell, TranscriptHyperlink, TranscriptLine,
    TranscriptRow, TranscriptRun, cell_attr,
};
pub(crate) use transcript::{expanded_deferred_rows, note_deferred_expansion};

#[derive(Clone, PartialEq)]
pub(crate) struct HistoryItem {
    // Native command identities survive history copies so a moved pane's completion
    // cannot attach to another pane's pending row.
    pub id: String,
    pub command: String,
    /// Written and never read. Real entries record `when: String::new()`
    /// because the app has no clock; only the seeded demo session sets a time
    /// ("10:02"). A timestamped history is a feature (it needs a time source
    /// and a format).
    #[allow(
        dead_code,
        reason = "History records retain optional display timestamps; live records currently leave them empty."
    )]
    pub when: String,
    pub exit_code: Option<i32>,
}

// ---------------------------------------------------------------------------
// Bounded per-session command history
// ---------------------------------------------------------------------------

/// How many completed commands one session retains.
///
/// The retention policy this names is **the newest `IN_APP_HISTORY_MAX`
/// commands of this session, oldest evicted first** — not "every command since
/// the tab opened", which is what the unbounded `Vec` behind this used to
/// promise and could not keep: a long-lived tab in a build loop appended for as
/// long as the process ran, and the overlay realised one row per entry.
///
/// 200 is sized to the affordance, not to memory: the overlay is a *browse the
/// recent past and copy one back* surface, and a user scrolling past 200 rows
/// of truncated commands is looking for something the shell's own history
/// (`Ctrl-R`) answers better. It is also the number the UI is allowed to state,
/// so if it moves the copy moves with it.
pub(crate) const IN_APP_HISTORY_MAX: usize = 200;

/// One session's bounded command history.
///
/// # Why a type and not a capped `push` helper
///
/// A bound enforced by a helper is a bound only on the paths that remember to
/// call it; `session.in_app_history.push(..)` next to it still compiles and
/// still grows forever. The items are private here, so **every** append and
/// import route goes through [`record`](Self::record), [`push`](Self::push) or
/// [`from_items`](Self::from_items), and each of those trims. There is no
/// syntax for an unbounded append.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CommandHistory {
    /// Oldest first. Length is `<= IN_APP_HISTORY_MAX` by construction.
    items: std::collections::VecDeque<HistoryItem>,
    /// Monotonic id source. Never reset by eviction, so a recycled slot never
    /// recycles a key.
    next_seq: u64,
}

impl CommandHistory {
    /// The retention bound this history enforces, as an inherent constant so a
    /// caller cannot compare against a different number than the one that runs.
    pub(crate) const CAPACITY: usize = IN_APP_HISTORY_MAX;

    pub(crate) fn new() -> Self {
        Self::default()
    }

    // Imported native receipts retain their identity; other rows receive local keys.
    pub(crate) fn from_items(items: impl IntoIterator<Item = HistoryItem>) -> Self {
        let mut history = Self::new();
        for item in items {
            history.push(item);
        }
        history
    }

    /// Record a submitted command with no exit code yet.
    ///
    /// Returns `false` — and records nothing — for a blank command. The PTY
    /// reader already drops an empty line, but a bare `Enter` at a prompt is
    /// the single most common keystroke in a terminal, so the *model* refuses
    /// it too rather than trusting one caller to keep filtering.
    #[cfg(test)]
    pub(crate) fn record(&mut self, command: String) -> bool {
        if command.trim().is_empty() {
            return false;
        }
        self.push(HistoryItem {
            id: String::new(),
            command,
            when: String::new(),
            exit_code: None,
        });
        true
    }

    pub(crate) fn apply_shell_event(&mut self, event: &crate::shell_history::ShellEvent) {
        use crate::shell_history::{ShellEvent, command_row_id};
        match event {
            ShellEvent::Started { key, command } => self.push(HistoryItem {
                id: command_row_id(*key),
                command: command.clone(),
                when: String::new(),
                exit_code: None,
            }),
            ShellEvent::Finished {
                key: Some(key),
                status,
            } => {
                let id = command_row_id(*key);
                if let Some(item) = self
                    .items
                    .iter_mut()
                    .find(|item| item.id == id && item.exit_code.is_none())
                {
                    item.exit_code = Some(*status);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn push(&mut self, mut item: HistoryItem) {
        if crate::shell_history::is_command_row_id(&item.id) {
            if self.items.iter().any(|existing| existing.id == item.id) {
                return;
            }
        } else {
            item.id = format!("h-{}", self.next_seq);
            self.next_seq += 1;
        }
        // `>=`, not `==`: a capacity that shrank under a live history must
        // still converge instead of parking one item above the new bound.
        while self.items.len() >= Self::CAPACITY {
            self.items.pop_front();
        }
        self.items.push_back(item);
    }

    #[cfg(test)]
    pub(crate) fn finish_newest_pending(&mut self, exit_code: i32) -> bool {
        match self
            .items
            .iter_mut()
            .rev()
            .find(|item| item.exit_code.is_none())
        {
            Some(item) => {
                item.exit_code = Some(exit_code);
                true
            }
            None => false,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item at `index` counting from the newest (`0` is the most recent),
    /// which is the order the overlay renders.
    pub(crate) fn newest_first(&self, index: usize) -> Option<&HistoryItem> {
        let len = self.items.len();
        // Saturating rather than wrapping: `index >= len` must be `None`, not
        // a clamp onto the oldest row.
        if index >= len {
            return None;
        }
        self.items.get(len - 1 - index)
    }

    /// A newest-first `KeyedProvider` view for the virtual list. Borrows;
    /// wrap in `Rc` at the call site because the rows are derived in `view`.
    pub(crate) fn newest_first_rows(&self) -> HistoryRows<'_> {
        HistoryRows(self)
    }
}

/// Newest-first row source over a [`CommandHistory`].
pub(crate) struct HistoryRows<'a>(&'a CommandHistory);

impl enzui_core::virtual_sequence::ItemProvider<HistoryItem> for HistoryRows<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn item(&self, index: u64) -> Option<HistoryItem> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.0.newest_first(i))
            .cloned()
    }
}

impl enzui_core::virtual_sequence::KeyedProvider<HistoryItem> for HistoryRows<'_> {
    fn key(&self, index: u64) -> Option<enzui_core::virtual_sequence::SequenceKey> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.0.newest_first(i))
            .map(|item| enzui_core::virtual_sequence::SequenceKey::new(item.id.clone()))
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

// The canvas comes from the active scheme, so there is no background-preset
// spacing guard; `grid_view::every_scheme_paints_its_own_canvas_background` is
// the positive control that it is painted.

/// Inclusive lower bound for the scrollback ring. Below this a terminal loses
/// so much history it stops being useful; `clamped_scrollback` enforces it and
/// the settings label advertises it.
pub(crate) const SCROLLBACK_MIN: i32 = 500;
/// Inclusive upper bound for the scrollback ring. Above this the per-session
/// ring buffer costs real memory across many tabs; the clamp caps it here.
pub(crate) const SCROLLBACK_MAX: i32 = 20_000;

/// The scrollback bound label, computed once. Held as a process-lifetime
/// `String` so a `view` can hand out a `&'static str`-ish borrow that outlives
/// the returned `Element` (the framework `enz_preferences_row` holds `&'a str`,
/// so a per-call `String` local would be dropped before the element is used).
pub(crate) static SCROLLBACK_BOUNDS_LABEL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(scrollback_bounds_label);

fn localized_number(n: i32) -> String {
    crate::i18n::localizer()
        .locale()
        .format_decimal(&enzui_core::localization::Decimal::from(n))
        .unwrap_or_else(|_| n.to_string())
}

pub(crate) fn scrollback_bounds_label() -> String {
    crate::i18n::format(
        crate::i18n::localizer(),
        "Scrollback lines ({} – {})",
        &[
            ("arg0", &localized_number(SCROLLBACK_MIN)),
            ("arg1", &localized_number(SCROLLBACK_MAX)),
        ],
    )
}

/// User-tuned terminal appearance / behaviour. Discriminators are resolved
/// defensively so one stale persisted field cannot poison the remaining state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// Terminal glyph size in logical pixels.
    pub font_size_px: i32,
    /// `"block"` / `"underline"` / `"bar"`.
    pub cursor_style: String,
    /// Scrollback ring size (clamped 500–20000).
    pub scrollback_lines: i32,
    /// Named colour scheme id.
    pub color_scheme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size_px: 12,
            cursor_style: "block".into(),
            scrollback_lines: 5000,
            color_scheme: crate::color_scheme::DEFAULT_ID.into(),
        }
    }
}

impl Settings {
    /// The bounded pixel value consumed by both grid geometry and PTY sizing.
    pub(crate) fn clamped_font_size_px(&self) -> i32 {
        self.font_size_px.clamp(FONT_SIZE_MIN_PX, FONT_SIZE_MAX_PX)
    }

    /// User-visible value for the stepper readout.
    pub(crate) fn font_size_value_label(&self) -> String {
        format!("{} px", self.clamped_font_size_px())
    }

    /// Resolve `cursor_style`, falling back to the block cursor.
    pub(crate) fn resolved_cursor_style(&self) -> &str {
        match self.cursor_style.as_str() {
            "underline" | "bar" => self.cursor_style.as_str(),
            _ => "block",
        }
    }

    /// Font size in logical px for grid geometry.
    pub(crate) fn resolved_font_size_px(&self) -> f32 {
        self.clamped_font_size_px() as f32
    }

    /// Clamp scrollback to a safe range. The bounds are the ONE source of
    /// truth for both the runtime clamp and the settings label "Scrollback
    /// lines (500 – 20 000)": `scrollback_bounds_label` formats them so the
    /// advertised range can never drift from the value the model actually
    /// enforces.
    pub(crate) fn clamped_scrollback(&self) -> i32 {
        self.scrollback_lines.clamp(SCROLLBACK_MIN, SCROLLBACK_MAX)
    }

    /// Resolve the colour-scheme id, falling back to `"default"`.
    pub(crate) fn resolved_color_scheme(&self) -> &str {
        let id = self.color_scheme.as_str();
        for s in crate::color_scheme::SCHEMES {
            if s.id == id {
                return s.id;
            }
        }
        crate::color_scheme::DEFAULT_ID
    }
}

// ---------------------------------------------------------------------------
// SSH host records
// ---------------------------------------------------------------------------

/// Authentication mode for an SSH connection.
///
/// Modelled as an enum carrying its own data so that invalid combinations are
/// **unrepresentable**: a `KeyFile` with no key path, or an `Agent` carrying a
/// plaintext password, cannot be expressed.  This is the same shape
/// `remote-app` uses for `ForwardKind::{Local(Dest), Remote(Dest), Dynamic}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SshAuthMode {
    /// Password authentication. The secret is deliberately not part of this
    /// persistable profile model; it is keyed by `SshHostRecord::id` in the
    /// framework credential-store seam.
    Password,
    /// Public-key file authentication.
    KeyFile {
        /// Absolute path to the private key, e.g. `~/.ssh/id_ed25519`.
        key_path: String,
    },
    /// SSH agent forwarding (`SSH_AUTH_SOCK`).
    Agent,
}

/// The three authentication-choice labels in display order, kept as a single
/// source of truth for both the shared form model and the index↔enum helpers.
///
/// Swapping two entries silently remaps every user's selection — the test
/// `auth_mode_radio_order_matches_labels` catches exactly that.
pub(crate) const AUTH_MODE_LABELS: [&str; 3] = ["Password", "Key file", "Agent"];

/// The enum values at the same indices as [`AUTH_MODE_LABELS`].
pub(crate) const AUTH_MODES: [SshAuthModeKind; 3] = [
    SshAuthModeKind::Password,
    SshAuthModeKind::KeyFile,
    SshAuthModeKind::Agent,
];

/// Discriminant-only variant used by the form choice (no associated data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SshAuthModeKind {
    Password,
    KeyFile,
    Agent,
}

impl SshAuthMode {
    /// The discriminant of this auth mode.
    pub(crate) fn kind(&self) -> SshAuthModeKind {
        match self {
            SshAuthMode::Password => SshAuthModeKind::Password,
            SshAuthMode::KeyFile { .. } => SshAuthModeKind::KeyFile,
            SshAuthMode::Agent => SshAuthModeKind::Agent,
        }
    }
}

/// A saved SSH connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SshHostRecord {
    pub id: String,
    pub name: String,
    /// `user@host` or just `host`.
    pub destination: String,
    /// SSH port (defaults to 22).
    pub port: u16,
    /// Login username (may be empty if included in `destination`).
    pub username: String,
    /// Default cwd.
    pub path: String,
    /// Authentication mode.
    pub auth: SshAuthMode,
    /// Free-form notes (displayed as a third line in the saved-host row).
    pub notes: String,
}

impl SshHostRecord {
    /// The key path, if this record uses key-file auth.
    pub(crate) fn key_path(&self) -> Option<&str> {
        match &self.auth {
            SshAuthMode::KeyFile { key_path } => Some(key_path.as_str()),
            _ => None,
        }
    }
}

/// Whether the current environment has a reachable SSH agent.
///
/// Checks `SSH_AUTH_SOCK`: if the env var is set AND the socket file exists,
/// the agent is considered "ready".  This is a **best-effort** check — the
/// agent process could have died — but it matches what `ssh` itself does on
/// connection attempt.
pub(crate) fn ssh_agent_available() -> bool {
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        std::path::Path::new(&sock).exists()
    } else {
        false
    }
}

/// State for the "Add / Edit SSH connection" form.
///
/// Separated from `SshHostRecord` so the form can be partially filled,
/// validated, and discarded without touching the saved host list.
#[derive(Clone)]
pub(crate) struct SshHostForm {
    /// `None` = adding a new host; `Some(id)` = editing an existing one.
    pub editing_id: Option<String>,
    pub name: String,
    pub destination: String,
    pub port_text: String,
    pub username: String,
    pub auth_kind: SshAuthModeKind,
    /// Password text (only meaningful when `auth_kind == Password`).
    pub password: String,
    /// Key file path (only meaningful when `auth_kind == KeyFile`).
    pub key_path: String,
    /// Passphrase protecting the key file (only meaningful when `auth_kind ==
    /// KeyFile`). Kept in the form only: like `password`, it is never part of
    /// `SshHostRecord` and never persisted; it is handed to the framework
    /// credential store as a `Secret` and dropped.
    pub passphrase: String,
    /// Default working directory on the remote host — the terminal opens here
    /// after connecting. Empty means "the remote's login default" (usually
    /// `$HOME`). Maps to `SshHostRecord::path`.
    pub default_path: String,
    pub notes: String,
}

impl std::fmt::Debug for SshHostForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshHostForm")
            .field("editing_id", &self.editing_id)
            .field("name", &self.name)
            .field("destination", &self.destination)
            .field("port_text", &self.port_text)
            .field("username", &self.username)
            .field("auth_kind", &self.auth_kind)
            .field("password", &enzui_core::form::REDACTED)
            .field("key_path", &self.key_path)
            .field("passphrase", &enzui_core::form::REDACTED)
            .field("default_path", &self.default_path)
            .field("notes", &self.notes)
            .finish()
    }
}

impl Default for SshHostForm {
    fn default() -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            destination: String::new(),
            port_text: DEFAULT_SSH_PORT.to_string(),
            username: String::new(),
            auth_kind: SshAuthModeKind::Password,
            password: String::new(),
            key_path: String::new(),
            passphrase: String::new(),
            default_path: String::new(),
            notes: String::new(),
        }
    }
}

impl SshHostForm {
    /// Build the shared form-dialog model for a new connection or an existing
    /// record. The active authentication kind controls which credential rows
    /// exist; [`FormPresenter::retarget`](enzui_widgets::form_dialog::FormPresenter::retarget)
    /// preserves every surviving draft when that choice changes and drops the
    /// now-inactive secret field.
    pub(crate) fn form_model(
        record: Option<&SshHostRecord>,
        auth_kind: SshAuthModeKind,
    ) -> FormModel {
        let initial = record.map(Self::from_record).unwrap_or_default();
        let auth = auth_kind.form_value();
        let mut fields = vec![
            FormField::text(SSH_FORM_NAME, crate::i18n::label("Session name"))
                .placeholder(crate::i18n::text("e.g. production-server"))
                .initial(FieldValue::Text(initial.name))
                .required(true),
            FormField::text(SSH_FORM_DESTINATION, crate::i18n::label("Host"))
                .placeholder(crate::i18n::text("hostname or IP"))
                .initial(FieldValue::Text(initial.destination))
                .required(true),
            FormField::text(SSH_FORM_PORT, crate::i18n::label("Port"))
                .placeholder(DEFAULT_SSH_PORT.to_string())
                .initial(FieldValue::Text(initial.port_text))
                .validator(validate_ssh_port),
            FormField::text(SSH_FORM_USERNAME, crate::i18n::label("Username"))
                .placeholder(crate::i18n::text("e.g. ubuntu"))
                .initial(FieldValue::Text(initial.username)),
            FormField::choice(
                SSH_FORM_AUTH_MODE,
                crate::i18n::label("Auth mode"),
                AUTH_MODES
                    .iter()
                    .zip(AUTH_MODE_LABELS)
                    .map(|(kind, label)| {
                        ChoiceOption::new(kind.form_value(), crate::i18n::label(label))
                    })
                    .collect(),
            )
            .expect("SSH authentication choices are non-empty")
            .initial(FieldValue::Choice(Some(auth.to_owned())))
            .helper(if auth_kind == SshAuthModeKind::Agent {
                if ssh_agent_available() {
                    crate::i18n::label("Agent is ready")
                } else {
                    crate::i18n::label(
                        "No SSH agent detected (SSH_AUTH_SOCK not set or socket missing)",
                    )
                }
            } else {
                crate::i18n::label("Choose how this connection authenticates.")
            }),
        ];

        match auth_kind {
            SshAuthModeKind::Password => fields.push(
                FormField::secure(SSH_FORM_PASSWORD, crate::i18n::label("Password"))
                    .placeholder(crate::i18n::text("Password"))
                    .helper(crate::i18n::label(
                        "Remembered in the system keyring. Leave empty to keep the saved one.",
                    )),
            ),
            SshAuthModeKind::KeyFile => {
                fields.push(
                    FormField::text(SSH_FORM_KEY_PATH, crate::i18n::label("SSH key path"))
                        .placeholder("~/.ssh/id_ed25519")
                        .initial(FieldValue::Text(initial.key_path)),
                );
                fields.push(
                    FormField::secure(SSH_FORM_PASSPHRASE, crate::i18n::label("Key passphrase"))
                        .placeholder(crate::i18n::text("Key passphrase"))
                        .helper(crate::i18n::label(
                            "Remembered in the system keyring. Leave empty to keep the saved one.",
                        )),
                );
            }
            SshAuthModeKind::Agent => {}
        }

        fields.extend([
            FormField::text(
                SSH_FORM_DEFAULT_PATH,
                crate::i18n::label("Default path (opens in this directory on the remote)"),
            )
            .placeholder(crate::i18n::text("e.g. /var/www"))
            .initial(FieldValue::Text(initial.default_path)),
            FormField::text(SSH_FORM_NOTES, crate::i18n::label("Notes"))
                .placeholder(crate::i18n::text("Optional notes"))
                .initial(FieldValue::Text(initial.notes)),
        ]);
        FormModel::new(fields).expect("SSH form field ids are unique")
    }

    /// Reconstruct the domain draft submitted by the shared form lifecycle.
    /// Secrets stay in this short-lived value only until the Save reducer
    /// moves them into the credential-store task; they never enter a host
    /// record or preferences bytes.
    pub(crate) fn from_submission(
        editing_id: Option<String>,
        submission: &FormSubmission,
    ) -> Option<Self> {
        let auth_kind =
            SshAuthModeKind::from_form_value(submission.value(&SSH_FORM_AUTH_MODE.into())?)?;
        let text = |id: &'static str| submission.text(&id.into()).unwrap_or_default().to_owned();
        Some(Self {
            editing_id,
            name: text(SSH_FORM_NAME),
            destination: text(SSH_FORM_DESTINATION),
            port_text: text(SSH_FORM_PORT),
            username: text(SSH_FORM_USERNAME),
            auth_kind,
            password: text(SSH_FORM_PASSWORD),
            key_path: text(SSH_FORM_KEY_PATH),
            passphrase: text(SSH_FORM_PASSPHRASE),
            default_path: text(SSH_FORM_DEFAULT_PATH),
            notes: text(SSH_FORM_NOTES),
        })
    }

    /// Populate the form from an existing record for editing.
    pub(crate) fn from_record(record: &SshHostRecord) -> Self {
        let (auth_kind, password, key_path) = match &record.auth {
            SshAuthMode::Password => (SshAuthModeKind::Password, String::new(), String::new()),
            SshAuthMode::KeyFile { key_path } => {
                (SshAuthModeKind::KeyFile, String::new(), key_path.clone())
            }
            SshAuthMode::Agent => (SshAuthModeKind::Agent, String::new(), String::new()),
        };
        Self {
            editing_id: Some(record.id.clone()),
            name: record.name.clone(),
            destination: record.destination.clone(),
            port_text: record.port.to_string(),
            username: record.username.clone(),
            auth_kind,
            password,
            key_path,
            // A remembered passphrase is never redisplayed: the form shows an
            // empty field, and leaving it empty keeps what is stored.
            passphrase: String::new(),
            default_path: record.path.clone(),
            notes: record.notes.clone(),
        }
    }

    /// Attempt to build a valid `SshHostRecord` from the form fields.
    ///
    /// Returns `None` when the form is not submittable: a required field is
    /// empty, or the port is present but not a valid `1..=65535` value. The
    /// port is never silently coerced to 22; an invalid port fails the build so
    /// the Save reducer surfaces an error instead of persisting a number the
    /// user did not type. The empty port still means "the default 22" (the
    /// field's placeholder promises exactly that), which is a chosen default,
    /// not a coercion of invalid input.
    ///
    /// The shared [`FormSession`](enzui_core::form::FormSession) is the
    /// caller-facing gate that names which field is wrong; this method remains
    /// the last-line invariant that the record it hands back is internally valid.
    pub(crate) fn to_record(&self, id: &str) -> Option<SshHostRecord> {
        let name = self.name.trim();
        let dest = self.destination.trim();
        if name.is_empty() || dest.is_empty() {
            return None;
        }
        let port_text = self.port_text.trim();
        let port = if port_text.is_empty() {
            DEFAULT_SSH_PORT
        } else {
            match port_text.parse::<u16>() {
                Ok(p) if p >= 1 => p,
                _ => return None,
            }
        };
        let auth = match self.auth_kind {
            SshAuthModeKind::Password => SshAuthMode::Password,
            SshAuthModeKind::KeyFile => SshAuthMode::KeyFile {
                key_path: self.key_path.trim().to_owned(),
            },
            SshAuthModeKind::Agent => SshAuthMode::Agent,
        };
        Some(SshHostRecord {
            id: id.to_owned(),
            name: name.to_owned(),
            destination: dest.to_owned(),
            port,
            username: self.username.trim().to_owned(),
            path: self.default_path.trim().to_owned(),
            auth,
            notes: self.notes.clone(),
        })
    }
}

/// Stable field ids for the shared SSH form session.
pub(crate) const SSH_FORM_NAME: &str = "ssh-name";
pub(crate) const SSH_FORM_DESTINATION: &str = "ssh-destination";
pub(crate) const SSH_FORM_PORT: &str = "ssh-port";
pub(crate) const SSH_FORM_USERNAME: &str = "ssh-username";
pub(crate) const SSH_FORM_AUTH_MODE: &str = "ssh-auth-mode";
pub(crate) const SSH_FORM_PASSWORD: &str = "ssh-password";
pub(crate) const SSH_FORM_KEY_PATH: &str = "ssh-key-path";
pub(crate) const SSH_FORM_PASSPHRASE: &str = "ssh-passphrase";
pub(crate) const SSH_FORM_DEFAULT_PATH: &str = "ssh-default-path";
pub(crate) const SSH_FORM_NOTES: &str = "ssh-notes";

fn validate_ssh_port(value: &FieldValue) -> Option<String> {
    let text = value.as_text()?.trim();
    if text.is_empty() || matches!(text.parse::<u16>(), Ok(1..=u16::MAX)) {
        None
    } else {
        Some("Enter a port between 1 and 65535.".to_owned())
    }
}

impl SshAuthModeKind {
    pub(crate) const fn form_value(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::KeyFile => "key-file",
            Self::Agent => "agent",
        }
    }

    pub(crate) fn from_form_value(value: &FieldValue) -> Option<Self> {
        match value {
            FieldValue::Choice(Some(value)) if value == "password" => Some(Self::Password),
            FieldValue::Choice(Some(value)) if value == "key-file" => Some(Self::KeyFile),
            FieldValue::Choice(Some(value)) if value == "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// The default SSH port an empty port field resolves to. Named so the form's
/// placeholder, its `to_record` default and its validation cannot drift onto
/// three different literals.
pub(crate) const DEFAULT_SSH_PORT: u16 = 22;

#[cfg(test)]
mod tests;

// Rendered terminal data may contain credentials even when its command was accepted.
macro_rules! redacted_terminal_debug {
    ($($kind:ty),+ $(,)?) => {$(impl std::fmt::Debug for $kind {
        fn fmt(&self, f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($kind),"([redacted])"))}
    })+};
}
redacted_terminal_debug!(
    Session,
    Pane,
    TranscriptLine,
    TranscriptCell,
    TranscriptRun,
    TranscriptHyperlink,
    HistoryItem
);
