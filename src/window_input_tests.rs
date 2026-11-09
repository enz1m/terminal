#![cfg(target_os = "linux")]

use super::*;
use crate::operation_origin::tests::complete;
use crate::{grid_view, pty, session_store, settings_store};
use enzui_core::persist::{InMemoryBackend, Store};
use iced::futures::StreamExt;
use iced::keyboard::{
    Location,
    key::{Named, NativeCode, Physical},
};
use iced_test::runtime::Action;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".input-fixture-{}-{time}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
    fn pane(&self, name: &str) -> PathBuf {
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
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    app: App,
    root: Root,
    main: iced::window::Id,
    detached: iced::window::Id,
    a: String,
    b: String,
    session: String,
}

fn fixture() -> Fixture {
    let root = Root::new();
    let (mut app, _) = App::new_with_stores_and_spawn(
        true,
        Store::new(
            InMemoryBackend::new(),
            settings_store::FORMAT_ID,
            settings_store::VERSION,
        ),
        Store::new(
            InMemoryBackend::new(),
            session_store::FORMAT_ID,
            session_store::VERSION,
        ),
        false,
        |_, _| panic!("fixture must not spawn a user shell"),
    );
    app.sessions.truncate(2);
    let a = app.sessions[0].panes[0].id.clone();
    let b = app.sessions[1].panes[0].id.clone();
    for (id, name) in [(&a, "main"), (&b, "detached")] {
        let path = root.pane(name);
        app.ptys.insert(
            id.clone(),
            pty::PtySession::spawn_history_probe(id, &path).expect("private PTY"),
        );
    }
    let main = iced::window::Id::unique();
    let detached = iced::window::Id::unique();
    dispatch(&mut app, Message::MainWindowOpened(main));
    dispatch(
        &mut app,
        Message::ContentResized(iced::Size::new(1400.0, 900.0)),
    );
    let session = app.sessions[1].id.clone();
    app.sessions[1].detached = true;
    dispatch(
        &mut app,
        Message::DetachedWindowOpened {
            session: session.clone(),
            window: detached,
        },
    );
    dispatch(
        &mut app,
        Message::WindowResized(detached, iced::Size::new(900.0, 640.0)),
    );
    dispatch(&mut app, Message::WindowFocusChanged(detached, true));
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
            "private terminals did not become ready"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    Fixture {
        app,
        root,
        main,
        detached,
        a,
        b,
        session,
    }
}

fn settle(app: &mut App, task: Task<Message>) {
    let mut tasks = vec![task];
    for _ in 0..32 {
        let mut next = Vec::new();
        for task in tasks {
            if let Some(mut stream) = iced_test::runtime::task::into_stream(task) {
                let mut exhausted = false;
                for _ in 0..64 {
                    let Some(action) = complete(stream.next()) else {
                        exhausted = true;
                        break;
                    };
                    let Some(action) = crate::operation_origin::tests::settle_window_query(action)
                    else {
                        continue;
                    };
                    match action {
                        Action::Output(message) => next.push(app.update(message)),
                        Action::Widget(_) | Action::WidgetIn { .. } | Action::Invalidate(_) => {}
                        Action::Window(iced_test::runtime::window::Action::Close(_)) => {}
                        other => panic!("unexpected isolated input task: {other:?}"),
                    }
                }
                assert!(exhausted, "bounded task action budget");
            }
        }
        if next.is_empty() {
            return;
        }
        tasks = next;
    }
    panic!("input reducer did not settle");
}
fn dispatch(app: &mut App, message: Message) {
    let task = app.update(message);
    settle(app, task);
}
fn origin(f: &Fixture) -> OperationOrigin {
    f.app.input_origin(&f.session, &f.b).unwrap()
}
fn scoped(app: &mut App, origin: &OperationOrigin, message: Message) {
    dispatch(app, Message::PaneInput(origin.clone(), Box::new(message)));
}

fn native(app: &App, window: iced::window::Id, event: iced::Event, captured: bool) -> Message {
    let mut messages = Vec::new();
    for recipe in iced::advanced::subscription::into_recipes(app.input_subscription()) {
        let input =
            iced::futures::stream::iter([iced::advanced::subscription::Event::Interaction {
                window,
                event: event.clone(),
                status: if captured {
                    iced::event::Status::Captured
                } else {
                    iced::event::Status::Ignored
                },
            }]);
        let mut stream = recipe.stream(Box::pin(input));
        let mut exhausted = false;
        for _ in 0..4 {
            let Some(message) = complete(stream.next()) else {
                exhausted = true;
                break;
            };
            messages.push(message);
        }
        assert!(exhausted, "single-event subscription budget");
    }
    assert_eq!(
        messages.len(),
        1,
        "one event belongs to exactly one window recipe"
    );
    messages.pop().unwrap()
}
fn key(key: Key, modified: Key, text: Option<&str>, modifiers: Modifiers) -> iced::Event {
    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key,
        modified_key: modified,
        physical_key: Physical::Unidentified(NativeCode::Unidentified),
        location: Location::Standard,
        modifiers,
        text: text.map(Into::into),
        repeat: false,
    })
}
fn named(named: Named, mods: Modifiers) -> iced::Event {
    key(Key::Named(named), Key::Named(named), None, mods)
}
fn ime(f: &mut Fixture, event: input_method::Event, captured: bool) {
    let message = native(
        &f.app,
        f.detached,
        iced::Event::InputMethod(event),
        captured,
    );
    dispatch(&mut f.app, message);
}
fn point(column: usize) -> grid_view::GridPointerTarget {
    grid_view::GridPointerTarget {
        window_row: 0,
        column,
        edge: enzui_widgets::text_grid::TextGridEdge::Inside,
        pane: None,
    }
}
fn row(text: &str) -> model::TranscriptRow {
    model::TranscriptRow::new(model::TranscriptLine::monochrome(
        "input-row",
        text,
        model::LineTone::Default,
        false,
    ))
}

fn focus_event(focused: bool) -> iced::Event {
    iced::Event::Window(if focused {
        iced::window::Event::Focused
    } else {
        iced::window::Event::Unfocused
    })
}

#[path = "window_input_tests/detached_tests.rs"]
mod detached_tests;
