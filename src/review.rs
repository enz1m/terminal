//! Consequence gates for find, history, grouping and grid tokens.
//!
//! Every test here measures the shipped code path, not a helper's return
//! value, and every one is paired with the exact source edit that reddens it
//! (recorded per test under *Sabotage*). None of them could be run in this
//! session: the shared app target's `enzui_core` rlib does not link
//! (`rust-lld: undefined hidden symbol`, reproduced on a plain
//! `cargo build --bin terminal-emulator`, and on `calendar-app` at the same
//! time), so these are calibrated at source level and carry **no runtime
//! green claim**.

use crate::model::{self};
use crate::pane_layout::PaneLayout;
use crate::{App, Message};

// ---------------------------------------------------------------------------
// item 2 — the session rail has a gap between rows
// ---------------------------------------------------------------------------

fn fixture_session(id: &str, pane_id: &str) -> model::Session {
    let layout = PaneLayout::leaf(pane_id);
    model::Session {
        id: id.into(),
        title: format!("session {id}"),
        subtitle: "zsh".into(),
        icon: "terminal".into(),
        status: model::SessionStatus::Idle,
        kind: model::SessionKind::Local,
        ssh_host_id: None,
        connection_error: None,
        pinned: false,
        detached: false,
        panes: vec![crate::scripted_pane(pane_id, "ready", "/tmp")],
        active_pane: 0,
        pane_grid: layout.pane_grid_state(),
        layout,
        in_app_history: model::CommandHistory::new(),
        group_members: Vec::new(),
    }
}

pub(crate) mod probe;

#[path = "review/group_tests.rs"]
mod group_tests;
#[path = "review/history_tests.rs"]
mod history_tests;
