//! Terminal message enum.

use super::*;

#[derive(Debug, Clone)]
pub(crate) enum Message {
    PaneInput(operation_origin::OperationOrigin, Box<Message>),
    PanePointer(
        operation_origin::OperationOrigin,
        operation_origin::RequestGeneration,
        usize,
        Box<Message>,
    ),
    InputIme(
        operation_origin::RequestGeneration,
        iced::advanced::input_method::Event,
        bool,
    ),
    InputKey(
        operation_origin::RequestGeneration,
        keyboard::Key,
        Modifiers,
        bool,
    ),
    InputFocus(bool),
    /// A keyboard focus edge mapped by the window's input stream, with its clock value.
    InputFocusEdge(iced::window::Id, u64),
    /// Input mapped while the window's input stream held keyboard focus, with
    /// the focus clock value at mapping time.
    StampedInput(iced::window::Id, u64, Box<Message>),
    InputCursor(iced::window::Id, iced::Point),
    WindowChromeKey(
        iced::window::Id,
        operation_origin::RequestGeneration,
        keyboard::Key,
        keyboard::Modifiers,
        bool,
    ),
    InputMenuOpen,
    InputMenuEvent(
        operation_origin::OperationOrigin,
        enzui_widgets::context_menu_host::ContextMenuEvent,
    ),
    InputGroupInto(String),
    OwnedUriActivation(
        operation_origin::OperationOrigin,
        String,
        enzui_parent_lease::Handoff<ActivationResponse>,
    ),
    OwnedLinkFinished(
        operation_origin::OperationOrigin,
        Option<Result<(), link_open::LinkOpenError>>,
    ),
    Theme(Box<ThemeEvent>),
    /// A raw keyboard key press (routed to the active PTY when typing).
    KeyPressed {
        key: keyboard::Key,
        mods: Modifiers,
        /// `true` when a focused widget (e.g. the find-bar search field)
        /// already consumed this event. Such keys must NOT also be written to
        /// the PTY, or typing into the find box echoes into the shell.
        captured: bool,
    },
    /// A key stamped with the runtime window that received it. The reducer
    /// unwraps main-window keys into `KeyPressed`, but detached keys stay
    /// targeted so they cannot page or type into whichever docked tab happens
    /// to be active.
    #[cfg(test)]
    WindowKeyPressed {
        window: iced::window::Id,
        key: keyboard::Key,
        mods: Modifiers,
        captured: bool,
    },
    /// The detached form is stamped with the active pane before dispatching
    /// again. Its reducer revalidates both keys, so a pane switch/close between
    /// publication and application cannot retarget terminal input.
    DetachedKeyPressed {
        window: iced::window::Id,
        pane: String,
        key: keyboard::Key,
        mods: Modifiers,
        captured: bool,
    },
    ModifiersChanged(Modifiers),
    /// The content pane resized (window resize / layout change).
    ContentResized(iced::Size),
    /// The boot task's `window::open` resolved: this is the main window.
    MainWindowOpened(iced::window::Id),
    ActivityWindowTagged(iced::window::Id, String, bool),
    ActivityPrune(u64),
    /// A detached session's own window opened and carries this id. The session
    /// id travels with it because a `window::Id` is minted by the runtime, not
    /// chosen by the app.
    DetachedWindowOpened {
        session: String,
        window: iced::window::Id,
    },
    /// Some window was destroyed — possibly the last one.
    WindowClosed(iced::window::Id),
    WindowFocusChanged(iced::window::Id, bool),
    /// A window resized. Carries the id because `resize_events()` reports every
    /// window's resizes (trap 4); the reducer decides whose geometry moved.
    WindowResized(iced::window::Id, iced::Size),
    /// A titlebar gesture from a KNOWN window. The detached
    /// window's own titlebar publishes this, so its drag/close can only ever
    /// act on itself.
    WindowFor(iced::window::Id, WindowCommand),
    /// Set the command menu for exactly one detached window.
    DetachedTitlebarOverflow(iced::window::Id, bool),
    /// Set the sidebar header command menu visibility.
    SidebarTitlebarOverflow(bool),
    /// Re-dock the named session: close its window and return the tab to the
    /// main shell. One message for the button and the context row, so both go
    /// through the same single re-dock path.
    RedockSession(String),
    /// Poll tick: drain PTYs and reproject.
    Poll,
    #[cfg(feature = "native-invalidation-acceptance")]
    NativeAcceptancePoll,
    /// Select a session by index.
    SelectSession(usize),
    /// The n-th tab (from 0) the main window shows, skipping detached tabs.
    SelectDockedSession(usize),
    /// A wheel gesture over the terminal canvas, in signed rows (positive =
    /// toward the newest output). Routed by
    /// [`scrollback::WheelRoute::resolve`]: the transcript on the primary
    /// screen, the PTY on the alternate one.
    TranscriptWheel(f32),
    /// Scroll the active pane's transcript by a signed row delta — the
    /// PageUp/PageDown route into the SAME offset the wheel moves.
    ScrollTranscriptRows(i64),
    /// Set the active pane's first visible transcript row absolutely: what a
    /// scrollbar thumb drag publishes.
    ScrollTranscriptToRow(usize),
    /// Re-pin the active pane to the newest output.
    JumpToLatest,
    /// Detached-window counterparts carry BOTH stable coordinates. Callbacks
    /// from a closing/replaced window, or from a pane that stopped being that
    /// session's active pane, are rejected rather than retargeted.
    DetachedTranscriptWheel {
        window: iced::window::Id,
        pane: String,
        rows: f32,
    },
    DetachedScrollTranscriptRows {
        window: iced::window::Id,
        pane: String,
        delta: i64,
    },
    DetachedScrollTranscriptToRow {
        window: iced::window::Id,
        pane: String,
        row: usize,
    },
    DetachedJumpToLatest {
        window: iced::window::Id,
        pane: String,
    },
    /// Toggle the command-history overlay of one detached window.
    DetachedToggleHistory(iced::window::Id),
    /// Close the command-history overlay of one detached window.
    DetachedCloseHistory(iced::window::Id),
    /// Paste a history command into the PTY of a detached window's session,
    /// never the main window's active PTY.
    DetachedPasteHistoryCommand {
        window: iced::window::Id,
        command: String,
    },
    // ---- Pointer reports from the cell grid ----------------------------
    /// A press landed on a rendered grid cell (the anchor of a future
    /// selection). Carries the row's own window row plus the column, resolved
    /// by the framework's `text_grid`.
    GridCellPressed(grid_view::GridPointerTarget),
    /// The pointer moved while a grid press is held.
    GridCellDragged(grid_view::GridPointerTarget),
    /// The grid gesture ended.
    GridCellReleased(grid_view::GridPointerTarget),
    /// A pane's terminal view fits `cols` by `rows` cells.
    TerminalFit {
        pane: String,
        cols: u16,
        rows: u16,
    },
    /// A terminal view event the app does not act on.
    TerminalViewIgnored,
    /// Completion of a descriptor-owned local path request.
    #[cfg(test)]
    LinkOpenFinished(Result<(), link_open::LinkOpenError>),
    /// Event edge for one exact framework OpenURI generation.
    UriPortalWake(link_open::UriWake),
    /// Select the next session, wrapping past the last back to the first
    /// (`tab.next` / `tab.next-alt`).
    SelectNextSession,
    /// Select the previous session, wrapping past the first to the last
    /// (`tab.prev` / `tab.prev-alt`).
    SelectPrevSession,
    /// New local session.
    NewLocal,
    /// A PTY spawned off the reducer thread finished: its ticket, its pane
    /// id, and the PTY or the reason there is none.
    PaneSpawned(
        u64,
        String,
        enzui_parent_lease::Handoff<session_spawn::PaneSpawnOutcome>,
    ),
    ReconnectRestoredPane,
    RestoredGridResized(iced::Size),
    ResetSavedSessions,
    /// Close the active session.
    CloseActive,
    /// Split the active pane horizontally (new pane to the RIGHT).
    /// `Split pane right (Ctrl+Shift+E)`.
    NewPane,
    /// Split the active pane vertically (new pane BELOW).
    /// `Split pane down (Ctrl+Shift+O)`.
    NewPaneDown,
    DetachedSplitPane {
        window: iced::window::Id,
        pane: String,
        direction: pane_layout::SplitDirection,
    },
    /// Close the currently focused pane within the active session.
    ClosePane,
    /// Focus a pane by stable id (pointer press or accessible pane action).
    FocusPane(String),
    /// Divider interaction from the framework's composed pane grid.
    PaneGridEvent(enzui_widgets::EnzPaneGridEvent),
    /// Begin renaming the active pane — opens the inline "Pane name" field
    /// seeded with the current title.
    BeginPaneRename,
    /// The inline pane-name field's text changed.
    PaneRenameEdited(String),
    /// Commit the pane rename (writes the draft to the active pane's title).
    /// A blank draft is treated as cancel so a pane never ends up nameless.
    CommitPaneRename,
    /// Abandon the pane rename, restoring the previous title.
    CancelPaneRename,
    /// Test-only direct route. Production detach is available from the
    /// session context menu, never the titlebar.
    #[cfg(test)]
    ToggleDetach,

    // ---- Session-row context menu ---------------------------------------
    /// A right-press landed on the sidebar session row with this id, at this
    /// window-space point.
    ///
    /// The point is an anchor the APP hit-tested,
    /// which is why the host is `enz_point_context_menu_host` (which takes a
    /// point) and not `enz_context_menu_host` (which measures its own trigger).
    OpenContextMenu(String, iced::Point),
    /// Every event the framework context-menu host reports — invocation,
    /// dismissal (outside press / Escape / host), arrow navigation, hover
    /// highlight and row activation — through ONE funnel.
    ///
    /// One variant rather than four, because that is what makes "a pointer
    /// dismissal and an Escape dismissal cannot drift apart" structural: both
    /// arrive as `Dismissed(cause)`, the same arm applies both, and the same
    /// `CloseCause::restores_focus` decides focus restoration for both.
    ///
    /// The highlight lives once, in the `MenuModel`, and the host moves it; the
    /// app keeps no second `active` field beside the surface.
    ContextMenuEvent(enzui_widgets::context_menu_host::ContextMenuEvent),
    /// A context-menu row fired against the session with this id.
    ContextRowActivated(String, context_menu::RowKind),
    /// Begin renaming the active tab/session. This is the target promised by
    /// the `tab.rename` action and its default F2 chord.
    BeginSessionRename,
    /// The inline session-rename field's text changed.
    SessionRenameEdited(String),
    /// Commit the session rename. A blank draft is treated as cancel, so a
    /// session never ends up nameless.
    CommitSessionRename,
    /// Abandon the session rename, keeping the existing title.
    CancelSessionRename,

    /// Dismiss the sidebar overlay drawer (scrim click / Escape).
    DismissSidebar,
    /// The shell's own sidebar toggle: the shell already resolved
    /// the presentation and flipped the right thing, so the app just stores it.
    SidebarState(NavSplitState),
    /// Move pane focus geometrically (`pane.focus-left` .. `-down`, default
    /// Ctrl+Alt+Arrow).
    ///
    /// `pane_layout::focus_direction` — a geometric nearest-neighbour
    /// search over the flattened pane rects, with its own tests — had NO
    /// caller. The four `pane.focus-*` chords were listed in the Shortcuts
    /// dialog and did nothing; with two panes open the only way to move focus
    /// was the mouse.
    FocusPaneDir(pane_layout::FocusDir),
    /// Toggle the sidebar from the KEYBOARD (`ui.toggle-sidebar`, default
    /// Ctrl+Shift+B). Bare Ctrl+B belongs to the PTY.
    ///
    /// The shell's own button publishes [`Self::SidebarState`] (it has
    /// already resolved the presentation); a key press has no such shell
    /// context, so it goes through `toggle_sidebar()`, which resolves the
    /// presentation itself. One helper, two entry points — not two policies.
    ToggleSidebar,
    /// Toggle the narrow-rung titlebar overflow (kebab) menu.
    ///
    /// The overflow TRIGGER owns this: pressing the kebab flips the menu open
    /// or shut. It must stay a toggle, and it must NOT be the dismissal route
    /// (see [`Self::CloseTitlebarOverflow`]).
    ToggleTitlebarOverflow,
    /// Close the framework titlebar menu after dismissal or activation.
    CloseTitlebarOverflow,
    /// Open/close the settings dialog.
    OpenSettings,
    CloseSettings,
    /// The retained settings surface's exit animation reported
    /// settlement. Retires only the generation that painted this frame; a
    /// stale settlement — one whose key does not match the current retained
    /// generation — is dropped by
    /// [`enzui_widgets::modal_host::ModalPresentation::finish_exit`], so a
    /// rapid reopen mid-exit is not torn down by the previous close.
    SettingsExitSettled(ModalPresentationKey),

    /// Every window-chrome gesture the framework `enz_title_bar` can raise
    ///: close, drag, toggle-maximize, minimize, show-system-menu.
    ///
    /// The bar publishes an *intent* rather than a task because a widget does
    /// not know its `iced::window::Id`; the app turns it into a per-window
    /// `Task` through `WindowCommand::into_task`. Every variant is serviced
    /// (one call, exhaustive by construction), because advertising a gesture
    /// that does nothing is worse than advertising none.
    ///
    /// This replaces the old bespoke `CloseWindow`, which was the ONLY window
    /// gesture this app could express — there was no drag, and on Wayland a
    /// compositor-owned drag is the only window-move mechanism.
    Window(WindowCommand),
    WindowState(enzui_widgets::app_shell::ShellWindowEvent),

    /// The settings modal reported a host event:
    /// Escape / scrim-press dismiss, and Tab / Shift+Tab wrap focus **inside**
    /// the dialog rather than walking out into the inert background.
    SettingsModalEvent(ModalHostEvent),
    /// One enzui-center connector event.
    #[cfg(feature = "center")]
    Center(enzui_agent::center::ConnectorEvent),

    /// Tab / Shift+Tab move keyboard focus through the shell's focusable
    /// widgets — including the titlebar's close X (`enz_window_controls`
    /// inside `enz_title_bar`, a composed `FocusButton`).
    ///
    /// iced does not traverse focus on Tab by itself: widgets register in the
    /// focus ring, but only the app can drive an operation, here the
    /// framework's wrapping `traversal_operation`. Without this the
    /// focus-ring-registered close button is mouse-hit-testable but has no
    /// keyboard path.
    ///
    /// # Why not plain Tab, in *this* app
    ///
    /// A terminal must pass keys through: plain Tab is a `\t` byte the child
    /// process is entitled to, and `Ctrl+Tab` / `Ctrl+Shift+Tab` are already
    /// claimed by the shipped keybinding catalog (`keybindings.rs`,
    /// `tabs.next` / `tabs.prev`). Chrome traversal is therefore bound to
    /// **Ctrl+F6 / Ctrl+Shift+F6** — GNOME's F6 "move focus to chrome"
    /// convention, with a Ctrl guard.
    ///
    /// That chord costs the child **nothing**: `keymap::named_to_bytes`
    /// ignores modifiers on every named key except Tab, so today `Ctrl+F6`
    /// and bare `F6` emit the identical `\x1b[17~` and are indistinguishable
    /// to the child. Bare `F6` still reaches the PTY unchanged — pinned by
    /// `plain_f6_still_reaches_the_pty`.
    FocusNext,
    FocusPrevious,
    /// Settings edits — actually consumed.
    SetColorScheme(String),
    /// Set the real logical-pixel terminal font size from the bounded stepper.
    SetFontSizePx(i32),
    /// Select the cursor shape painted by the terminal grid.
    SetCursorStyle(String),
    // `SetBackground(String)` removed with the Dark / Darker / Black
    // presets. Those three buttons were live in the settings dialog and inert
    // for every scheme that owned its canvas — which, after the low-contrast
    // `Default` scheme was deleted, is all of them. A scheme owns its
    // background; there is no second background system to message.
    /// Scrollback size — now a live writer. The stepper in the
    /// Appearance group emits this; the update arm clamps into the
    /// `SCROLLBACK_MIN..=SCROLLBACK_MAX` range advertised by the row's
    /// title and pushes the resolved value into every live PTY via
    /// `PtySession::set_max_scrollback`.
    SetScrollback(i32),

    // ---- SSH connection manager -------------------------------------------
    /// Open/close the SSH manager overlay.
    OpenSshManager,
    CloseSshManager,
    /// The SSH manager modal reported a host event.
    SshManagerModalEvent(ModalHostEvent),
    /// Open the "Add connection" form (blank).
    OpenSshAddForm,
    /// Open the "Edit connection" form, pre-filled from the host at index.
    OpenSshEditForm(usize),
    /// Shared add/edit form event. Edits, secure reveal, submit, keyboard
    /// traversal, Cancel, Escape and scrim dismissal all enter one lifecycle.
    SshFormEvent(FormDialogEvent),
    /// Compatibility commands used by reducer-focused tests; each delegates
    /// immediately to `SshFormEvent`.
    #[cfg(test)]
    CloseSshForm,
    #[cfg(test)]
    SaveSshConnection,
    /// Request deletion of the host at index (shows confirmation).
    DeleteSshHost(usize),
    /// Confirm or cancel the SSH host deletion.
    ConfirmDeleteSshHost(bool),
    /// Confirm-dialog events (buttons / Escape / scrim / focus wrap) for the
    /// delete-host confirmation. A dismissal cancels the delete; the surface is
    /// the framework `enz_confirm_dialog` preset, so the buttons arrive here
    /// too rather than through a second wiring.
    ConfirmDeleteSshHostEvent(ConfirmDialogEvent),
    /// Connect to the app-saved host at index (creates an SSH session).
    ConnectSshHost(usize),
    /// Start the secret-free interactive session that lets OpenSSH ask for an
    /// unconfirmed host key, for the exact pane, profile and connection
    /// generation the refusal belonged to.
    ConfirmHostKey(operation_origin::OperationOrigin),
    // A row index is valid only in the discovery generation that rendered it.
    ConnectOpenSshHost(usize, operation_origin::RequestGeneration),
    ReloadOpenSshConfig,
    OpenSshFirstFrame(iced::window::Id),
    OpenSshDiscoveryFinished(ssh_discovery::Completion),
    #[cfg(test)]
    OpenSshConfigLoaded(Result<Vec<ssh_config::OpenSshHost>, ssh_config::OpenSshConfigError>),
    /// The real Secret Service transport finished connecting. Until
    /// this arrives the store is the truthful `Unavailable` one; there is no
    /// window in which a secret is silently written somewhere else.
    CredentialBackendReady(CredentialBackend),
    /// Capture-only: install a scripted credential backend and seed one saved
    /// profile, so a review shot can show the saved / locked / no-service
    /// states (`capture_keyring`). Never raised by a UI gesture.
    InstallCaptureKeyring(capture_keyring::CaptureKeyring),
    CancelSshCredentialOps,
    SshCredentialStored(
        operation_origin::OperationOrigin,
        String,
        CredentialSlot,
        SshCredentialResult,
    ),
    SshCredentialLoaded(
        operation_origin::OperationOrigin,
        String,
        CredentialSlot,
        u64,
        SshCredentialResult,
    ),
    SshCredentialDeleted(
        operation_origin::OperationOrigin,
        String,
        CredentialSlot,
        SshCredentialResult,
    ),
    SshCredentialReconciled(
        operation_origin::OperationOrigin,
        String,
        CredentialSlot,
        bool,
        SshCredentialResult,
    ),
    CredentialMutationWake,
    RetryCredentialMutations,
    // ---- Appearance and shortcut settings -------------------------------
    /// Reset all appearance settings to defaults.
    ResetAppearance,
    /// Responsive Settings page navigation and narrow-stack Back route.
    SelectSettingsPage(String),
    SettingsBack,
    /// Shared shortcut capture/reset events.
    EditShortcut(EditableShortcutsEvent),
    /// Completion of the off-thread atomic preferences save.
    PreferencesSaved(Result<(), PersistError>),
    PreferencesWorkerFinished(
        operation_origin::RequestGeneration,
        Result<(), PersistError>,
    ),
    SshHostSaveFinished(operation_origin::RequestGeneration, ssh_host_save::Outcome),
    StartupPreferences(persistence_runtime::Delivery<settings_store::AppPreferences>),
    StartupSessions(persistence_runtime::Delivery<session_store::SessionState>),
    /// Completion of one serialized XDG session-state save. The snapshot is
    /// carried back so a change made while this write ran can queue the newer
    /// state only after the older write has finished.
    PinnedStorageFinished(Box<pinned_storage::Completion>),
    RetryPinnedStorage,
    /// The notice with this id was dismissed.
    DismissNotice(String),
    /// A pane's directory is on this git branch (`None`: not a repository).
    GitBranch(String, Option<String>),
    #[cfg(test)]
    CancelPinnedStorage,
    ClearPinnedOutput,
    /// A cooperative request to close one window. The window is still alive:
    /// nothing is destroyed until this app approves the close.
    CloseRequested(iced::window::Id),
    /// One bounded wake of a close that is waiting for its durable debt.
    CloseSettle,
    /// Re-attempt the writes a vetoed close is still waiting for.
    RetryClose,
    /// Re-sweep owned teardown debt after an incomplete cleanup was reported.
    RetryTeardown,
    /// Collect the teardown executor's typed reports while an authorized exit
    /// waits for it.
    TeardownPoll,
    /// Commit a vetoed close, abandoning the state it could not write.
    CloseWithoutSaving,

    // ---- Find bar (cluster B) -------------------------------------------
    /// Open the find bar.
    OpenFind,
    /// Close the find bar.
    CloseFind,
    /// The find query text changed.
    FindQueryChanged(String),
    /// Go to the next match.
    FindNext,
    /// Go to the previous match.
    FindPrev,

    // ---- Clipboard (cluster B) -------------------------------------------
    /// The copy chord (`clipboard.copy`, default Ctrl+Shift+C) fired: copy
    /// the current selection to the system clipboard.
    RequestCopy,
    /// The paste chord (`clipboard.paste`, default Ctrl+Shift+V) fired: begin
    /// an async read of the system clipboard. The bytes are not known yet —
    /// reading the clipboard is a `Task` that resolves to
    /// [`Message::ClipboardPasteReady`].
    RequestPaste,
    DataDropped(
        crate::operation_origin::OperationOrigin,
        enzui_widgets::file_drop::DataDrop,
    ),
    FilesDropped(
        operation_origin::OperationOrigin,
        enzui_widgets::file_drop::FileDrop,
    ),
    FilesCopied(
        operation_origin::OperationOrigin,
        enzui_widgets::file_drop::FileDrop,
        Result<enzui_core::drop_files::CopiedFiles, String>,
    ),
    // DECSET 2004 belongs to the destination pane, not whichever pane currently has focus.
    ClipboardPasteReady(operation_origin::OperationOrigin, Option<String>),
    /// Shared toast-host lifecycle events. Runtime expiry is delivered by a
    /// deadline-keyed one-shot subscription, not a frame or interval loop.
    Toast(ToastEvent),

    // ---- History overlay (cluster C) ------------------------------------
    /// Open the command-history overlay.
    OpenHistory,
    /// FLIP the command-history overlay — the keyboard binding's
    /// semantics. The history accelerator toggled before the catalog dispatcher
    /// landed (`ctrl_shift_h_toggles_history_overlay` pins it), and a key that
    /// opens but cannot close is a shortcut a user must reach for the mouse to
    /// undo.
    ToggleHistory,
    /// Close the command-history overlay.
    CloseHistory,
    /// Copy the complete, untruncated command from a history row.
    CopyHistoryCommand(String),
    /// Take one merged session back out of a group.
    ///
    /// Carries the GROUP's session id and the member's position in
    /// `Session::group_members`, resolved at activation time: the sidebar can
    /// reorder and other sessions can close between the frame that drew this
    /// control and the press.
    UngroupSession(String, usize),
    /// The shared drag host accepted a drop.
    ///
    /// Carried verbatim from `enz_drag_drop_host`'s one `on_item_drop`
    /// publication, because the host is the only participant that knows both
    /// ends of a cross-collection drag. `pane_group::resolve_drop`
    /// decides whether it means anything to this app; everything it refuses
    /// (the inert centre, a non-edge target, a multi-item drag, a drop of the
    /// session already on screen) ends here and changes nothing.
    SessionDropped(enzui_core::drag::ItemDropRequest),
    /// Put a history command back on the terminal's input line.
    ///
    /// The review asked for *"a paste button right to the copy that closes the
    /// history and pastes the command in the terminal"*. Deliberately
    /// **not** routed through the system clipboard: the user's clipboard is
    /// theirs, and a button that silently overwrites it to move a string
    /// inside one app is a side effect nobody asked for. The command's exact
    /// bytes go straight to the active pane's PTY.
    ///
    /// It carries the command rather than an index because the history list
    /// is newest-first and evicts under a bound: an index captured at build
    /// time can name a different command by the time the button is pressed.
    PasteHistoryCommand(String),

    // ---- Test-only SSH form compatibility edits ---
    #[cfg(test)]
    SshFormNameEdited(String),
    #[cfg(test)]
    SshFormDestEdited(String),
    #[cfg(test)]
    SshFormPortEdited(String),
    #[cfg(test)]
    SshFormUsernameEdited(String),
    #[cfg(test)]
    SshFormAuthModeSelected(usize),
    #[cfg(test)]
    SshFormPasswordEdited(String),
    #[cfg(test)]
    SshFormTogglePasswordRevealed,
    #[cfg(test)]
    SshFormKeyPathEdited(String),
    #[cfg(test)]
    SshFormPassphraseEdited(String),
    #[cfg(test)]
    SshFormTogglePassphraseRevealed,
    #[cfg(test)]
    SshFormDefaultPathEdited(String),
    #[cfg(test)]
    SshFormNotesEdited(String),
}
