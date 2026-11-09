use crate::{
    App, Message, SshFormPresentation,
    model::{SshAuthMode, SshHostRecord},
    operation_origin::{RequestGeneration, tests::complete},
    pinned_sessions::tests::dispatch,
    session_store,
    settings_store::{self, AppPreferences},
    ssh_credentials::{CredentialBackend, CredentialSlot},
};
use enzui_core::{
    blocking_task::BlockingTask,
    form::{FieldId, FieldValue},
    persist::{BackendLoad, InMemoryBackend, PersistBackend, PersistError, RawByteLimit, Store},
};
use enzui_services::credential_store::{
    CredentialKey, CredentialOutcome, CredentialPresence, CredentialStore, MemoryCredentialStore,
    PendingCredentialOp, Secret,
};
use iced::{Task, futures::StreamExt};
use iced_test::runtime::Action;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const PASSWORD: &str = "submitted-3035-password";
const ORIGINAL: &str = "last-good-3035-password";
const PRIVATE_PATH: &str = "/private-generated-3035/preferences";

#[derive(Default)]
struct Credentials {
    inner: MemoryCredentialStore,
    calls: [AtomicUsize; 3],
}
impl Credentials {
    fn counts(&self) -> [usize; 3] {
        self.calls
            .each_ref()
            .map(|count| count.load(Ordering::SeqCst))
    }
}
impl CredentialStore for Credentials {
    fn get(&self, key: CredentialKey) -> PendingCredentialOp {
        self.calls[0].fetch_add(1, Ordering::SeqCst);
        self.inner.get(key)
    }
    fn set(&self, key: CredentialKey, secret: Secret) -> PendingCredentialOp {
        self.calls[1].fetch_add(1, Ordering::SeqCst);
        self.inner.set(key, secret)
    }
    fn delete(&self, key: CredentialKey) -> PendingCredentialOp {
        self.calls[2].fetch_add(1, Ordering::SeqCst);
        self.inner.delete(key)
    }
    fn contains(&self, key: &CredentialKey) -> CredentialPresence {
        self.inner.contains(key)
    }
}

#[derive(Clone, Default)]
struct Preferences {
    inner: InMemoryBackend,
    fail: Arc<AtomicBool>,
}
impl Preferences {
    fn store(&self) -> Store<AppPreferences> {
        Store::new(
            self.clone(),
            settings_store::FORMAT_ID,
            settings_store::VERSION,
        )
    }
    fn bytes(&self) -> String {
        self.inner
            .peek(settings_store::FORMAT_ID)
            .expect("seeded preferences")
    }
    fn install(&self, body: &str) {
        self.inner
            .save_raw(settings_store::FORMAT_ID, settings_store::VERSION, body)
            .unwrap();
    }
}
impl PersistBackend for Preferences {
    fn load_raw(&self, format: &str, version: u32) -> BackendLoad {
        self.inner.load_raw(format, version)
    }
    fn save_raw(&self, format: &str, version: u32, body: &str) -> Result<(), PersistError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(PersistError::new(format!(
                "refused write at {PRIVATE_PATH}"
            )));
        }
        self.inner.save_raw(format, version, body)
    }
    fn raw_byte_limit(&self) -> RawByteLimit {
        settings_store::RAW_BYTE_LIMIT
    }
}

fn host(sequence: u64) -> SshHostRecord {
    SshHostRecord {
        id: format!("ssh-host-{sequence}"),
        name: "original profile".into(),
        destination: "original.example.invalid".into(),
        port: 22,
        username: "original-user".into(),
        path: "/original-working-directory".into(),
        auth: SshAuthMode::Password,
        notes: "original metadata".into(),
    }
}

struct Fixture {
    app: App,
    preferences: Preferences,
    credentials: Arc<Credentials>,
}
impl Fixture {
    fn new(sequence: u64) -> Self {
        let preferences = Preferences::default();
        let value = AppPreferences {
            ssh_hosts: vec![host(0)],
            ssh_host_seq: sequence,
            ..Default::default()
        };
        preferences.store().save_blocking(&value).unwrap();
        Self::from_preferences(preferences)
    }
    fn from_preferences(preferences: Preferences) -> Self {
        let (mut app, boot) = App::new_with_stores_and_spawn(
            true,
            preferences.store(),
            Store::new(
                InMemoryBackend::new(),
                session_store::FORMAT_ID,
                session_store::VERSION,
            ),
            false,
            |_, _| panic!("identity fixture must never start a shell"),
        );
        drop(boot);
        assert!(app.ptys.is_empty());
        let credentials = Arc::new(Credentials::default());
        assert_eq!(
            complete(credentials.set(
                CredentialSlot::Password.key("ssh-host-0"),
                Secret::from_string(ORIGINAL.into()),
            )),
            CredentialOutcome::Stored
        );
        dispatch(
            &mut app,
            Message::CredentialBackendReady(CredentialBackend::new(credentials.clone())),
        );
        let mut fixture = Self {
            app,
            preferences,
            credentials,
        };
        fixture.add_form();
        fixture
    }
    fn add_form(&mut self) {
        dispatch(&mut self.app, Message::OpenSshManager);
        dispatch(&mut self.app, Message::OpenSshAddForm);
        self.fill_form();
    }
    fn edit_form(&mut self) {
        dispatch(&mut self.app, Message::OpenSshEditForm(0));
        self.fill_form();
    }
    fn fill_form(&mut self) {
        for message in [
            Message::SshFormNameEdited("  submitted profile  ".into()),
            Message::SshFormDestEdited("  submitted.example.invalid  ".into()),
            Message::SshFormPortEdited("2202".into()),
            Message::SshFormUsernameEdited("submitted-user".into()),
            Message::SshFormDefaultPathEdited("/submitted-working-directory".into()),
            Message::SshFormNotesEdited("submitted metadata".into()),
            Message::SshFormPasswordEdited(PASSWORD.into()),
        ] {
            dispatch(&mut self.app, message);
        }
        let session = self.app.ssh_form.as_ref().unwrap().presenter.session();
        for field in session
            .model()
            .fields()
            .iter()
            .filter(|field| field.required)
        {
            assert!(
                field.is_editable(),
                "fixture fills required fields before saving"
            );
            assert!(!session.value(&field.id).unwrap().is_empty());
        }
        assert!(
            session.clone().submit().is_ok(),
            "fixture supplies a valid submission"
        );
    }
    fn drain_credentials(&mut self) {
        let futures = std::mem::take(&mut self.app.credential_futures);
        assert!(
            !futures.is_empty(),
            "positive credential operations must be exercised"
        );
        for future in futures {
            dispatch(&mut self.app, complete(future));
        }
        assert!(self.app.credential_ops.is_empty());
    }
    fn assert_original_secret(&self) {
        let value = complete(
            self.credentials
                .inner
                .get(CredentialSlot::Password.key("ssh-host-0")),
        );
        let CredentialOutcome::Retrieved(value) = value else {
            panic!("original credential must remain")
        };
        assert_eq!(value.expose_secret(), ORIGINAL.as_bytes());
    }
}

fn values(form: &SshFormPresentation) -> Vec<(FieldId, FieldValue)> {
    let session = form.presenter.session();
    session
        .model()
        .fields()
        .iter()
        .map(|field| (field.id.clone(), session.value(&field.id).unwrap().clone()))
        .collect()
}

struct Before {
    hosts: Vec<SshHostRecord>,
    sequence: u64,
    form: Vec<(FieldId, FieldValue)>,
    editing: Option<String>,
    generation: RequestGeneration,
    bytes: String,
    calls: [usize; 3],
    operations: usize,
    futures: usize,
}
impl Before {
    fn capture(fixture: &Fixture) -> Self {
        let form = fixture.app.ssh_form.as_ref().unwrap();
        Self {
            hosts: fixture.app.ssh_hosts.clone(),
            sequence: fixture.app.ssh_host_seq,
            form: values(form),
            editing: form.editing_id.clone(),
            generation: form.generation.clone(),
            bytes: fixture.preferences.bytes(),
            calls: fixture.credentials.counts(),
            operations: fixture.app.credential_ops.len(),
            futures: fixture.app.credential_futures.len(),
        }
    }
    fn assert_unchanged(&self, fixture: &Fixture) {
        assert_eq!(fixture.app.ssh_hosts, self.hosts);
        assert_eq!(fixture.app.ssh_host_seq, self.sequence);
        let form = fixture
            .app
            .ssh_form
            .as_ref()
            .expect("refusal must retain the submitted form");
        assert_eq!(values(form), self.form);
        assert_eq!(form.editing_id, self.editing);
        assert_eq!(form.generation, self.generation);
        assert_eq!(fixture.preferences.bytes(), self.bytes);
        assert_eq!(fixture.credentials.counts(), self.calls);
        assert_eq!(fixture.app.credential_ops.len(), self.operations);
        assert_eq!(fixture.app.credential_futures.len(), self.futures);
    }
}

fn assert_refusal(fixture: &mut Fixture) {
    let before = Before::capture(fixture);
    dispatch(&mut fixture.app, Message::SaveSshConnection);
    before.assert_unchanged(fixture);
    assert!(fixture.app.pending_ssh_host_save.is_none());
    let form = fixture.app.ssh_form.as_ref().unwrap();
    if let Some(error) = form.save_error.as_deref() {
        assert!(!error.contains(PRIVATE_PATH));
        assert!(error.to_ascii_lowercase().contains("retry"));
    } else {
        assert!(
            form.presenter.session().error_count() > 0,
            "field validation must explain refusal"
        );
        assert!(form.presenter.session().first_invalid().is_some());
    }
    assert!(
        form.presenter
            .session()
            .model()
            .fields()
            .iter()
            .all(|field| !field.read_only)
    );
}

struct HeldWorker {
    release: Option<mpsc::Sender<()>>,
    task: Option<BlockingTask<()>>,
}
impl HeldWorker {
    fn new(app: &App) -> Self {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let task = app.blocking.submit_fifo(move |_| {
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("bounded worker release");
        });
        let held = Self {
            release: Some(release),
            task: Some(task),
        };
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker entered");
        held
    }
    fn finish(mut self) {
        self.release.take().unwrap().send(()).unwrap();
        complete(self.task.take().unwrap()).into_result().unwrap();
    }
}
impl Drop for HeldWorker {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

fn publication(app: &mut App, task: Task<Message>) -> Message {
    let mut stream = iced_test::runtime::task::into_stream(task).expect("publication task");
    let mut result = None;
    for _ in 0..32 {
        match complete(stream.next()) {
            Some(Action::Output(message @ Message::SshHostSaveFinished(..))) => {
                assert!(result.replace(message).is_none(), "one publication receipt");
            }
            Some(Action::Output(message)) => dispatch(app, message),
            Some(Action::Widget(_) | Action::WidgetIn { .. } | Action::Invalidate(_)) => {}
            None => return result.expect("publication completion must be delivered"),
            other => panic!("unexpected publication task action: {other:?}"),
        }
    }
    panic!("publication task did not terminate");
}
