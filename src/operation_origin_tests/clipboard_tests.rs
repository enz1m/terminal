//! Paste and clipboard origins: bracketed paste, clipboard reads,
//! and generation retirement.

use super::*;

#[cfg(target_os = "linux")]
pub(super) struct Sandbox(std::path::PathBuf);

#[cfg(target_os = "linux")]
impl Sandbox {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".operation-origin-fixture-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }

    fn child(&self, name: &str) -> std::path::PathBuf {
        use std::os::unix::fs::DirBuilderExt;
        let path = self.0.join(name);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        for name in ["config", "state", "data", "cache", "runtime"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(path.join(name))
                .unwrap();
        }
        path
    }
}

#[cfg(target_os = "linux")]
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(target_os = "linux")]
pub(super) struct Terminals {
    pub(super) app: App,
    root: Sandbox,
    pub(super) a: String,
    b: String,
}

#[cfg(target_os = "linux")]
pub(super) fn terminals(split: bool) -> Terminals {
    let root = Sandbox::new();
    let mut app = app(&FakeSecretService::new());
    app.sessions.truncate(2);
    let a = app.sessions[0].panes[0].id.clone();
    let b = app.sessions[1].panes[0].id.clone();
    if split {
        let pane = app.sessions.remove(1).panes.remove(0);
        let session = &mut app.sessions[0];
        assert!(
            session
                .layout
                .split_leaf(&a, crate::pane_layout::SplitDirection::Vertical, &b)
        );
        session.panes.push(pane);
        session
            .pane_grid
            .replace_tree(session.layout.pane_grid_tree());
    }
    for (id, name) in [(&a, "a"), (&b, "b")] {
        let path = root.child(name);
        let pty =
            pty::PtySession::spawn_history_probe(id, &path).expect("isolated interactive PTY");
        app.ptys.insert(id.clone(), pty);
    }
    dispatch(&mut app, Message::SelectSession(0));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        dispatch(&mut app, Message::Poll);
        if app
            .sessions
            .iter()
            .flat_map(|session| &session.panes)
            .all(|pane| {
                pane.transcript
                    .iter()
                    .any(|row| row.text.contains("fixture-prompt>"))
            })
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "isolated PTYs did not reach their prompts"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    Terminals { app, root, a, b }
}

/// The reducer bound one paste admission must stay inside. Generous enough for
/// a loaded machine, far below the writer stall it is proving is not waited on.
#[cfg(target_os = "linux")]
const ADMISSION_BOUND: Duration = Duration::from_secs(1);

/// Wait for one session's queue to discharge, then take what the writer wrote.
#[cfg(target_os = "linux")]
pub(super) fn drain_queue(session: &pty::PtySession) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.queued_input() != (0, 0) {
        assert!(
            Instant::now() < deadline,
            "the released writer must discharge its queue"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    session.take_written_input()
}
