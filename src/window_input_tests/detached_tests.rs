#![cfg(target_os = "linux")]

//! Detached-pane input: find, resize, links, close, and shortcuts.

use super::*;

fn activation_reply(app: &mut App, task: Task<Message>, window: iced::window::Id) -> Message {
    let mut stream = iced_test::runtime::task::into_stream(task).expect("activation task");
    let mut reply = None;
    let mut exhausted = false;
    for _ in 0..64 {
        let Some(action) = complete(stream.next()) else {
            exhausted = true;
            break;
        };
        match action {
            Action::Window(iced_test::runtime::window::Action::RequestActivationToken(
                source,
                channel,
            )) => {
                assert_eq!(source, window);
                channel
                    .send(Err(iced::window::ActivationError::Unsupported))
                    .unwrap();
            }
            Action::Output(message @ Message::OwnedUriActivation(..)) => reply = Some(message),
            Action::Output(message) => dispatch(app, message),
            Action::Widget(_) | Action::WidgetIn { .. } => {}
            other => panic!("unexpected activation action {other:?}"),
        }
    }
    assert!(exhausted, "bounded activation task");
    reply.expect("actual activation response")
}

fn ime_strategy(app: &App, window: iced::window::Id) -> input_method::InputMethod {
    use iced::advanced::renderer::Headless;
    let size = app.detached_windows[&window].size;
    let mut renderer = complete(<iced_test::renderer::Renderer as Headless>::new(
        iced::advanced::renderer::Settings::default(),
        std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
    ))
    .unwrap();
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        app.view_window(window),
        size,
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let (state, _) = ui.update(
        &iced_test::core::window::Headless,
        &iced_test::core::shell::Waker::noop(),
        &[iced::Event::Window(iced::window::Event::RedrawRequested(
            iced_test::core::time::Instant::now(),
        ))],
        iced::mouse::Cursor::Unavailable,
        &mut renderer,
        &mut iced::advanced::shell::Bus::new(),
    );
    match state {
        iced_test::runtime::user_interface::State::Updated { input_method, .. } => input_method,
        _ => panic!("IME surface must publish a stable strategy"),
    }
}
