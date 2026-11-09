use crate::{App, Message, color_scheme, session_store, settings_store};
use enzui_core::{
    editable_shortcuts::EditableShortcutCatalog,
    persist::{LoadCompletion, LoadController, LoadOutcome, LoadSettlement, PersistError, Store},
};
use iced::Task;
use std::sync::{Arc, Mutex};

pub(crate) struct Delivery<T>(Arc<Mutex<Option<LoadCompletion<T>>>>);
impl<T> Clone for Delivery<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> std::fmt::Debug for Delivery<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PersistenceDelivery")
    }
}
impl<T> Delivery<T> {
    fn new(value: LoadCompletion<T>) -> Self {
        Self(Arc::new(Mutex::new(Some(value))))
    }
    fn take(&self) -> Option<LoadCompletion<T>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

pub(crate) struct Startup {
    preferences: LoadController,
    sessions: LoadController,
    prefs: Option<LoadOutcome<settings_store::AppPreferences>>,
    workspace: Option<LoadOutcome<session_store::SessionState>>,
    prefs_failed: bool,
    sessions_failed: bool,
    pub redraw_seen: bool,
    restore_windows: bool,
}
impl Startup {
    pub(crate) fn begin(
        preferences: &Store<settings_store::AppPreferences>,
        sessions: &Store<session_store::SessionState>,
        restore_processes: bool,
    ) -> (Self, Task<Message>) {
        let mut startup = Self {
            preferences: LoadController::new(),
            sessions: LoadController::new(),
            prefs: None,
            workspace: None,
            prefs_failed: false,
            sessions_failed: false,
            redraw_seen: false,
            restore_windows: restore_processes,
        };
        let prefs = match startup.preferences.begin(preferences) {
            Ok(task) => task.map(|value| Message::StartupPreferences(Delivery::new(value))),
            Err(_) => {
                startup.prefs_failed = true;
                Task::none()
            }
        };
        let sessions = match startup.sessions.begin(sessions) {
            Ok(task) => task.map(|value| Message::StartupSessions(Delivery::new(value))),
            Err(_) => {
                startup.sessions_failed = true;
                Task::none()
            }
        };
        (startup, Task::batch([prefs, sessions]))
    }
    pub(crate) fn invalidate(&mut self) {
        self.preferences.invalidate();
        self.sessions.invalidate();
        self.redraw_seen = false;
    }
    fn ready(&self) -> bool {
        self.redraw_seen
            && (self.prefs.is_some() || self.prefs_failed)
            && (self.workspace.is_some() || self.sessions_failed)
    }
    pub(crate) fn accepts(message: &Message) -> bool {
        matches!(
            message,
            Message::StartupPreferences(_)
                | Message::StartupSessions(_)
                | Message::OpenSshFirstFrame(_)
                | Message::Theme(_)
                | Message::ContentResized(_)
                | Message::MainWindowOpened(_)
                | Message::CloseRequested(_)
                | Message::WindowClosed(_)
                | Message::WindowResized(_, _)
                | Message::WindowFocusChanged(_, _)
                | Message::Window(_)
                | Message::WindowState(_)
                | Message::WindowFor(_, _)
                | Message::DetachedTitlebarOverflow(..)
                | Message::WindowChromeKey(..)
                | Message::InputFocus(_)
                | Message::InputCursor(..)
                | Message::ModifiersChanged(_)
                | Message::CredentialBackendReady(_)
                | Message::Poll
        ) || Self::accepts_center(message)
    }

    /// Every connector event reaches the host, which must answer each
    /// call exactly once; a call during startup is refused, not dropped.
    #[cfg(feature = "center")]
    fn accepts_center(message: &Message) -> bool {
        matches!(message, Message::Center(_))
    }

    #[cfg(not(feature = "center"))]
    fn accepts_center(_message: &Message) -> bool {
        false
    }
}
impl App {
    pub(crate) fn startup_notice(&self) -> iced::Element<'_, Message> {
        enzui_widgets::enz_status_page(crate::i18n::label("Loading saved workspace…"))
            .icon("terminal")
            .into_element()
    }
    pub(crate) fn startup_preferences(
        &mut self,
        delivery: Delivery<settings_store::AppPreferences>,
    ) -> Task<Message> {
        if let (Some(startup), Some(completion)) = (&mut self.startup, delivery.take()) {
            match startup.preferences.settle(completion) {
                LoadSettlement::Applied(outcome) => startup.prefs = Some(outcome),
                LoadSettlement::WorkerFailed(_) => startup.prefs_failed = true,
                LoadSettlement::DiscardedStale { .. } => {}
            }
        }
        self.finish_startup()
    }
    pub(crate) fn startup_sessions(
        &mut self,
        delivery: Delivery<session_store::SessionState>,
    ) -> Task<Message> {
        if let (Some(startup), Some(completion)) = (&mut self.startup, delivery.take()) {
            match startup.sessions.settle(completion) {
                LoadSettlement::Applied(outcome) => startup.workspace = Some(outcome),
                LoadSettlement::WorkerFailed(_) => startup.sessions_failed = true,
                LoadSettlement::DiscardedStale { .. } => {}
            }
        }
        self.finish_startup()
    }
    pub(crate) fn finish_startup(&mut self) -> Task<Message> {
        if !self.startup.as_ref().is_some_and(Startup::ready) {
            return Task::none();
        }
        let Some(startup) = self.startup.take() else {
            return Task::none();
        };
        self.startup_settings_refused = startup.prefs_failed;
        self.startup_sessions_refused = startup.sessions_failed;
        if let Some(outcome) = startup.prefs {
            self.settings_load_error = outcome.user_summary();
            let prefs = outcome.into_value();
            self.settings = prefs.settings;
            self.shortcuts = EditableShortcutCatalog::from_overrides(
                Self::shortcut_defaults(),
                prefs.shortcut_overrides,
            );
            self.ssh_hosts = prefs.ssh_hosts;
            self.ssh_host_seq = prefs.ssh_host_seq;
            color_scheme::set_active_by_id(self.settings.resolved_color_scheme());
        } else {
            self.settings_load_error =
                Some("Preferences are unavailable. Saving is disabled until restart.".into());
        }
        let mut spawns = Vec::new();
        if let Some(outcome) = startup.workspace {
            let absent = matches!(&outcome, LoadOutcome::Absent { .. });
            self.session_load_error = outcome.user_summary();
            if absent {
                spawns.push(self.new_local_session());
            } else if self.session_load_error.is_none()
                && !self.install_restored_sessions(&outcome.into_value(), startup.restore_windows)
            {
                // A refusal because the previous workspace's children are
                // still being torn down is not an invalid document, and
                // must not be reported as one.
                self.session_load_error = Some(
                    if self.restore_held_by_teardown {
                        "Terminal processes from the previous workspace are still shutting down. \
                         The saved workspace was not loaded."
                    } else {
                        "Saved workspace is invalid. Saving is disabled until restart."
                    }
                    .into(),
                );
                self.startup_sessions_refused = true;
            }
        } else {
            self.session_load_error =
                Some("Saved workspace is unavailable. Saving is disabled until restart.".into());
        }
        if self.launch_directory.is_some() {
            spawns.push(self.new_local_session());
        }
        self.sync_pinned_capture();
        self.session_snapshot = self.pinned_snapshot();
        self.session_mru.clear();
        if let Some(session) = self.sessions.get(self.active) {
            self.session_mru.push(session.id.clone());
        }
        self.invalidate_sidebar();
        #[cfg(feature = "center")]
        self.center_after_startup();
        spawns.push(self.resume_endpoint_restores());
        Task::batch(spawns)
    }
}

/// The user-facing wording for a dispatcher terminal that ended a save before
/// it reached the store.
///
/// Each class stays distinct: a refusal invites a retry, a supersession points
/// at the newer write that now owes the document, and a stopped or failed
/// worker is neither of those.
pub(crate) fn dispatcher_failure(
    error: enzui_core::blocking_task::BlockingTaskError,
    superseded: bool,
) -> PersistError {
    use enzui_core::blocking_task::BlockingTaskError as Terminal;
    PersistError::new(match error {
        Terminal::Busy => "Saving is busy; preferences were not written. Retry.",
        Terminal::Cancelled if superseded => {
            "A newer preferences change replaced this one before it was written."
        }
        Terminal::Cancelled => "Saving was stopped before preferences were written.",
        Terminal::Shutdown => "The background saver stopped before preferences were written.",
        Terminal::Panicked => "The background saver failed before preferences were written.",
        _ => "Preferences were not written.",
    })
}
