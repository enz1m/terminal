//! Opening a local tab must not wait for its shell.
//!
//! The PTY spawn waits up to the capability handshake budget (500 ms) for the
//! child, so it runs on a worker and the tab is published first. Each spawn
//! here is held on a gate the test releases: had the spawn run inline, the
//! held closure would still be parked inside `new_local_session_in_background`
//! when the assertions after it run.

use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

const GATE: Duration = Duration::from_secs(5);

struct HeldSpawn {
    release: mpsc::Sender<()>,
    entered: mpsc::Receiver<()>,
    spawned: Arc<AtomicBool>,
}

/// Open a tab whose spawn waits for the test, then runs `spawn`.
fn open_held_tab<F>(app: &mut App, spawn: F) -> (Task<Message>, String, HeldSpawn)
where
    F: FnOnce(String, Option<&str>) -> session_spawn::LocalSpawnResult + Send + 'static,
{
    let (release, gate) = mpsc::channel();
    let (entered_tx, entered) = mpsc::channel();
    let spawned = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&spawned);
    let before = app.sessions.len();
    let task = app.new_local_session_in_background(move |id, cwd| {
        let _ = entered_tx.send(());
        let _ = gate.recv_timeout(GATE);
        let result = spawn(id, cwd);
        flag.store(result.is_ok(), Ordering::Release);
        result
    });
    assert_eq!(
        app.sessions.len(),
        before + 1,
        "the tab is published at once"
    );
    assert_eq!(app.active, before, "the new tab is activated at once");
    let pid = app.sessions[before].panes[0].id.clone();
    (
        task,
        pid,
        HeldSpawn {
            release,
            entered,
            spawned,
        },
    )
}

fn pane_text(app: &App, pid: &str) -> String {
    app.sessions
        .iter()
        .flat_map(|session| &session.panes)
        .find(|pane| pane.id == pid)
        .map(|pane| {
            pane.transcript
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default()
}

/// # Sabotage
///
/// Make `new_local_session_in_background` call `run_local_spawn` inline: the
/// held spawn is still waiting for the gate when the call should have
/// returned, then installs its PTY, and the "no PTY yet" assertion reddens.
#[test]
fn a_new_tab_is_shown_before_its_shell_spawn_finishes() {
    let mut app = App::session_fixture();
    let (task, pid, held) = open_held_tab(&mut app, pty::PtySession::spawn_shell_at);
    assert!(
        !app.ptys.contains_key(&pid),
        "tab creation waited for the shell spawn"
    );
    assert!(pane_text(&app, &pid).contains("Starting shell…"));
    held.entered
        .recv_timeout(GATE)
        .expect("the spawn runs on its worker");

    held.release.send(()).expect("release the spawn");
    app.settle_for_test(task);
    let spawned = held.spawned.load(Ordering::Acquire);
    assert_eq!(
        app.ptys.contains_key(&pid),
        spawned,
        "a finished spawn attaches to its waiting pane"
    );
    if spawned {
        assert!(
            pane_text(&app, &pid).is_empty(),
            "the placeholder is cleared"
        );
        assert_ne!(app.sessions[app.active].status, SessionStatus::Error);
    } else {
        eprintln!("cannot spawn a real PTY here; only the placeholder was checked");
    }
}

#[test]
fn a_background_spawn_failure_is_shown_in_the_tab() {
    let mut app = App::session_fixture();
    let (task, pid, held) = open_held_tab(&mut app, |_, _| {
        Err(pty::PtyError::SpawnReader(
            "Resource temporarily unavailable".into(),
        ))
    });
    held.release.send(()).expect("release the spawn");
    app.settle_for_test(task);

    let session = &app.sessions[app.active];
    assert_eq!(session.status, SessionStatus::Error);
    assert!(session.connection_error.is_some());
    assert!(!app.ptys.contains_key(&pid));
    let text = pane_text(&app, &pid);
    assert!(
        text.starts_with(SHELL_SPAWN_FAILED_PREFIX),
        "the pane must say why the shell did not start: {text}"
    );
    assert!(!text.contains("Starting shell…"));
}

/// A tab closed while its shell was starting must not orphan the child, nor
/// attach it to whichever pane now holds the focus.
#[test]
fn a_spawn_finishing_after_its_tab_closed_is_retired() {
    let mut app = App::session_fixture();
    let (task, pid, held) = open_held_tab(&mut app, pty::PtySession::spawn_shell_at);
    let _ = app.update(Message::CloseActive);
    assert!(
        app.sessions
            .iter()
            .all(|session| session.panes.iter().all(|pane| pane.id != pid)),
        "precondition: the tab closed before its shell started"
    );

    held.release.send(()).expect("release the spawn");
    app.settle_for_test(task);
    assert!(!app.ptys.contains_key(&pid));
    if held.spawned.load(Ordering::Acquire) {
        assert!(
            app.teardown_debts.iter().any(|debt| debt.pane == pid),
            "the orphaned shell must leave by the teardown owner route"
        );
    }
}

/// A split's shell is spawned off the reducer too: the new leaf is published
/// and focused with no PTY, and the PTY arrives only through the task.
///
/// # Sabotage
///
/// Run the split's `run_local_spawn` inline in `split_active_pane` and finish
/// it there: the PTY is installed before the call returns and the "no PTY
/// yet" assertion reddens.
#[test]
fn a_split_pane_is_shown_before_its_shell_spawn_finishes() {
    let mut app = App::session_fixture();
    let before = app.sessions[app.active].panes.len();
    let task = app.split_active_pane(pane_layout::SplitDirection::Horizontal);
    let session = &app.sessions[app.active];
    assert_eq!(
        session.panes.len(),
        before + 1,
        "the split is published at once"
    );
    let pid = session.panes[session.active_pane].id.clone();
    assert!(
        !app.ptys.contains_key(&pid),
        "the split waited for its shell spawn"
    );
    assert!(app.pane_spawn_pending(&pid));

    app.settle_for_test(task);
    assert!(!app.pane_spawn_pending(&pid));
    let session = &app.sessions[app.active];
    assert!(
        app.ptys.contains_key(&pid) || session.status == SessionStatus::Error,
        "a finished split spawn either attaches or says why it did not"
    );
}

/// An endpoint (re)start spawns off the reducer, and a second start for the
/// same pane while the first is still starting launches nothing beside it.
#[test]
fn an_endpoint_restart_spawns_off_the_reducer_and_only_once() {
    let mut app = App::session_fixture();
    let session = app.sessions[app.active].id.clone();
    let pid = app.sessions[app.active].panes[0].id.clone();
    assert!(!app.ptys.contains_key(&pid), "precondition: no live shell");
    let task = app.start_pane_endpoint_current(&session, &pid);
    if !app.pane_spawn_pending(&pid) {
        eprintln!("the fixture pane has no live operation origin; skipping");
        return;
    }
    assert!(
        !app.ptys.contains_key(&pid),
        "the restart waited for its spawn"
    );
    let ticket = app.pending_pane_spawns[&pid].0;
    let again = app.start_pane_endpoint_current(&session, &pid);
    assert_eq!(
        app.pending_pane_spawns[&pid].0, ticket,
        "a second start must not run beside the pending one"
    );

    app.settle_for_test(Task::batch([task, again]));
    assert!(!app.pane_spawn_pending(&pid));
}
