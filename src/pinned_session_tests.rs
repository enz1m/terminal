#![cfg(target_os = "linux")]

use super::*;
use crate::{operation_origin::tests::complete, pty, settings_store};
use enzui_core::persist::{FileBackend, InMemoryBackend, LoadOutcome, Store};
use enzui_services::credential_store::{
    CredentialOutcome, CredentialStore, FakeMode, FakeSecretService, TransportCredentialStore,
};
use iced::{
    Task,
    futures::StreamExt,
    keyboard::{Key, Modifiers, key::Named},
};
use iced_test::runtime::Action;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".pinned-fixture-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        private_dir(&path);
        Self(path)
    }
    fn pane(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        private_dir(&path);
        for name in ["config", "state", "data", "cache", "runtime"] {
            private_dir(&path.join(name));
        }
        path
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn private_dir(path: &Path) {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path).unwrap();
}
fn prefs() -> Store<settings_store::AppPreferences> {
    Store::new(
        InMemoryBackend::new(),
        settings_store::FORMAT_ID,
        settings_store::VERSION,
    )
}
fn settle(app: &mut App, task: Task<Message>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut tasks = vec![task];
    for _ in 0..32 {
        let mut next = Vec::new();
        for task in tasks {
            if let Some(mut stream) = iced_test::runtime::task::into_stream(task) {
                let mut exhausted = false;
                for _ in 0..128 {
                    assert!(Instant::now() < deadline, "bounded pinned-store settlement");
                    let Some(action) =
                        crate::operation_origin::tests::complete_until(stream.next(), deadline)
                    else {
                        exhausted = true;
                        break;
                    };
                    let Some(action) = crate::operation_origin::tests::settle_window_query(action)
                    else {
                        continue;
                    };
                    match action {
                        Action::Output(message) => next.push(app.update(message)),
                        Action::Widget(_)
                        | Action::WidgetIn { .. }
                        | Action::Invalidate(_)
                        | Action::Exit => {}
                        Action::Window(iced_test::runtime::window::Action::Close(_)) => {}
                        Action::Window(iced_test::runtime::window::Action::Open(id, _, reply)) => {
                            reply.send(id).unwrap();
                        }
                        other => panic!("unexpected private fixture task: {other:?}"),
                    }
                }
                assert!(exhausted, "bounded action stream");
            }
        }
        if next.is_empty() {
            return;
        }
        tasks = next;
    }
    panic!("pinned-store reducer did not settle");
}
pub(crate) fn dispatch(app: &mut App, message: Message) {
    let task = app.update(message);
    settle(app, task);
}
fn open(app: &mut App) -> iced::window::Id {
    let window = iced::window::Id::unique();
    dispatch(app, Message::MainWindowOpened(window));
    dispatch(app, Message::WindowFocusChanged(window, true));
    app.window_inputs.get_mut(&window).unwrap().terminal_focus = true;
    window
}
fn restart(path: &Path) -> App {
    restart_with(path, prefs())
}
fn restart_with(path: &Path, prefs: Store<settings_store::AppPreferences>) -> App {
    let (mut app, _) = App::new_with_stores_and_spawn(
        false,
        prefs,
        session_store::private_store(FileBackend::new(path, session_store::RAW_BYTE_LIMIT)),
        true,
        |_, _| panic!("a loaded/refused workspace must not launch a shell"),
    );
    app.credentials = Arc::new(TransportCredentialStore::new(
        FakeSecretService::with_mode(FakeMode::Unavailable).transport(),
    ));
    app.endpoint_credentials_ready = true;
    app
}
pub(crate) struct Fixture {
    pub(crate) app: App,
    pub(crate) window: iced::window::Id,
    pub(crate) pane: String,
    pub(crate) shell: PathBuf,
    file: PathBuf,
    root: Root,
    service: Arc<FakeSecretService>,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        Self::with_service(Arc::new(FakeSecretService::new()))
    }
    fn with_service(service: Arc<FakeSecretService>) -> Self {
        let root = Root::new();
        let shell = root.pane("shell");
        let file = root.0.join("sessions.state");
        let mut app = App::session_fixture();
        app.secure_pins.capture_mode = false;
        app.sessions.truncate(1);
        app.sessions[0].pinned = true;
        app.session_store = Arc::new(session_store::private_store(FileBackend::new(
            &file,
            session_store::RAW_BYTE_LIMIT,
        )));
        app.credentials = Arc::new(TransportCredentialStore::new(service.transport()));
        app.endpoint_credentials_ready = true;
        app.settings.scrollback_lines = 500;
        app.sessions[0].panes[0].cwd = shell.display().to_string();
        let pane = app.sessions[0].panes[0].id.clone();
        let pty = pty::PtySession::spawn_history_probe(&pane, &shell).unwrap();
        pty.resize(4, 80).unwrap();
        app.install_pty(pane.clone(), pty);
        let window = open(&mut app);
        let mut fixture = Self {
            app,
            window,
            pane,
            shell,
            file,
            root,
            service,
        };
        fixture.wait(|f| f.text().contains("fixture-prompt>"));
        assert!(!fixture.app.ptys[&fixture.pane].pinned_capture_paused());
        fixture
    }
    pub(crate) fn text(&self) -> String {
        self.app
            .sessions
            .iter()
            .flat_map(|s| &s.panes)
            .find(|p| p.id == self.pane)
            .unwrap()
            .transcript
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub(crate) fn scope(&mut self, message: Message) {
        let owner = self
            .app
            .sessions
            .iter()
            .find(|s| s.panes.iter().any(|p| p.id == self.pane))
            .unwrap();
        let origin = self.app.input_origin(&owner.id, &self.pane).unwrap();
        dispatch(&mut self.app, Message::PaneInput(origin, Box::new(message)));
    }
    pub(crate) fn key(&mut self, key: Key, mods: Modifiers) {
        let epoch = self.app.window_inputs[&self.window]
            .focus_generation
            .clone();
        self.scope(Message::InputKey(epoch, key, mods, false));
    }
    pub(crate) fn enter(&mut self) {
        self.key(Key::Named(Named::Enter), Modifiers::empty());
    }
    pub(crate) fn type_text(&mut self, text: &str) {
        self.key(Key::Character(text.into()), Modifiers::empty());
    }
    pub(crate) fn start(&mut self, command: &str) {
        self.type_text(command);
        self.enter();
    }
    pub(crate) fn command(&mut self, command: &str) {
        self.start(command);
        self.wait(|f| {
            f.app.sessions.iter().any(|s| {
                s.in_app_history
                    .newest_first(0)
                    .is_some_and(|item| item.command == command && item.exit_code.is_some())
            })
        });
    }
    pub(crate) fn wait(&mut self, ready: impl Fn(&Self) -> bool) {
        // Real shells under a full parallel run can take seconds to answer.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            dispatch(&mut self.app, Message::Poll);
            if ready(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "private PTY did not reach its protocol boundary"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn state(&self) -> session_store::SessionState {
        match session_store::private_store(FileBackend::new(
            &self.file,
            session_store::RAW_BYTE_LIMIT,
        ))
        .load()
        {
            LoadOutcome::Loaded(state) => state,
            other => panic!("expected actual saved file, got {other:?}"),
        }
    }
    fn payload(&self) -> Option<crate::pinned_secret::Restored> {
        let state = self.state();
        let session = state.sessions.first()?;
        let reference = session.archive.as_ref()?;
        reference.marker.as_ref()?;
        let backend = TransportCredentialStore::new(self.service.transport());
        let CredentialOutcome::Retrieved(secret) =
            complete(backend.get(reference.credential_key()))
        else {
            return None;
        };
        crate::pinned_secret::open(&secret, session, reference).ok()
    }
    pub(crate) fn has(&self, text: &str) -> bool {
        self.payload().is_some_and(|payload| {
            payload.panes.values().any(|(output, _)| {
                output
                    .rows()
                    .any(|row| row.text.expose_secret().contains(text))
            })
        })
    }
    fn restart(&self) -> App {
        // The restarted process reads the preferences this one saved; a
        // different scrollback bound would change every pinned marker and
        // make the restore re-save what it just loaded.
        let saved = prefs();
        saved
            .prepare_save(&settings_store::AppPreferences::snapshot(
                &self.app.settings,
                self.app.shortcuts.overrides(),
                &self.app.ssh_hosts,
            ))
            .unwrap()
            .execute()
            .unwrap();
        let mut app = restart_with(&self.file, saved);
        app.credentials = Arc::new(TransportCredentialStore::new(self.service.transport()));
        app.endpoint_credentials_ready = true;
        open(&mut app);
        app
    }
    pub(crate) fn no_file_bytes(&self, needle: &str) {
        fn visit(path: &Path, needle: &str) {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    visit(&entry.path(), needle);
                } else if entry.file_type().unwrap().is_file() {
                    let bytes = std::fs::read(entry.path()).unwrap();
                    assert!(
                        !bytes
                            .windows(needle.len())
                            .any(|window| window == needle.as_bytes()),
                        "plaintext in a normal fixture file"
                    );
                    let encoded = session_store::hex(needle.as_bytes());
                    assert!(
                        !bytes
                            .windows(encoded.len())
                            .any(|window| window == encoded.as_bytes()),
                        "hex-encoded plaintext in a normal fixture file"
                    );
                }
            }
        }
        visit(&self.root.0, needle);
    }
    pub(crate) fn positive(&mut self) {
        self.command("printf 'PUBLIC_2929 界é\\n'");
        assert!(self.has("PUBLIC_2929 界é"));
    }
    pub(crate) fn absent(&self, marker: &str) {
        assert!(!self.has(marker));
        assert!(
            !std::fs::read_to_string(&self.file)
                .unwrap()
                .contains(&session_store::hex(marker.as_bytes()))
        );
        assert!(
            self.has("PUBLIC_2929"),
            "exclusion needs retained output alongside it"
        );
    }
}
pub(crate) fn reconnect_all(app: &mut App) {
    let selected = app.active;
    let panes: Vec<_> = app
        .sessions
        .iter()
        .enumerate()
        .flat_map(|(si, session)| {
            session
                .panes
                .iter()
                .enumerate()
                .map(move |(pi, pane)| (si, pi, session.id.clone(), pane.id.clone()))
        })
        .collect();
    for (si, pi, session, pane) in panes {
        if !app.restored_outputs.contains_key(&pane) {
            continue;
        }
        let selected_pane = app.sessions[si].active_pane;
        app.sessions[si].active_pane = pi;
        if app.detached_window_of(&session).is_none() {
            app.active = si;
        }
        dispatch(app, Message::Poll);
        let origin = app
            .input_origin(&session, &pane)
            .expect("admitted restored pane");
        dispatch(
            app,
            Message::PaneInput(origin, Box::new(Message::ReconnectRestoredPane)),
        );
        app.sessions[si].active_pane = selected_pane;
    }
    app.active = selected;
    dispatch(app, Message::Poll);
}

#[path = "pinned_session_tests/restore_tests.rs"]
mod restore_tests;
