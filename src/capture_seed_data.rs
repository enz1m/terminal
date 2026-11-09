//! Capture-only startup input and deterministic terminal transcript.
//!
//! These helpers are isolated from the application reducer so the sizeable
//! review fixture does not add production-source debt to `main.rs`.

use crate::model::{
    HistoryItem, LineTone, Session, SessionKind, SessionStatus, TranscriptLine, TranscriptRow,
};
#[cfg(not(test))]
use crate::pane_layout::SplitDirection;
use crate::{App, model, pane_layout::PaneLayout, pty, scripted_pane};

/// Read the capture-only startup setting.
///
/// Only the literal `"1"` enables it — the same contract `ENZUI_PROBE_LIGHT`
/// uses — so an empty pre-export (the nested compositor sets several vars
/// empty) is off. `main` calls this exactly once and passes the result to
/// application construction; application state never reads the environment.
pub(crate) fn enabled() -> bool {
    enabled_from(std::env::var_os("TERMINAL_EMULATOR_CAPTURE_SEED").as_deref())
}

pub(crate) fn enabled_from(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("1"))
}

impl App {
    pub(super) fn seed_capture_sessions(&mut self) {
        let p0 = self.fresh_pane_id();
        let initial_local_cwd = pty::resolve_local_cwd(None).effective.display().to_string();
        let s0 = Session {
            id: "local-0".into(),
            title: "cmuxterm0".into(),
            subtitle: "zsh".into(),
            icon: "terminal".into(),
            status: SessionStatus::Idle,
            kind: SessionKind::Local,
            ssh_host_id: None,
            connection_error: None,
            pinned: true,
            detached: false,
            panes: vec![scripted_pane(&p0, "Starting shell…", &initial_local_cwd)],
            active_pane: 0,
            pane_grid: PaneLayout::leaf(p0.clone()).pane_grid_state(),
            layout: PaneLayout::leaf(p0.clone()),
            in_app_history: model::CommandHistory::from_items([HistoryItem {
                id: String::new(),
                command: "cargo build".into(),
                when: "10:02".into(),
                exit_code: Some(0),
            }]),
            group_members: Vec::new(),
        };
        let p1 = self.fresh_pane_id();
        let s1 = Session {
            id: "local-1".into(),
            title: "build-watch".into(),
            subtitle: "npm run watch".into(),
            icon: "terminal".into(),
            status: SessionStatus::Ok,
            kind: SessionKind::Local,
            ssh_host_id: None,
            connection_error: None,
            pinned: false,
            detached: false,
            panes: vec![scripted_pane(
                &p1,
                "$ npm run watch\n\n  building… done in 412ms\n  watching for changes",
                "~/proj/web",
            )],
            active_pane: 0,
            pane_grid: PaneLayout::leaf(p1.clone()).pane_grid_state(),
            layout: PaneLayout::leaf(p1),
            in_app_history: model::CommandHistory::new(),
            group_members: Vec::new(),
        };

        let p2 = self.fresh_pane_id();
        let mut s2 = Session {
            id: "ssh-0".into(),
            title: "edge-01".into(),
            subtitle: "ubuntu@edge-01".into(),
            icon: "server".into(),
            status: SessionStatus::Idle,
            kind: SessionKind::Ssh,
            ssh_host_id: Some("edge-01".into()),
            connection_error: None,
            pinned: false,
            detached: false,
            panes: vec![scripted_pane(
                &p2,
                "$ ssh ubuntu@edge-01\nConnection closed.",
                "edge-01:~",
            )],
            active_pane: 0,
            pane_grid: PaneLayout::leaf(p2.clone()).pane_grid_state(),
            layout: PaneLayout::leaf(p2),
            in_app_history: model::CommandHistory::new(),
            group_members: Vec::new(),
        };

        s2.panes[0].endpoint =
            crate::pane_endpoint::PaneEndpoint::LegacySsh(Some("edge-01".into()));
        self.sessions = vec![s0, s1, s2];
        self.active = 0;

        self.apply_capture_seed(true);
    }
}

#[cfg(test)]
impl App {
    pub(crate) fn session_fixture() -> Self {
        let mut app = Self::new(true).0;
        let pane = &mut app.sessions[0].panes[0];
        // Static view fixtures keep a short transcript; I/O fixtures attach their own PTY.
        pane.transcript = scripted_pane(&pane.id, "Starting shell…", &pane.cwd).transcript;
        app
    }
}

#[cfg(not(test))]
impl App {
    /// Apply the optional deterministic 30/70 split used by split-pane
    /// captures.
    pub(super) fn apply_split_capture_seed(&mut self) {
        let Ok(value) = std::env::var("TERMINAL_EMULATOR_CAPTURE_SPLIT") else {
            return;
        };
        let direction = match value.as_str() {
            "horizontal" => SplitDirection::Horizontal,
            "vertical" => SplitDirection::Vertical,
            _ => return,
        };
        let Some(first_id) = self.active_pane_id() else {
            return;
        };
        let second_id = self.fresh_pane_id();
        let Some(session) = self.sessions.get_mut(self.active) else {
            return;
        };
        let Some(mut second) = session.panes.get(session.active_pane).cloned() else {
            return;
        };
        if !session
            .layout
            .split_leaf(&first_id, direction, second_id.clone())
        {
            return;
        }
        if let PaneLayout::Split { ratio, .. } = &mut session.layout {
            *ratio = 0.30;
        }
        second.id = second_id;
        second.title = "Tests".into();
        second.cwd = "/work/tests".into();
        session.panes[0].title = "Editor".into();
        session.panes[0].cwd = "/work/editor".into();
        session.panes.push(second);
        session.active_pane = 1;
        session
            .pane_grid
            .replace_tree(session.layout.pane_grid_tree());
    }
}

/// Representative terminal content for review captures.
///
/// Builds a command echo, coloured status lines, a deliberately long line so
/// wrapping is visible, and enough deterministic output to exercise real
/// scrollback. It is pure in the pane id so capture tests can assert its shape.
pub(crate) fn demo_transcript(pane_id: &str) -> Vec<TranscriptRow> {
    // `TranscriptLine::monochrome` splits on grapheme clusters; the flag and
    // ZWJ family below make that behavior visible in standard shots.
    let rows: [(&str, LineTone, bool); 9] = [
        (
            "$ cargo test -p enzui-widgets --lib",
            LineTone::Default,
            false,
        ),
        (
            "$ echo \u{1F1FA}\u{1F1F8} \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}  # flag + ZWJ family",
            LineTone::Default,
            false,
        ),
        ("   Compiling enzui-widgets v0.1.0", LineTone::Muted, false),
        (
            "    Finished test profile in 4.57s",
            LineTone::Success,
            false,
        ),
        ("running 12 tests", LineTone::Default, false),
        (
            "test result: ok. 12 passed; 0 failed",
            LineTone::Success,
            false,
        ),
        (
            "warning: unused variable `spill` in src/bounds.rs:88",
            LineTone::Warning,
            false,
        ),
        (
            "error: could not resolve host build-cache.internal: Name or service not known \
          — retrying in 5s (attempt 2 of 3)",
            LineTone::Error,
            false,
        ),
        (
            "note: run with RUST_BACKTRACE=1 for a backtrace",
            LineTone::Subtle,
            false,
        ),
    ];
    let mut transcript: Vec<TranscriptRow> = rows
        .iter()
        .enumerate()
        .map(|(i, (text, tone, is_prompt))| {
            TranscriptRow::new(TranscriptLine::monochrome(
                format!("{pane_id}-demo-r{i}"),
                *text,
                *tone,
                *is_prompt,
            ))
        })
        .collect();

    // 240 rows place the coloured output above the fold at every
    // acceptance viewport and make a selection that survives scrolling visible.
    transcript.extend((0..240).map(|i| {
        let tone = if i % 17 == 0 {
            LineTone::Success
        } else if i % 11 == 0 {
            LineTone::Muted
        } else {
            LineTone::Default
        };
        TranscriptRow::new(TranscriptLine::monochrome(
            format!("{pane_id}-demo-r{}", rows.len() + i),
            format!("build shard {i:03}/239  ok   target/debug/deps/terminal_capture_{i:03}"),
            tone,
            false,
        ))
    }));
    transcript.push(TranscriptRow::new(TranscriptLine::monochrome(
        format!("{pane_id}-demo-r{}", transcript.len()),
        "[userm@laptopA1 ~]$",
        LineTone::Default,
        true,
    )));
    transcript
}
