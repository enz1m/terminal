use super::*;
use crate::model::{SshAuthMode, SshHostRecord};
use crate::ssh_credentials::{CredentialBackend, CredentialNotice};
use crate::{pty, session_store, settings_store};
use enzui_core::persist::{InMemoryBackend, Store};
use enzui_services::credential_store::{
    CredentialOutcome, CredentialStore, FakeSecretService, Secret, TransportCredentialStore,
};
use iced::advanced::clipboard::{Content, Error, Kind};
use iced_test::runtime::Action;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

type Stream = Pin<Box<dyn iced::futures::Stream<Item = Action<Message>> + Send>>;

fn wait<T>(deadline: Instant, mut poll: impl FnMut(&mut Context<'_>) -> Poll<T>) -> T {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        assert!(
            Instant::now() < deadline,
            "operation fixture deadline exceeded"
        );
        match poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                std::thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
            }
        }
    }
}

pub(crate) fn complete<F: Future>(future: F) -> F::Output {
    complete_until(future, Instant::now() + Duration::from_secs(5))
}

pub(crate) fn complete_until<F: Future>(future: F, deadline: Instant) -> F::Output {
    let mut future = std::pin::pin!(future);
    wait(deadline, |context| future.as_mut().poll(context))
}

pub(crate) fn settle_window_query(action: Action<Message>) -> Option<Action<Message>> {
    use iced_test::runtime::window::Action as WindowAction;
    match action {
        Action::Window(WindowAction::GetMaximized(_, reply)) => {
            let _ = reply.send(false);
            None
        }
        Action::Window(WindowAction::GetMode(_, reply)) => {
            let _ = reply.send(iced::window::Mode::Windowed);
            None
        }
        other => Some(other),
    }
}

fn settle(app: &mut App, task: Task<Message>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut tasks = vec![task];
    for _ in 0..16 {
        let mut next = Vec::new();
        for task in tasks {
            if let Some(mut stream) = iced_test::runtime::task::into_stream(task) {
                while let Some(action) =
                    wait(deadline, |context| stream.as_mut().poll_next(context))
                {
                    match action {
                        Action::Output(message) => next.push(app.update(message)),
                        Action::Widget(_) => {}
                        other => panic!("unexpected fixture action: {other:?}"),
                    }
                }
            }
        }
        if next.is_empty() {
            return;
        }
        tasks = next;
    }
    panic!("operation fixture reducer did not settle");
}

fn dispatch(app: &mut App, message: Message) {
    let task = app.update(message);
    settle(app, task);
}

struct ClipboardRead {
    stream: Stream,
    channel: iced::futures::channel::oneshot::Sender<Result<Content, Error>>,
}

impl ClipboardRead {
    fn start(app: &mut App) -> Self {
        let task = app.update(Message::RequestPaste);
        let mut stream = iced_test::runtime::task::into_stream(task).expect("paste issued a task");
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Some(action) = wait(deadline, |context| stream.as_mut().poll_next(context)) {
            match action {
                Action::Clipboard(iced_test::runtime::clipboard::Action::Read {
                    kind,
                    channel,
                }) => {
                    assert!(matches!(kind, Kind::Text));
                    return Self { stream, channel };
                }
                Action::Output(message) => dispatch(app, message),
                Action::Widget(_) => {}
                other => panic!("unexpected paste task action: {other:?}"),
            }
        }
        panic!("RequestPaste did not request clipboard text");
    }

    fn reply(self, app: &mut App, text: Option<&str>) -> Message {
        let Self {
            mut stream,
            channel,
        } = self;
        let result = text
            .map(|text| Content::Text(text.into()))
            .ok_or(Error::ContentNotAvailable);
        channel
            .send(result)
            .expect("clipboard task still awaits its reply");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut result = None;
        while let Some(action) = wait(deadline, |context| stream.as_mut().poll_next(context)) {
            match action {
                Action::Output(message @ Message::ClipboardPasteReady(..)) => {
                    assert!(
                        result.replace(message).is_none(),
                        "one read must produce one completion"
                    );
                }
                Action::Output(message) => dispatch(app, message),
                Action::Widget(_) => {}
                other => panic!("unexpected clipboard completion action: {other:?}"),
            }
        }
        result.expect("clipboard read completion")
    }
}

pub(crate) fn clipboard_reply(app: &mut App, text: Option<&str>) -> Message {
    ClipboardRead::start(app).reply(app, text)
}

fn host(id: &str) -> SshHostRecord {
    SshHostRecord {
        id: id.into(),
        name: id.into(),
        destination: format!("{id}.invalid"),
        port: 22,
        username: "fixture".into(),
        path: String::new(),
        auth: SshAuthMode::Password,
        notes: String::new(),
    }
}

fn app(fake: &FakeSecretService) -> App {
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
        |_, _| panic!("credential fixture must not launch a user shell"),
    );
    app.ssh_hosts = vec![host("alpha"), host("bravo")];
    dispatch(
        &mut app,
        Message::CredentialBackendReady(CredentialBackend::new(Arc::new(
            TransportCredentialStore::new(fake.transport()),
        ))),
    );
    app
}

type Spawns = Arc<Mutex<Vec<(String, Option<String>)>>>;

fn record_spawns(app: &mut App) -> Spawns {
    let spawns = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&spawns);
    app.ssh_spawn_override = Some(Box::new(move |_, host, secret| {
        recorded
            .lock()
            .unwrap()
            .push((host.id.clone(), secret.map(str::to_owned)));
        Err(pty::PtyError::Spawn("generated spawn refusal".into()))
    }));
    spawns
}

fn seed(fake: &FakeSecretService, owner: &str, secret: &str) {
    let store = TransportCredentialStore::new(fake.transport());
    assert_eq!(
        complete(store.set(
            CredentialSlot::Password.key(owner),
            Secret::from_string(secret.into())
        )),
        CredentialOutcome::Stored
    );
}

fn take_credential(app: &mut App) -> crate::CredentialFuture {
    assert_eq!(
        app.credential_futures.len(),
        1,
        "one credential request was dispatched"
    );
    app.credential_futures.pop().unwrap()
}

fn begin_remember(app: &mut App, secret: &str) -> crate::CredentialFuture {
    dispatch(app, Message::OpenSshManager);
    dispatch(app, Message::OpenSshEditForm(0));
    dispatch(app, Message::SshFormPasswordEdited(secret.into()));
    dispatch(app, Message::SaveSshConnection);
    let mut futures = std::mem::take(&mut app.credential_futures);
    assert_eq!(futures.len(), 2, "remember plus inactive-slot cleanup");
    let cleanup = complete(futures.pop().unwrap());
    assert!(matches!(cleanup, Message::SshCredentialDeleted(..)));
    dispatch(app, cleanup);
    futures.pop().unwrap()
}

fn begin_delete(app: &mut App) -> Vec<crate::CredentialFuture> {
    dispatch(app, Message::DeleteSshHost(0));
    dispatch(app, Message::ConfirmDeleteSshHost(true));
    let futures = std::mem::take(&mut app.credential_futures);
    assert_eq!(futures.len(), 2, "both profile slots are deleted");
    futures
}

#[derive(Debug, PartialEq)]
struct State {
    model: u64,
    operations: Vec<(
        OperationOrigin,
        String,
        CredentialSlot,
        CredentialIntent,
        bool,
    )>,
    pastes: Vec<(OperationOrigin, bool)>,
    pending_connections: Vec<(OperationOrigin, u64, String, CredentialSlot)>,
    pending_deletion: Option<(String, Vec<RequestGeneration>, bool)>,
    notices: String,
    saves: (session_store::SessionState, bool),
    writes: Vec<(String, Vec<u8>)>,
}

fn state(app: &App) -> State {
    use std::hash::{Hash, Hasher};
    let mut model = std::collections::hash_map::DefaultHasher::new();
    format!(
        "{:?}",
        (&app.sessions, &app.ssh_hosts, app.active, &app.session_mru)
    )
    .hash(&mut model);
    State {
        model: model.finish(),
        operations: app
            .credential_ops
            .iter()
            .map(|op| {
                (
                    op.origin.clone(),
                    op.owner.clone(),
                    op.slot,
                    op.intent,
                    op.origin.generation.is_live(),
                )
            })
            .collect(),
        pastes: app
            .paste_requests
            .iter()
            .map(|pending| (pending.origin.clone(), pending.origin.generation.is_live()))
            .collect(),
        pending_connections: app
            .pending_ssh_connections
            .iter()
            .map(|pending| {
                (
                    pending.origin.clone(),
                    pending.token,
                    pending.owner.clone(),
                    pending.slot,
                )
            })
            .collect(),
        pending_deletion: app.pending_secret_deletion.as_ref().map(|pending| {
            (
                pending.profile_id.clone(),
                pending.operations.clone(),
                pending.refused,
            )
        }),
        notices: format!(
            "{:?}",
            (
                &app.ssh_credential_notice,
                app.toasts.visible_ids(),
                app.toasts.next_expiry(),
                &app.session_save_error
            )
        ),
        saves: (app.session_snapshot.clone(), app.session_save_in_flight),
        writes: app.targeted_pty_write_log.clone(),
    }
}

fn sentinel(app: &mut App) {
    app.ssh_credential_notice = Some(CredentialNotice::info("current operation notice"));
    app.post_history_copy_acknowledgement_at(Instant::now());
}

fn assert_inert(app: &mut App, message: Message) {
    let before = state(app);
    let task = app.update(message);
    assert_eq!(state(app), before, "stale completion mutated current state");
    assert!(
        iced_test::runtime::task::into_stream(task).is_none(),
        "stale completion scheduled work"
    );
    for pty in app.ptys.values() {
        assert!(
            pty.take_recorded_input().is_empty(),
            "stale completion wrote PTY input"
        );
    }
}

#[path = "operation_origin_tests/clipboard_tests.rs"]
mod clipboard_tests;
#[path = "operation_origin_tests/typing_tests.rs"]
mod typing_tests;
