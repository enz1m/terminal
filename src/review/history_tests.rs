//! History rendering.

use super::*;

fn history_fixture(command: &str, size: iced::Size) -> App {
    let mut app = App::session_fixture();
    let active = app.active;
    assert!(
        app.sessions[active]
            .in_app_history
            .record(command.to_owned())
    );
    let _ = app.update(Message::ContentResized(size));
    let _ = app.update(Message::OpenHistory);
    app
}
