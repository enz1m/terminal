#![cfg(target_os = "linux")]

//! Pinned session restore: atomic publication, eviction, and
//! secret-service failure modes.

use super::*;

fn ready_capture(f: &mut Fixture, command: &str) {
    let session = f.app.sessions[0].id.clone();
    f.app.write_pane_pty(&session, &f.pane, command.as_bytes());
    f.app.write_pane_pty(&session, &f.pane, b"\r");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f.app.ptys[&f.pane]
        .pinned_output()
        .commands()
        .any(|item| item.text.expose_secret() == command)
    {
        assert!(
            Instant::now() < deadline,
            "reader did not settle the capture"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
