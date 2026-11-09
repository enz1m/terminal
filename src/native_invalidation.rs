use super::*;

use enzui_core::app_runtime::native_invalidation::{Controller, Window};
use enzui_theme::EnzThemeMode;

const NATIVE_WIDTH: f32 = 580.0;
const NATIVE_HEIGHT: f32 = 660.0;
const SOURCE_ID: &str = match option_env!("ENZUI_ACCEPTANCE_SOURCE_ID") {
    Some(value) => value,
    None => "unbound-source",
};

pub(super) struct NativeAcceptance {
    controller: Controller,
    ready: bool,
}

impl NativeAcceptance {
    pub(super) fn from_env() -> Option<Self> {
        match Controller::from_env("terminal-emulator", SOURCE_ID) {
            Ok(controller) => controller.map(|controller| Self {
                controller,
                ready: false,
            }),
            Err(error) => {
                eprintln!("terminal native acceptance configuration failed: {error}");
                std::process::exit(2);
            }
        }
    }
}

impl App {
    pub(super) fn prepare_native_acceptance(&mut self) -> Task<Message> {
        let Some(expected) = self
            .native_acceptance
            .as_ref()
            .map(|native| native.controller.window_count())
        else {
            return Task::none();
        };

        while self.sessions.len() < expected {
            let source = self
                .sessions
                .last()
                .cloned()
                .expect("BUG: terminal seed must contain a session");
            let index = self.sessions.len();
            let pane_id = self.fresh_pane_id();
            let mut session = source;
            session.id = format!("native-session-{index:02}");
            session.title = format!("native-{index:02}");
            session.detached = false;
            session.panes.truncate(1);
            session.active_pane = 0;
            session.panes[0].id.clone_from(&pane_id);
            session.panes[0].title = format!("native-pane-{index:02}");
            session.panes[0].transcript = capture_demo_transcript(&pane_id);
            session.panes[0].scroll = scrollback::ScrollPosition::pinned_to_tail();
            session.pane_grid = PaneLayout::leaf(pane_id.clone()).pane_grid_state();
            session.layout = PaneLayout::leaf(pane_id);
            self.sessions.push(session);
        }
        for (index, session) in self.sessions.iter_mut().enumerate() {
            let pane = &mut session.panes[session.active_pane];
            let rows = if index == 0 { 240 } else { 20 };
            pane.transcript = (0..rows)
                .map(|row| {
                    TranscriptRow::new(TranscriptLine::monochrome(
                        format!("native-{index:02}-{row:03}"),
                        format!("window {index:02} production transcript row {row:03}"),
                        model::LineTone::Default,
                        false,
                    ))
                })
                .collect();
            pane.scroll = scrollback::ScrollPosition::pinned_to_tail();
        }

        self.active = expected - 1;
        let sessions: Vec<String> = self.sessions[..expected - 1]
            .iter()
            .map(|session| session.id.clone())
            .collect();
        Task::batch(
            sessions
                .iter()
                .map(|session| self.set_session_detached(session, true))
                .collect::<Vec<_>>(),
        )
    }

    pub(super) fn native_acceptance_poll(&mut self) -> Task<Message> {
        let Some(mut native) = self.native_acceptance.take() else {
            return iced::window::invalidate(iced::window::Invalidation::none());
        };
        let expected = native.controller.window_count();
        if !native.ready
            && self.main_window.is_some()
            && self.detached_windows.len() + 1 == expected
            && (self.content_size.width - NATIVE_WIDTH).abs() < 1.0
            && (self.content_size.height - NATIVE_HEIGHT).abs() < 1.0
            && self.detached_windows.values().all(|window| {
                (window.size.width - NATIVE_WIDTH).abs() < 1.0
                    && (window.size.height - NATIVE_HEIGHT).abs() < 1.0
            })
        {
            native.ready = true;
            native
                .controller
                .emit_event("ready", "all windows configured");
        }
        let command = match native.controller.poll() {
            Ok(command) => command,
            Err(error) => {
                native.controller.emit_event("control-error", &error);
                None
            }
        };
        self.native_acceptance = Some(native);
        let Some(command) = command else {
            return iced::window::invalidate(iced::window::Invalidation::none());
        };

        match command.verb.as_str() {
            "reset" => {
                let stage = command.argument.as_deref().unwrap_or("unnamed");
                if let Some(native) = self.native_acceptance.as_mut() {
                    native.controller.reset_stage(stage);
                }
                iced::window::invalidate(iced::window::Invalidation::none())
            }
            "global" => iced::window::invalidate_all(),
            "local" => {
                let Some((window, pane)) = self.native_target() else {
                    return self.native_refusal("local", "target window is not ready");
                };
                iced::Task::batch([
                    iced::window::invalidate(iced::window::Invalidation::none()),
                    iced::Task::done(Message::DetachedScrollTranscriptRows {
                        window,
                        pane,
                        delta: -24,
                    }),
                ])
            }
            "theme" => {
                let theme = match self.enz_theme().mode() {
                    EnzThemeMode::Dark => EnzTheme::light(),
                    EnzThemeMode::Light => EnzTheme::dark(),
                };
                iced::Task::done(Message::Theme(Box::new(ThemeEvent::OsChanged(Box::new(
                    theme,
                )))))
            }
            "report" => {
                let stage = command.argument.as_deref().unwrap_or("unnamed");
                self.native_report(stage);
                iced::window::invalidate(iced::window::Invalidation::none())
            }
            "exit" => iced::exit(),
            verb => self.native_refusal(verb, "unknown command"),
        }
    }

    fn native_target(&self) -> Option<(iced::window::Id, String)> {
        let mut windows: Vec<_> = self.detached_windows.iter().collect();
        windows.sort_by_key(|(_, detached)| detached.session.as_str());
        let (window, detached) = *windows.first()?;
        let window = *window;
        let session = self
            .sessions
            .iter()
            .find(|session| session.id == detached.session)?;
        Some((window, session.panes.get(session.active_pane)?.id.clone()))
    }

    fn native_report(&mut self, stage: &str) {
        let state = self.native_state_signature();
        let mut labelled: Vec<(String, iced::window::Id)> = self
            .detached_windows
            .iter()
            .map(|(id, detached)| (detached.session.clone(), *id))
            .collect();
        labelled.sort_by(|a, b| a.0.cmp(&b.0));
        let mut names = Vec::with_capacity(labelled.len() + 1);
        if let Some(main) = self.main_window {
            names.push(("main".to_owned(), main));
        }
        names.extend(
            labelled
                .into_iter()
                .enumerate()
                .map(|(index, (_, id))| (format!("detached-{index:02}"), id)),
        );
        let windows: Vec<_> = names
            .iter()
            .map(|(label, id)| Window {
                label: label.as_str(),
                id: *id,
            })
            .collect();
        if let Some(native) = self.native_acceptance.as_ref() {
            native.controller.report(stage, &state, &windows);
        }
    }

    fn native_state_signature(&self) -> String {
        let mut detached: Vec<_> = self
            .detached_windows
            .values()
            .map(|window| {
                let viewport = window
                    .viewports
                    .values()
                    .next()
                    .map(|viewport| format!("{:?}", viewport.scroll))
                    .unwrap_or_default();
                format!(
                    "{}:{viewport}:{:.0}x{:.0}",
                    window.session, window.size.width, window.size.height
                )
            })
            .collect();
        detached.sort();
        format!(
            "theme={:?};font={};active={};main={:.0}x{:.0};sessions={};detached={}",
            self.enz_theme().mode(),
            self.settings.font_size_px,
            self.active,
            self.content_size.width,
            self.content_size.height,
            self.sessions.len(),
            detached.join("|")
        )
    }

    fn native_refusal(&self, command: &str, reason: &str) -> Task<Message> {
        if let Some(native) = self.native_acceptance.as_ref() {
            native
                .controller
                .emit_event("command-refused", &format!("{command}: {reason}"));
        }
        iced::window::invalidate(iced::window::Invalidation::none())
    }

    pub(super) fn native_window_title(&self, window: iced::window::Id) -> Option<String> {
        self.native_acceptance.as_ref()?;
        if self.main_window == Some(window) {
            return Some("NativeInv Terminal main".to_owned());
        }
        let mut windows: Vec<_> = self.detached_windows.iter().collect();
        windows.sort_by_key(|(_, detached)| detached.session.as_str());
        windows
            .iter()
            .position(|(id, _)| **id == window)
            .map(|index| format!("NativeInv Terminal detached-{index:02}"))
    }
}
