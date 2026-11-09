//! Terminal application struct.

use super::*;

type FittedGrid = ((u16, u16), (u16, u16));

pub(crate) struct App {
    pub(crate) themes: ThemeIntegration,
    pub(crate) shell_activity: crate::shell_activity::ShellActivity,
    pub(crate) screen: Screen,
    pub(crate) sessions: Vec<Session>,
    pub(crate) active: usize,
    pub(crate) settings: Settings,
    /// One atomic typed store for Appearance plus shortcut overrides.
    pub(crate) settings_store: Store<settings_store::AppPreferences>,
    /// Restartable tabs/panes live in XDG state, not the preferences file.
    pub(crate) session_store: Arc<Store<session_store::SessionState>>,
    pub(crate) secure_pins: pinned_storage::SecurePins,
    /// Latest session snapshot queued for the serialized async writer.
    pub(crate) session_snapshot: session_store::SessionState,
    pub(crate) restored_outputs: HashMap<String, Arc<pinned_output::PinnedOutput>>,
    pub(crate) finished_outputs: HashMap<String, Arc<pinned_output::PinnedOutput>>,
    pub(crate) durable_titles: HashMap<String, String>,
    pub(crate) restored_grid_sizes: HashMap<String, iced::Size>,
    /// The GPU terminal screen of every painted pane, per window.
    pub(crate) screens: crate::terminal_view::Screens,
    /// The grid each pane's view reported it fits, `(cols, rows)`, with the
    /// app's own estimate at that time: the report wins while the estimate is
    /// unchanged (HiDPI rounding), and a new layout falls back to the
    /// estimate until the view reports again.
    pub(crate) terminal_fit: HashMap<String, FittedGrid>,
    /// See [`App::terminal_family`].
    pub(crate) terminal_font_family: Option<String>,
    /// Only one state write runs at once, so an older snapshot cannot finish
    /// after a newer one and overwrite it.
    pub(crate) session_save_in_flight: bool,
    pub(crate) empty_save_requested: bool,
    /// A failed state load is retained separately from write failures so the
    /// banner can explain that safe defaults are active and the old bytes are
    /// protected from automatic replacement.
    pub(crate) session_load_error: Option<String>,
    pub(crate) startup: Option<persistence_runtime::Startup>,
    pub(crate) launch_directory: Option<String>,
    pub(crate) startup_settings_refused: bool,
    pub(crate) startup_sessions_refused: bool,
    /// The single bounded owner every synchronous call in this app is
    /// submitted to.
    pub(crate) blocking: Arc<blocking_dispatch::Dispatch>,
    /// Set while the preferences document is owed because its last write never
    /// reached the store. Cleared by a publication or by an explicit retry.
    pub(crate) preferences_write_failed: bool,
    pub(crate) preferences_save_generation: operation_origin::RequestGeneration,
    /// A failed state save is retained and rendered as a persistent assertive
    /// banner, so the user knows this workspace will not survive a restart.
    pub(crate) session_save_error: Option<String>,
    /// Bounded transient acknowledgement lifecycle. The only producer
    /// is command-history copy; same-key pushes replace in place and reset the
    /// deterministic dwell deadline instead of stacking permanent labels.
    pub(crate) toasts: ToastHost,
    /// Shared earliest-deadline one-shot expiry schedule. Reconciled from the
    /// host after every turn that touches it via [`ToastHost::sync_expiry`];
    /// keeping the wait fixed across unrelated updates prevents a declarative
    /// subscription rebuild from restarting the dwell clock. Replaces the
    /// app-local `(deadline, wait)` adapter this app used to hand-roll.
    pub(crate) toast_expiry: ToastExpirySchedule,
    /// The one resolved shortcut table borrowed by capture, labels, and dispatch.
    pub(crate) shortcuts: EditableShortcutCatalog<String>,
    /// Selected Settings page and narrow stacked-navigation depth.
    pub(crate) settings_page: String,
    pub(crate) settings_drilled_in: bool,
    /// Controlled shortcut recorder and its last validation/conflict feedback.
    pub(crate) shortcut_armed: Option<String>,
    pub(crate) shortcut_feedback: Option<(String, String)>,
    pub(crate) nav_split: NavSplitState,
    /// Live PTY per pane, keyed by pane id. Not every pane must be PTY-backed
    /// (a failed spawn leaves a scripted fallback).
    pub(crate) ptys: HashMap<String, pty::PtySession>,
    /// Retired sessions whose processes, workers or descriptors are still owned.
    /// A non-empty list holds every window close. Structurally bounded by the
    /// shared reservation capacity, because every ticket holds a reservation.
    pub(crate) teardown_debts: Vec<TeardownDebt>,
    /// The bounded executor that owns every exact teardown. Held as a field
    /// rather than reached for, so a route control can assert exit authority
    /// against its own executor instead of the process-wide one.
    pub(crate) teardown: &'static pty::TeardownSupervisor,
    /// A workspace adoption refused because the previous workspace's children
    /// were still being torn down. Surfaced, never silent.
    pub(crate) restore_held_by_teardown: bool,
    /// The process has been authorized to end but the executor still owns exact
    /// teardown custody, so the exit is withheld.
    pub(crate) exit_withheld: bool,
    /// Modifier state, tracked from key events so the grid can build chords.
    pub(crate) mods: Modifiers,
    /// Sub-row wheel residue. A trackpad reports pixel deltas far
    /// smaller than a cell, so truncating each event to whole rows would drop
    /// every fine gesture on the floor. The fraction is carried here and spent
    /// as soon as it completes a row.
    pub(crate) wheel_row_residue: f32,
    /// The main window's id, learned when the boot task's `window::open`
    /// resolves.
    ///
    /// `None` only before that resolves. Stored rather than asked for with
    /// `iced::window::latest()`, which IS "my own id" while there is one window
    /// and silently becomes "whichever window opened most recently" the day a
    /// tab is detached — trap 3 of `app_shell`'s second-window section, and the
    /// reason a titlebar drag would otherwise start moving the detached window.
    pub(crate) main_window: Option<iced::window::Id>,
    pub(crate) window_chrome: HashMap<iced::window::Id, enzui_widgets::app_shell::ShellWindowState>,
    pub(crate) focused_window: Option<iced::window::Id>,
    /// Which windows are alive, and therefore when the process exits. The
    /// shared `WindowLedger`; a daemon with no windows keeps running, so this
    /// is what stands between a user's last close and a process that survives
    /// invisibly.
    pub(crate) windows: enzui_core::window_lifecycle::WindowLedger,
    /// Detached sessions, keyed by the window each one owns.
    ///
    /// Keyed by window id because `view`/`title` are asked *by id*: a detached
    /// terminal has to be answerable from the id alone. The session id is the
    /// value, so the mapping survives a session being reordered in the sidebar
    /// (an index would not), and the per-window size lives here too because
    /// `resize_events()` is app-wide and each window's geometry drives its own
    /// PTY.
    pub(crate) detached_windows: HashMap<iced::window::Id, DetachedWindow>,
    /// Windows whose session has been closed, parked for `update`'s single
    /// drain. A `Vec` because two tabs can be closed in one
    /// dispatch chain.
    pub(crate) orphaned_windows: Vec<iced::window::Id>,
    /// Windows whose destruction this app asked for, and why. Recorded before
    /// the close task is issued, so the `Closed` event that follows is read as
    /// this app's own rather than as an external teardown.
    pub(crate) intentional_closes: HashMap<iced::window::Id, CloseIntent>,
    /// Every window the orphan drain has closed, for the disposal gate only
    ///: the drain runs inside one `update`, so nothing else
    /// survives it to observe.
    #[cfg(test)]
    pub(crate) disposed_windows: Vec<iced::window::Id>,
    #[cfg(feature = "native-invalidation-acceptance")]
    pub(crate) native_acceptance: Option<native_invalidation::NativeAcceptance>,
    /// The last 1-based screen cell a pointer gesture reported to a tracking
    /// application. A wheel notch carries a coordinate, but the wheel
    /// message does not itself know where the pointer is, so the last cell a
    /// press/drag reported is used. Defaults to `(1, 1)`, the top-left cell.
    pub(crate) mouse_last_cell: (usize, usize),
    /// Test-only tap recording every byte slice handed to
    /// [`App::write_active_pty`], so a mouse-report gate can assert the exact
    /// bytes on the wire without a live child to echo them back.
    #[cfg(test)]
    pub(crate) pty_write_log: Vec<Vec<u8>>,
    /// Targeted companion to `pty_write_log`: proves a detached input reached
    /// the pane named by its window instead of the docked active pane.
    #[cfg(test)]
    pub(crate) targeted_pty_write_log: Vec<(String, Vec<u8>)>,
    /// The link authority a fixture publishes for panes it builds without a
    /// live PTY. Production always reads the running screen's generation.
    #[cfg(test)]
    pub(crate) link_authority_override: Option<u64>,
    /// SSH connections whose PTY spawn is deferred until the credential read
    /// resolves. A password or passphrase host retrieves its secret before the
    /// child is spawned so the secret travels the sealed `memfd` sshpass
    /// channel at spawn time rather than a plaintext PTY write into a live
    /// session. Each entry is matched to its completion by a unique request
    /// token together with the stable profile id and slot, so neither a row
    /// mutation nor a second concurrent read for the same profile can route
    /// one secret to another connection.
    pub(crate) pending_ssh_connections: Vec<PendingSshConnection>,
    pub(crate) pending_pane_connections: Vec<endpoint_connections::PendingPaneConnection>,
    /// Panes whose PTY is being spawned on a worker thread, by pane id, with the
    /// ticket their completion must present and the owning session id. A
    /// completion whose ticket is no longer here (the pane closed, or a
    /// workspace restore replaced it) retires its PTY instead of installing it.
    pub(crate) pending_pane_spawns: HashMap<String, (u64, String)>,
    pub(crate) next_pane_spawn_ticket: u64,
    /// At most one owed host-trust confirmation per pane, each bound to its
    /// profile id, endpoint snapshot and connection generation.
    pub(crate) host_trust: Vec<host_trust::HostTrustOffer>,
    pub(crate) pending_endpoint_restores: Vec<String>,
    pub(crate) pending_detached_restores: Vec<String>,
    pub(crate) endpoint_credentials_ready: bool,
    #[cfg(test)]
    pub(crate) endpoint_spawn_override: Option<endpoint_connections::EndpointSpawnOverride>,
    /// Monotonic token stamped on each deferred SSH credential read so its
    /// completion routes only to the pending connection that issued it. Starts
    /// at 1; 0 is reserved for reads that never park a pending connection, so
    /// such a completion can never match one.
    pub(crate) next_credential_token: u64,
    /// Test-only seam for the SSH child spawn, so a gate can drive the real
    /// `update`/credential flow and observe the exact `Option<&str>` password
    /// the reducer hands to [`pty::PtySession::spawn_ssh_with_scrollback`] without launching a
    /// real `ssh`.
    #[cfg(test)]
    pub(crate) ssh_spawn_override: Option<SshSpawnOverride>,
    /// The last pointer coordinates the terminal grid reported.
    ///
    /// Coordinates that feed the selection model: `text_grid` reports the cell
    /// under the pointer, and the selection model, its paint and the clipboard
    /// copy build on it.
    pub(crate) grid_pointer: GridPointerProbe,
    /// Multi-click detector for word (double) and line (triple) selection.
    ///
    /// Uses `SingleClickPolicy::Immediate`: the first click clears the
    /// selection / places the anchor immediately, and a double/triple fires the
    /// word/line selection on top of it.
    pub(crate) click_detector: ClickDetector,
    /// Each pane's text selection, keyed by stable pane id.
    ///
    /// Built from the grid pointer's anchor and focus once a drag produces a
    /// non-empty range. Cleared on reflow, since a re-wrap voids column
    /// positions. Keyed per pane so two panes hold independent selections and
    /// no leaf receives a sibling's coordinates; [`App::selection`] is the
    /// active pane's entry.
    pub(crate) selections: HashMap<String, selection::SelectionRange>,
    /// Last content-pane pixel size, used to compute the PTY cols/rows.
    pub(crate) content_size: iced::Size,
    /// Whether the settings dialog is open.
    pub(crate) settings_open: bool,
    /// Retention for the adaptive settings surface. Kept in sync with
    /// `settings_open` so `Dismissed` (Escape / scrim / X) starts the exit
    /// animation on the outgoing dialog/sheet and `SettingsExitSettled` retires
    /// it only on the matching generation. A rapid reopen mints a new key that
    /// the previous close's late settlement cannot destroy.
    pub(crate) settings_presentation: ModalPresentation<()>,
    /// Focus owner retained beside the live Settings presentation.
    pub(crate) settings_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,
    // ---- SSH connection manager state ---------------------------------
    /// Whether the SSH manager dialog is open.
    pub(crate) ssh_manager_open: bool,
    /// Manager-level focus scope retained while its form or host list is topmost.
    pub(crate) ssh_manager_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,
    /// The shared lifecycle for the currently presented SSH add/edit form.
    /// `None` means the manager is showing its host list. Drafts, validation
    /// errors, first-invalid focus, multiline editor state, and secure-field
    /// reveal all live in this one presentation.
    pub(crate) ssh_form: Option<SshFormPresentation>,
    /// App-authored SSH profiles loaded from the typed XDG preferences store.
    /// Their password bytes are absent; those live behind `credentials`.
    pub(crate) ssh_hosts: Vec<SshHostRecord>,
    /// Read-only named aliases discovered from OpenSSH user config.
    pub(crate) openssh_hosts: Vec<ssh_config::OpenSshHost>,
    pub(crate) openssh_load_error: Option<String>,
    pub(crate) openssh_discovery: ssh_discovery::Discovery,
    pub(crate) openssh_intent: Option<ssh_discovery_app::Intent>,
    /// Framework credential-store seam. Boot installs the real
    /// freedesktop Secret Service transport through
    /// `Message::CredentialBackendReady`; until that async connect resolves —
    /// and forever, on a box with no session bus — this is the framework's
    /// `UnavailableCredentialStore`, which reports `Unavailable` truthfully
    /// instead of downgrading to plaintext on disk.
    pub(crate) credentials: Arc<dyn CredentialStore>,
    // Production retains both halves of the bounded mutation worker; injected
    // capture and test stores deliberately leave these absent.
    pub(crate) credential_mutation_owner: Option<Arc<CredentialMutationOwner>>,
    /// The process-wide enzui-center session, when opted in and enrolled.
    #[cfg(feature = "center")]
    pub(crate) center: Option<enzui_agent::center::CenterHost<crate::center::CenterCall>>,
    /// The boot asked for the center; it connects once startup has finished,
    /// so the app registers only when it can serve its functions.
    #[cfg(feature = "center")]
    pub(crate) center_requested: bool,
    pub(crate) credential_mutation_store: Option<CredentialMutationStore>,
    pub(crate) credential_mutation_admission_open: bool,
    // Latest exact set/delete intent per profile slot until provider state is
    // definitive.
    pub(crate) credential_mutations: Vec<PendingCredentialMutation>,
    pub(crate) credential_ops: Vec<InFlightCredentialOp>,
    pub(crate) paste_requests: Vec<operation_origin::PendingPaste>,
    /// A profile whose record is held back until its secrets are actually
    /// gone. See `Message::SshCredentialDeleted`.
    pub(crate) pending_secret_deletion: Option<PendingSecretDeletion>,
    /// Set by a capture fixture so the boot connect, which resolves later,
    /// cannot replace the scripted backend.
    pub(crate) credential_backend_pinned: bool,
    /// Test-only parking lot for the futures `spawn_credential` would hand to
    /// the iced runtime. Never present in a product build.
    #[cfg(test)]
    pub(crate) credential_futures: Vec<CredentialFuture>,
    /// Descriptor-owning local file/directory portal seam. URI lifecycle is
    /// synchronous reducer state in `uri_portal`, so task drop is never used as
    /// cancellation.
    pub(crate) link_opener: Arc<dyn link_open::LinkOpener>,
    /// Shared cancellable OpenURI transport, keyed by originating session and
    /// pane plus a monotone generation.
    pub(crate) uri_portal: link_open::UriPortal,
    pub(crate) activation_broker: ActivationBroker,
    /// The one typed credential line the manager shows. Not an error string:
    /// a dismissed unlock prompt and a policy refusal carry `NoticeTone::Info`
    /// and their own words.
    pub(crate) ssh_credential_notice: Option<CredentialNotice>,
    /// Safe feedback for a refused or malformed settings document. It applies
    /// to Appearance, shortcuts, and SSH host metadata in the shared file.
    pub(crate) settings_load_error: Option<String>,
    pub(crate) ssh_save_error: Option<String>,
    pub(crate) ssh_host_seq: u64,
    pub(crate) pending_ssh_host_save: Option<ssh_host_save::Pending>,
    pub(crate) ssh_host_save_uncertain: bool,
    /// Whether the terminal grid currently has keyboard focus (typing target).
    pub(crate) next_pane_seq: u64,
    /// Set after the first `ContentResized` event arrives.  Before that, the
    /// window geometry is unknown (winit reports its default, not the
    /// compositor's actual tiled size).  Gating the PTY-poll timer on this
    /// flag prevents the 33ms timer from triggering a render with the stale
    /// initial size before the compositor's configure event is processed.
    pub(crate) has_initial_resize: bool,
    /// The `(rows, cols)` the most recent [`App::resize_active_pty`]
    /// call pushed, or `None` if it has never run.
    ///
    /// Test-only, and it records the CALL, not the computation: a test that
    /// only compared `pty_geometry()` across a sidebar toggle would stay green
    /// with every `resize_active_pty()` call site deleted, because the number
    /// moves whether or not anything sends it. The shell only learns its new
    /// width via `SIGWINCH`, so "was the resize performed?" is the fact under
    /// test.
    #[cfg(test)]
    pub(crate) last_pty_resize: Option<(u16, u16)>,
    // ---- Find bar state -----------------------------------------------
    /// Per-pane find bar state. Keyed by the active session index for now
    /// (find is session-scoped).
    pub(crate) find: FindState,
    pub(crate) find_owner: Option<String>,
    pub(crate) input_context: Option<operation_origin::OperationOrigin>,
    pub(crate) input_frame_start: Option<usize>,
    pub(crate) transcript_epochs: HashMap<String, operation_origin::RequestGeneration>,
    pub(crate) input_origins: HashMap<String, operation_origin::OperationOrigin>,
    pub(crate) window_inputs: HashMap<iced::window::Id, window_input::WindowInput>,
    /// The window whose ordered input stream held keyboard focus when the
    /// message now being reduced was mapped; set only for current stamps.
    pub(crate) ordered_focus: Option<iced::window::Id>,
    /// The main window between `window::open` and its opened message; it can
    /// already have keyboard focus and receive keys.
    pub(crate) opening_window: Option<iced::window::Id>,
    /// Keys typed before the first pane accepted input, replayed in order.
    pub(crate) typeahead: Vec<window_input::Typeahead>,
    pub(crate) pending_links: HashMap<link_open::UriOwner, window_input::PendingLink>,
    // ---- Titlebar overflow state --------------------------------
    /// Whether the narrow-rung titlebar overflow (kebab) menu is open.
    ///
    /// Transient view state, not a durable preference: it is cleared whenever
    /// the window widens back out of the compact rung, so a menu opened on a
    /// phone does not linger invisibly at desktop width.
    pub(crate) titlebar_overflow_open: bool,
    /// Framework command menu in the sidebar header.
    pub(crate) sidebar_titlebar_overflow_open: bool,
    // ---- History overlay state ----------------------------------------
    /// Whether the command-history overlay is open.
    pub(crate) history_open: bool,
    /// Live scroll geometry of the history overlay's virtual list.
    /// A cell rather than a `Message` because the only consumer is the next
    /// `view`: the realisation window needs the offset and the measured
    /// viewport, and routing them through the reducer would repaint the whole
    /// shell on every wheel tick to compute a number `view` already has.
    pub(crate) history_scroll: enzui_widgets::scroll_view::ScrollGeometryCell,
    /// The main window's pane-grid area, as the view last laid it out. `view`
    /// records the size the pane grid is actually allocated (below the floating
    /// bar and any rename or error banner), and PTY geometry reads it so the
    /// shell is told the rows the leaf really shows rather than the full window
    /// height. `None` until the first layout.
    pub(crate) grid_area: std::rc::Rc<std::cell::Cell<Option<iced::Size>>>,
    /// The pane-grid area the PTYs were last resized for, so a change published
    /// by `view` triggers exactly one reflow on the next poll.
    pub(crate) resized_grid_area: Option<iced::Size>,
    /// Persistent terminal-surface failure (pane ceiling or OpenURI failure),
    /// shown as a notice (`window_notices`).
    pub(crate) pane_feedback: Option<String>,
    /// Notices the user dismissed, by id, while their text is unchanged.
    pub(crate) dismissed_notices: std::collections::BTreeSet<String>,
    /// The git branch each local pane's directory is on, for the sidebar.
    pub(crate) git_branches: HashMap<String, String>,
    /// Panes whose branch is to be looked up, with their directory; drained
    /// into dispatcher work after each update.
    pub(crate) git_lookups_due: Vec<(String, std::path::PathBuf)>,
    /// Most-recently-used session ids, newest first. IDs survive reordering;
    /// stale entries are pruned when a tab closes.
    pub(crate) session_mru: Vec<String>,
    /// Retained subtree boundary for the sidebar session list.
    /// Warm when the PTY content changes but the session list does not.
    pub(crate) sidebar_cache: SubtreeHandle,
    /// The one drag/drop channel shared by the sidebar's
    /// session rows and the terminal content area's edge drop zones.
    ///
    /// Durable rather than rebuilt in `view`: the sidebar rail is inside a
    /// retained `enz_cached_subtree`, so a per-frame channel would leave those
    /// cached rows registering on a channel no host is listening to — the
    /// rows would simply never drag, silently. The channel holds no durable
    /// state of its own (one `Rc` to a per-pass registry), so keeping it here
    /// costs nothing and removes the whole class of bug.
    pub(crate) drag_channel: enzui_widgets::drag_drop_host::DragHostChannel,
    /// Pending SSH host deletion index (awaiting confirmation).
    pub(crate) pending_delete_ssh_host: Option<usize>,
    /// Lifecycle state for the framework `enz_confirm_dialog` behind
    /// destructive SSH-host deletion. There is no checkbox, so it carries
    /// nothing but the preset's own contract — the preset takes it by
    /// reference and makes every exit (button, Escape, scrim) fan into one
    /// `ConfirmDialogEvent::Resolved(cause)`.
    pub(crate) confirm_state: enzui_core::confirm::ConfirmState,

    // ---- Pane rename state ---------------------------------------------
    /// In-progress pane rename: the draft title being typed. `None` when no
    /// rename is active. The rename targets the active session's active pane;
    /// committing writes the trimmed draft to `Pane::title` (empty draft
    /// cancels, keeping the existing title).
    pub(crate) pane_rename_draft: Option<String>,

    // ---- Session-row context menu ---------------------------------------
    /// The open session context menu, or `None`. Holds the target session id
    /// and the raw click anchor — see [`context_menu::OpenContextMenu`].
    ///
    /// One `Option`, not a `bool` plus a separate anchor: an open menu with no
    /// target is not a state the view can render, and two fields that can
    /// disagree are how a menu ends up drawn at the origin.
    pub(crate) context_menu: Option<context_menu::OpenContextMenu>,

    /// In-progress **session** rename, as `(session id, draft title)`.
    ///
    /// Raised by the context menu's `Rename` row. Keyed by id rather than
    /// index for the same reason the menu is: a reorder or a close must not
    /// silently retarget the rename onto a different tab. Mirrors the existing
    /// `pane_rename_draft` shape one level up.
    pub(crate) session_rename: Option<(String, String)>,
    /// Session ids explicitly renamed by the user. An explicit rename wins
    /// permanently over later OSC 0/2 requests from that shell.
    pub(crate) title_overrides: HashSet<String>,
}
