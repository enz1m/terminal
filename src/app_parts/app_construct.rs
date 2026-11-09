//! Terminal App constructors.

use super::*;

impl App {
    pub(crate) fn new(capture_seed: bool) -> (Self, Task<Message>) {
        Self::new_with_stores(
            capture_seed,
            settings_store::store(),
            session_store::store(),
            true,
        )
    }

    pub(crate) fn new_in_directory(capture_seed: bool, directory: String) -> (Self, Task<Message>) {
        Self::new_with_launch_directory_and_spawn(
            capture_seed,
            settings_store::store(),
            session_store::store(),
            true,
            Some(directory),
            pty::PtySession::spawn_explicit_directory,
        )
    }

    /// Boot with injected stores so restart tests share an in-memory backend
    /// and never inspect the developer's XDG directories.
    pub(crate) fn new_with_stores(
        capture_seed: bool,
        settings_store: Store<settings_store::AppPreferences>,
        session_store: Store<session_store::SessionState>,
        restore_processes: bool,
    ) -> (Self, Task<Message>) {
        Self::new_with_stores_and_spawn_configured(
            capture_seed,
            settings_store,
            session_store,
            restore_processes,
            pty::PtySession::spawn_shell_at_with_scrollback,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_stores_and_spawn<F>(
        capture_seed: bool,
        settings_store: Store<settings_store::AppPreferences>,
        session_store: Store<session_store::SessionState>,
        restore_processes: bool,
        spawn: F,
    ) -> (Self, Task<Message>)
    where
        F: FnOnce(
            String,
            Option<&str>,
        ) -> Result<(pty::PtySession, pty::LocalCwdResolution), pty::PtyError>,
    {
        Self::new_with_stores_and_spawn_configured(
            capture_seed,
            settings_store,
            session_store,
            restore_processes,
            |id, cwd, _| spawn(id, cwd),
        )
    }

    pub(crate) fn new_with_stores_and_spawn_configured<F>(
        capture_seed: bool,
        settings_store: Store<settings_store::AppPreferences>,
        session_store: Store<session_store::SessionState>,
        restore_processes: bool,
        spawn: F,
    ) -> (Self, Task<Message>)
    where
        F: FnOnce(
            String,
            Option<&str>,
            usize,
        ) -> Result<(pty::PtySession, pty::LocalCwdResolution), pty::PtyError>,
    {
        Self::new_with_launch_directory_and_spawn(
            capture_seed,
            settings_store,
            session_store,
            restore_processes,
            None,
            spawn,
        )
    }

    pub(crate) fn new_with_launch_directory_and_spawn<F>(
        capture_seed: bool,
        settings_store: Store<settings_store::AppPreferences>,
        session_store: Store<session_store::SessionState>,
        restore_processes: bool,
        launch_directory: Option<String>,
        spawn: F,
    ) -> (Self, Task<Message>)
    where
        F: FnOnce(
            String,
            Option<&str>,
            usize,
        ) -> Result<(pty::PtySession, pty::LocalCwdResolution), pty::PtyError>,
    {
        // Capture sessions must never read or overwrite the user's restart state.
        let session_store = if capture_seed {
            Store::new(
                enzui_core::persist::InMemoryBackend::new(),
                session_store::FORMAT_ID,
                session_store::VERSION,
            )
        } else {
            session_store
        };
        let deferred = !cfg!(test) && !capture_seed;
        let persisted = if deferred {
            enzui_core::persist::LoadOutcome::Absent {
                default: settings_store::AppPreferences::default(),
            }
        } else {
            settings_store.load()
        };
        let settings_load_error = persisted.user_summary();
        let persisted = persisted.into_value();
        let persisted_sessions = if deferred {
            enzui_core::persist::LoadOutcome::Absent {
                default: session_store::SessionState::default(),
            }
        } else {
            session_store.load()
        };
        let sessions_absent = matches!(
            &persisted_sessions,
            enzui_core::persist::LoadOutcome::Absent { .. }
        );
        let session_load_error = persisted_sessions.user_summary();
        let persisted_sessions = persisted_sessions.into_value();
        let settings = persisted.settings;
        let shortcuts = EditableShortcutCatalog::from_overrides(
            Self::shortcut_defaults(),
            persisted.shortcut_overrides,
        );
        let ssh_hosts = persisted.ssh_hosts;
        let ssh_host_seq = persisted.ssh_host_seq;
        let (openssh_hosts, openssh_load_error) = (Vec::new(), None);
        color_scheme::set_active_by_id(settings.resolved_color_scheme());

        let (themes, theme_boot) = ThemeIntegration::boot();
        let blocking = blocking_dispatch::Dispatch::shared();
        #[cfg(not(test))]
        let link_opener = Arc::new(link_open::PortalLinkOpener::new(Arc::clone(&blocking)));
        #[cfg(test)]
        let link_opener = Arc::new(link_open::UnavailableLinkOpener);
        let mut app = App {
            themes,
            screen: Screen::Shell,
            sessions: Vec::new(),
            active: 0,
            settings,
            settings_store,
            session_store: Arc::new(session_store),
            secure_pins: pinned_storage::SecurePins::new(capture_seed),
            session_snapshot: session_store::SessionState::default(),
            restored_outputs: HashMap::new(),
            finished_outputs: HashMap::new(),
            durable_titles: HashMap::new(),
            restored_grid_sizes: HashMap::new(),
            screens: crate::terminal_view::Screens::default(),
            terminal_fit: HashMap::new(),
            terminal_font_family: None,
            session_save_in_flight: false,
            empty_save_requested: false,
            session_load_error,
            startup: None,
            launch_directory: if capture_seed { None } else { launch_directory },
            startup_settings_refused: false,
            startup_sessions_refused: false,
            blocking,
            preferences_write_failed: false,
            preferences_save_generation: operation_origin::RequestGeneration::new(),
            session_save_error: None,
            toasts: ToastHost::new()
                .max_visible(1)
                .waiting_budget(WaitingBudget::unbounded().max_waiting(Some(0)))
                .id_prefix("terminal-emulator-toast"),
            toast_expiry: ToastExpirySchedule::default(),
            shell_activity: Default::default(),
            shortcuts,
            main_window: None,
            window_chrome: HashMap::new(),
            focused_window: None,
            // A detached tab is a window of its own: closing the main window
            // leaves it open, and the app ends with the last window.
            windows: enzui_core::window_lifecycle::WindowLedger::with_policy(
                enzui_core::window_lifecycle::PrimaryClosePolicy::LastWindowWins,
            ),
            detached_windows: HashMap::new(),
            orphaned_windows: Vec::new(),
            intentional_closes: HashMap::new(),
            #[cfg(test)]
            disposed_windows: Vec::new(),
            #[cfg(feature = "native-invalidation-acceptance")]
            native_acceptance: native_invalidation::NativeAcceptance::from_env(),
            settings_page: "appearance".into(),
            settings_drilled_in: false,
            shortcut_armed: None,
            shortcut_feedback: None,
            nav_split: NavSplitState::new(),
            ptys: HashMap::new(),
            teardown_debts: Vec::new(),
            #[cfg(not(test))]
            teardown: pty::teardown_supervisor(),
            // Each test app reaps its own PTYs (see the test `Drop` in
            // `teardown_debts`), so the process executor is left to the tests
            // that measure it instead of queueing every parallel test's shells.
            #[cfg(test)]
            teardown: Box::leak(Box::new(pty::TeardownSupervisor::for_test(1024, 0))),
            restore_held_by_teardown: false,
            exit_withheld: false,
            mods: Modifiers::default(),
            wheel_row_residue: 0.0,
            mouse_last_cell: (1, 1),
            #[cfg(test)]
            pty_write_log: Vec::new(),
            #[cfg(test)]
            targeted_pty_write_log: Vec::new(),
            #[cfg(test)]
            link_authority_override: None,
            pending_ssh_connections: Vec::new(),
            pending_pane_connections: Vec::new(),
            pending_pane_spawns: HashMap::new(),
            next_pane_spawn_ticket: 0,
            host_trust: Vec::new(),
            pending_endpoint_restores: Vec::new(),
            pending_detached_restores: Vec::new(),
            endpoint_credentials_ready: false,
            #[cfg(test)]
            endpoint_spawn_override: None,
            next_credential_token: 1,
            #[cfg(test)]
            ssh_spawn_override: None,
            grid_pointer: GridPointerProbe::default(),
            click_detector: ClickDetector::new(SingleClickPolicy::Immediate),
            selections: HashMap::new(),
            // Start at ZERO so the first frame (before any compositor
            // configure arrives) defaults to the compact presentation.  The
            // resize subscription fires within one event cycle, updating this
            // to the real window geometry.
            content_size: iced::Size::ZERO,
            has_initial_resize: false,
            #[cfg(test)]
            last_pty_resize: None,
            settings_open: false,
            settings_presentation: ModalPresentation::default(),
            settings_focus: None,
            ssh_manager_open: false,
            ssh_manager_focus: None,
            ssh_form: None,
            ssh_hosts,
            openssh_hosts,
            openssh_load_error,
            openssh_discovery: ssh_discovery::Discovery::new(!cfg!(test) && !capture_seed),
            openssh_intent: None,
            // Truthful until the real transport connects (see
            // `credential_backend_task`). Never a plaintext fallback.
            credentials: Arc::new(UnavailableCredentialStore),
            credential_mutation_owner: None,
            #[cfg(feature = "center")]
            center: None,
            #[cfg(feature = "center")]
            center_requested: false,
            credential_mutation_store: None,
            credential_mutation_admission_open: true,
            credential_mutations: Vec::new(),
            credential_ops: Vec::new(),
            paste_requests: Vec::new(),
            pending_secret_deletion: None,
            credential_backend_pinned: false,
            #[cfg(test)]
            credential_futures: Vec::new(),
            link_opener,
            uri_portal: link_open::UriPortal::new(),
            activation_broker: ActivationBroker::default(),
            ssh_credential_notice: None,
            settings_load_error,
            ssh_save_error: None,
            ssh_host_seq,
            pending_ssh_host_save: None,
            ssh_host_save_uncertain: false,
            next_pane_seq: 0,
            find: FindState::default(),
            find_owner: None,
            input_context: None,
            input_frame_start: None,
            transcript_epochs: HashMap::new(),
            input_origins: HashMap::new(),
            window_inputs: HashMap::new(),
            ordered_focus: None,
            opening_window: None,
            typeahead: Vec::new(),
            pending_links: HashMap::new(),
            titlebar_overflow_open: false,
            sidebar_titlebar_overflow_open: false,
            history_open: false,
            history_scroll: enzui_widgets::scroll_view::ScrollGeometryCell::new(),
            grid_area: std::rc::Rc::new(std::cell::Cell::new(None)),
            resized_grid_area: None,
            pane_feedback: None,
            dismissed_notices: std::collections::BTreeSet::new(),
            git_branches: HashMap::new(),
            git_lookups_due: Vec::new(),
            session_mru: Vec::new(),
            sidebar_cache: SubtreeHandle::new(SubtreeId::new("terminal-sidebar")),
            drag_channel: enzui_widgets::drag_drop_host::DragHostChannel::new(),
            pending_delete_ssh_host: None,
            confirm_state: enzui_core::confirm::ConfirmState::new(None),
            pane_rename_draft: None,
            context_menu: None,
            session_rename: None,
            title_overrides: HashSet::new(),
        };
        app.resolve_terminal_family();
        if capture_seed {
            app.seed_capture_sessions();
        } else if !deferred {
            if !sessions_absent && app.session_load_error.is_none() {
                app.install_restored_sessions(&persisted_sessions, restore_processes);
            }
            if sessions_absent || app.launch_directory.is_some() {
                let cap = app.settings.clamped_scrollback() as usize;
                app.new_local_session_with(|id, cwd| spawn(id, cwd, cap));
            }
        }
        #[cfg(not(test))]
        if capture_seed {
            app.apply_split_capture_seed();
            // Visual-QA fixture: exercises the shared toast and persistent
            // banner surfaces without depending on a clipboard or a failing
            // host. The long dwell belongs only to this explicitly selected
            // capture process; ordinary copy uses the host default.
            if std::env::var("TERMINAL_EMULATOR_CAPTURE_STATUS").as_deref() == Ok("feedback") {
                app.pane_feedback = Some(crate::i18n::format(
                    crate::i18n::localizer(),
                    "Pane limit reached: at most {} terminal surfaces can be rendered.",
                    &[("arg0", &(pane_layout::MAX_PANES))],
                ));
                let now = Instant::now();
                app.toasts.push_at(
                    ToastSpec::new(crate::i18n::label("Command copied"))
                        .tone(ToastTone::Success)
                        .duration(Duration::from_secs(3600))
                        .coalesce_key(HISTORY_COPY_TOAST_KEY),
                    now,
                );
                app.toasts.sync_expiry(&mut app.toast_expiry, now);
            }
        }
        app.sync_pinned_capture();
        app.session_snapshot = app.pinned_snapshot();
        let persisted_scrollback = app.settings.clamped_scrollback() as usize;
        for session in app.ptys.values() {
            session.set_max_scrollback(persisted_scrollback);
        }
        app.session_mru.clear();
        if let Some(active) = app.sessions.get(app.active) {
            app.session_mru.push(active.id.clone());
        }

        // Optional startup context menu for screenshots
        // (`ENZUI_TERMINAL_CONTEXT`). The user asked for right-click menu
        // captures, but `capture.sh` shoots on FIRST PAINT and the menu is
        // normally opened by a runtime right-click hit test on a session row
        // — so a running binary has no way to show one for a shot. This
        // seeds `context_menu` at launch on a seeded session:
        //
        //   session  (or any non-empty value) — a menu on the FIRST session.
        //   pinned                            — a menu on a PINNED session,
        //                                        so the pin row reads Unpin.
        //
        // The anchor (40, 120) sits below the ~40px title bar and well inside
        // the 375x812 mobile rung, so the clamped menu stays fully on-screen
        // at every capture size (the host clamps against the live viewport, so
        // a larger window only moves it further from the edges).
        if let Ok(v) = std::env::var("ENZUI_TERMINAL_CONTEXT") {
            let kind = v.trim().to_ascii_lowercase();
            if !kind.is_empty() {
                let index = 0;
                if kind == "pinned"
                    && let Some(s) = app.sessions.get_mut(index)
                {
                    s.pinned = true;
                }
                // Built through the same `menu_model` a real right-click
                // uses, and refused when no row is operable. The host presents
                // only when `ContextMenuState::open` has been called, so a
                // literal that set the fields but not the invocation would seed
                // a menu that photographs as nothing.
                if let Some(id) = app.session_id_at(index)
                    && let Some(s) = app.sessions.get(index)
                {
                    let rows = context_menu::menu_model_in(
                        s.pinned,
                        context_menu::row_context(&app, index),
                    );
                    app.context_menu =
                        context_menu::OpenContextMenu::new(id, iced::Point::new(40.0, 120.0), rows);
                }
            }
        }

        let startup_task = if deferred {
            let (startup, task) = persistence_runtime::Startup::begin(
                &app.settings_store,
                &app.session_store,
                restore_processes,
            );
            app.startup = Some(startup);
            task
        } else {
            Task::none()
        };
        (
            app,
            Task::batch([
                startup_task,
                Self::credential_backend_task(capture_seed),
                theme_boot.map(|event| Message::Theme(Box::new(event))),
            ]),
        )
    }
}
