//! SSH credentials persist through the framework Secret Service
//! seam, and every non-success outcome stays a *typed* state.
//!
//! Every test here drives the REAL reducer path: a UI message goes into
//! `App::update`, the arm builds its credential future through
//! `spawn_credential`, `drive_credentials` runs that exact future and feeds
//! the completion message back into `update`. Nothing is asserted about a
//! helper in isolation.
//!
//! The backend is the framework's own `FakeSecretService` daemon wrapped in
//! `TransportCredentialStore` — the same `CredentialStore` implementation the
//! real D-Bus transport is wrapped in — installed through the same
//! `Message::CredentialBackendReady` production uses. No session bus is
//! touched.

use std::sync::{Arc, Mutex};

use enzui_services::credential_store::{
    CredentialStore, FakeMode, FakeSecretService, MemoryCredentialStore, TransportCredentialStore,
    UnavailableCredentialStore,
};

use crate::model::{SshAuthModeKind, SshHostRecord};
use crate::ssh_credentials::{CredentialBackend, CredentialSlot, SshCredentialResult};
use crate::{App, Message, pty};

const PASSWORD: &str = "prod-password-1896";
const PASSPHRASE: &str = "key-passphrase-1896";

// A booted app with `store` installed through the production message.
fn app_with(store: Arc<dyn CredentialStore>) -> App {
    let mut app = App::session_fixture();
    let _ = app.update(Message::CredentialBackendReady(CredentialBackend::new(
        store,
    )));
    app
}

fn store_over(fake: &FakeSecretService) -> Arc<dyn CredentialStore> {
    Arc::new(TransportCredentialStore::new(fake.transport()))
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    iced_test::futures::futures::executor::block_on(fut)
}

// Fill the add-connection form the way a user does, one message per field, and
// save it. Returns the id of the saved profile.
fn save_profile(app: &mut App, name: &str, kind: SshAuthModeKind, secret: &str) -> String {
    let _ = app.update(Message::OpenSshManager);
    let _ = app.update(Message::OpenSshAddForm);
    let _ = app.update(Message::SshFormNameEdited(name.into()));
    let _ = app.update(Message::SshFormDestEdited("prod.example".into()));
    match kind {
        SshAuthModeKind::Password => {
            let _ = app.update(Message::SshFormAuthModeSelected(0));
            let _ = app.update(Message::SshFormPasswordEdited(secret.into()));
        }
        SshAuthModeKind::KeyFile => {
            let _ = app.update(Message::SshFormAuthModeSelected(1));
            let _ = app.update(Message::SshFormKeyPathEdited("~/.ssh/id_ed25519".into()));
            let _ = app.update(Message::SshFormPassphraseEdited(secret.into()));
        }
        SshAuthModeKind::Agent => {
            let _ = app.update(Message::SshFormAuthModeSelected(2));
        }
    }
    let save = app.update(Message::SaveSshConnection);
    app.settle_for_test(save);
    app.ssh_hosts
        .last()
        .expect("the form saved a profile")
        .id
        .clone()
}

// The secret the app would use for the saved host at `idx`, observed as the
// reducer sees it (the `Retrieved` completion message). The message carries no
// plaintext copy; only this test mints a `String` from the borrowed bytes.
fn load_secret(app: &mut App, idx: usize) -> Option<String> {
    let host = app.ssh_hosts[idx].clone();
    let slot = App::secret_slot_for(&host.auth).expect("this auth mode has a secret");
    let (_, fut) = app
        .load_secret_op(host.id, slot, 0)
        .expect("live credential origin");
    let message = iced_test::futures::futures::executor::block_on(fut);
    let retrieved = match &message {
        Message::SshCredentialLoaded(
            _,
            _,
            _,
            _,
            crate::ssh_credentials::SshCredentialResult::Retrieved(handle),
        ) => handle.with_bytes(|bytes| String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    };
    let _ = app.update(message);
    retrieved
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

// A remembered password outlives the app state that stored it: one daemon and
// two independent apps with independent client stores built from it is a
// restart, and the second app must still read what the first remembered.
#[test]
fn a_remembered_password_survives_an_independent_app_state() {
    let daemon = FakeSecretService::new();

    let mut first = app_with(store_over(&daemon));
    let id = save_profile(
        &mut first,
        "Production",
        SshAuthModeKind::Password,
        PASSWORD,
    );
    let _ = first.drive_credentials();
    assert_eq!(daemon.item_count(), 1, "the daemon holds the item");
    drop(first);

    // A fresh app, a fresh client store, the same daemon.
    let mut second = app_with(store_over(&daemon));
    second.ssh_hosts = vec![SshHostRecord {
        id,
        name: "Production".into(),
        destination: "prod.example".into(),
        port: 22,
        username: String::new(),
        path: String::new(),
        auth: crate::model::SshAuthMode::Password,
        notes: String::new(),
    }];
    assert_eq!(
        load_secret(&mut second, 0).as_deref(),
        Some(PASSWORD),
        "the restarted app must read the password the previous one remembered"
    );
}

// A profile's password and its key passphrase are two items, not one: same
// profile id, both slots stored, each reads back its own value. A single
// shared label would return whichever was written last.
#[test]
fn password_and_passphrase_keys_cannot_collide() {
    let daemon = FakeSecretService::new();
    let store = store_over(&daemon);
    assert_ne!(
        CredentialSlot::Password.label(),
        CredentialSlot::Passphrase.label()
    );

    let mut app = app_with(Arc::clone(&store));
    let id = save_profile(&mut app, "Production", SshAuthModeKind::Password, PASSWORD);
    let _ = app.drive_credentials();
    // Same profile, now switched to key-file auth with a passphrase.
    let _ = app.update(Message::OpenSshEditForm(0));
    let _ = app.update(Message::SshFormAuthModeSelected(1));
    let _ = app.update(Message::SshFormKeyPathEdited("~/.ssh/id_ed25519".into()));
    let _ = app.update(Message::SshFormPassphraseEdited(PASSPHRASE.into()));
    let save = app.update(Message::SaveSshConnection);
    app.settle_for_test(save);
    let _ = app.drive_credentials();

    // Write the password slot again behind the app's back so BOTH slots of the
    // one profile hold a value at the same time.
    let outcome = iced_test::futures::futures::executor::block_on(store.set(
        CredentialSlot::Password.key(&id),
        enzui_services::credential_store::Secret::from_string(PASSWORD.into()),
    ));
    assert!(outcome.is_success());
    assert_eq!(
        daemon.item_count(),
        2,
        "two slots of one profile are two Secret Service items"
    );

    for (slot, expected) in [
        (CredentialSlot::Password, PASSWORD),
        (CredentialSlot::Passphrase, PASSPHRASE),
    ] {
        let outcome = iced_test::futures::futures::executor::block_on(store.get(slot.key(&id)));
        let read = match outcome {
            enzui_services::credential_store::CredentialOutcome::Retrieved(secret) => {
                String::from_utf8_lossy(secret.expose_secret()).into_owned()
            }
            other => panic!("{slot:?} did not read back: {other:?}"),
        };
        assert_eq!(read, expected, "{slot:?} returned another slot's secret");
    }
}

// ---------------------------------------------------------------------------
// Spawn routing: the secret rides the sealed channel, never a live PTY
// ---------------------------------------------------------------------------

// One recorded SSH child spawn: the pane, the profile id, and the exact
// `Option<String>` password the reducer handed the spawn boundary. It holds no
// plaintext the reducer did not itself pass through `spawn_ssh`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SpawnCall {
    pane: String,
    host_id: String,
    password: Option<String>,
}

#[derive(Clone, Default)]
struct SpawnLog(Arc<Mutex<Vec<SpawnCall>>>);

impl SpawnLog {
    fn calls(&self) -> Vec<SpawnCall> {
        self.0.lock().expect("spawn log").clone()
    }
}

// Record each SSH child spawn's password and then fail the spawn, so a gate
// can observe the reducer's decision on any host without a live child.
fn capture_spawns_failing(app: &mut App) -> SpawnLog {
    let log = SpawnLog::default();
    let sink = log.clone();
    app.ssh_spawn_override = Some(Box::new(move |id, host, password| {
        sink.0.lock().expect("spawn log").push(SpawnCall {
            pane: id.to_owned(),
            host_id: host.id.clone(),
            password: password.map(str::to_owned),
        });
        Err(pty::PtyError::Other("test: spawn suppressed".into()))
    }));
    log
}

// Record the password and return a real local shell PTY, so a gate can read the
// writer (`recorded_input`) that a plaintext PTY write would leak into. A
// probe failure here is a loud failure: a live-writer gate needs a real PTY.
fn capture_spawns_live(app: &mut App) -> SpawnLog {
    drop(pty::PtySession::spawn_shell("ssh-1896-probe").expect("open a probe PTY"));
    let log = SpawnLog::default();
    let sink = log.clone();
    app.ssh_spawn_override = Some(Box::new(move |id, host, password| {
        sink.0.lock().expect("spawn log").push(SpawnCall {
            pane: id.to_owned(),
            host_id: host.id.clone(),
            password: password.map(str::to_owned),
        });
        pty::PtySession::spawn_shell(id)
    }));
    log
}

// A password host record with no stored secret, so a connect's read answers
// `NotFound` rather than a retrieval.
fn push_bare_password_host(app: &mut App, id: &str) -> String {
    app.ssh_hosts.push(SshHostRecord {
        id: id.into(),
        name: "Bare".into(),
        destination: "prod.example".into(),
        port: 22,
        username: String::new(),
        path: String::new(),
        auth: crate::model::SshAuthMode::Password,
        notes: String::new(),
    });
    id.into()
}

// The concatenated text the session's transcript projection renders.
fn transcript_text(session: &pty::PtySession) -> String {
    session
        .snapshot_now()
        .into_iter()
        .flatten()
        .flat_map(|row| {
            row.runs
                .iter()
                .map(|run| run.text.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

// A read with no secret still spawns the connection once, with no password, so
// `ssh` prompts in-terminal.
fn assert_degraded_spawn(log: &SpawnLog, host_id: &str, app: &App) {
    let calls = log.calls();
    assert_eq!(
        calls.len(),
        1,
        "a no-secret read must still spawn the degraded connection: {calls:?}"
    );
    assert_eq!(calls[0].host_id, host_id);
    assert_eq!(
        calls[0].password, None,
        "the degraded path spawns with no password, not the plaintext"
    );
    assert!(app.pending_ssh_connections.is_empty());
}

// The prompt substring a built sshpass command tells sshpass to match (`-P
// <prompt>`); `None` when the command took the plain-ssh branch with no `-P`.
fn sshpass_prompt_arg(cmd: &portable_pty::CommandBuilder) -> Option<String> {
    let argv: Vec<String> = cmd
        .get_argv()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let at = argv.iter().position(|a| a == "-P")?;
    argv.get(at + 1).cloned()
}

// Connecting a saved password host retrieves the secret before spawning, and
// the spawn places it only in the sealed `memfd` channel sshpass reads through
// `/proc/<pid>/fd/<n>` — never argv, the environment, or an accessibility
// label. The reducer's own captured secret is fed back through the production
// command builder to prove the sealed placement end to end.
#[test]
#[cfg(target_os = "linux")]
fn connecting_routes_the_secret_only_through_the_sealed_memfd_channel() {
    const SECRET: &str = "sealed-only-1896";

    let daemon = FakeSecretService::new();
    let mut app = app_with(store_over(&daemon));
    let _ = save_profile(&mut app, "Production", SshAuthModeKind::Password, SECRET);
    let _ = app.drive_credentials();
    let host = app.ssh_hosts[0].clone();

    let log = capture_spawns_failing(&mut app);
    let _ = app.update(Message::ConnectSshHost(0));
    // Deferred: nothing is spawned until the credential read resolves.
    assert!(
        log.calls().is_empty(),
        "the child must not spawn before the secret is retrieved"
    );
    let _ = app.drive_credentials();

    let calls = log.calls();
    assert_eq!(calls.len(), 1, "one connect spawns one child: {calls:?}");
    assert_eq!(calls[0].host_id, host.id, "the spawn is the connected host");
    assert_eq!(
        calls[0].password.as_deref(),
        Some(SECRET),
        "the reducer must hand the retrieved secret to the spawn"
    );
    assert!(
        app.pending_ssh_connections.is_empty(),
        "the resolved connection must release its pending slot"
    );

    // Adopt the production sealed path with the reducer's own captured secret:
    // it appears only in the memfd, never argv/env, and the `-f` argument names
    // the non-secret /proc magic path that reads back the exact password line.
    let resolve = |bin: &str| -> Option<std::path::PathBuf> {
        matches!(bin, "ssh" | "sshpass").then(|| std::path::PathBuf::from(format!("/bin/{bin}")))
    };
    let (cmd, channel) = pty::build_ssh_command_with(&host, calls[0].password.as_deref(), resolve)
        .expect("build the sshpass command from the reducer's secret");
    let secret = channel.expect("Some(password) must produce a sealed memfd channel");
    // A password answers ssh's `…'s password:` prompt, so sshpass matches the
    // default password indicator.
    assert_eq!(
        sshpass_prompt_arg(&cmd).as_deref(),
        Some("assword"),
        "a password connect must match the password prompt"
    );
    for arg in cmd.get_argv() {
        assert!(
            !arg.to_string_lossy().contains(SECRET),
            "the secret leaked into argv: {arg:?}"
        );
    }
    for (k, v) in cmd.iter_full_env_as_str() {
        assert!(
            !k.contains(SECRET) && !v.contains(SECRET),
            "the secret leaked into the environment: {k}={v}"
        );
    }
    let payload = std::fs::read_to_string(secret.proc_path()).expect("read the sealed memfd");
    assert_eq!(
        payload,
        format!("{SECRET}\n"),
        "the secret must live only in the sealed memfd channel"
    );

    let _ = app.update(Message::OpenSshManager);
    let labels = manager_labels(&app);
    // Positive control: the manager publishes the profile name, so the label
    // set is non-empty and the secret-absence assertion is not vacuous.
    assert!(
        labels.iter().any(|label| label.contains("Production")),
        "the manager must publish the profile name: {labels:?}"
    );
    assert!(
        labels.iter().all(|label| !label.contains(SECRET)),
        "the secret reached an accessibility label: {labels:?}"
    );
}

// A key-file connection delivers the remembered passphrase, and delivers it at
// the passphrase prompt. OpenSSH prompts `Enter passphrase for key '…':`,
// which contains no `password`, so unless sshpass is told to match
// `passphrase` the secret is never sent and the sealed memfd is retained for
// nothing. The gate drives the real reducer, then rebuilds through the
// production command builder to read the argument choice and the memfd back.
#[test]
#[cfg(target_os = "linux")]
fn a_key_file_connection_delivers_the_passphrase_at_the_passphrase_prompt() {
    const PASSPHRASE: &str = "keyfile-passphrase-1896";

    let daemon = FakeSecretService::new();
    let mut app = app_with(store_over(&daemon));
    let _ = save_profile(&mut app, "Key host", SshAuthModeKind::KeyFile, PASSPHRASE);
    let _ = app.drive_credentials();
    let host = app.ssh_hosts[0].clone();

    let log = capture_spawns_failing(&mut app);
    let _ = app.update(Message::ConnectSshHost(0));
    let _ = app.drive_credentials();

    let calls = log.calls();
    assert_eq!(
        calls.len(),
        1,
        "the key-file connect spawns once: {calls:?}"
    );
    assert_eq!(
        calls[0].password.as_deref(),
        Some(PASSPHRASE),
        "the reducer must hand the retrieved passphrase to the spawn"
    );

    let resolve = |bin: &str| -> Option<std::path::PathBuf> {
        matches!(bin, "ssh" | "sshpass").then(|| std::path::PathBuf::from(format!("/bin/{bin}")))
    };
    let (cmd, channel) = pty::build_ssh_command_with(&host, calls[0].password.as_deref(), resolve)
        .expect("build the sshpass command from the passphrase");
    let secret = channel.expect("Some(passphrase) must produce a sealed memfd channel");
    assert_eq!(
        sshpass_prompt_arg(&cmd).as_deref(),
        Some("passphrase"),
        "a key-file connect must match the passphrase prompt, not the password one"
    );
    for arg in cmd.get_argv() {
        assert!(
            !arg.to_string_lossy().contains(PASSPHRASE),
            "the passphrase leaked into argv: {arg:?}"
        );
    }
    let payload = std::fs::read_to_string(secret.proc_path()).expect("read the sealed memfd");
    assert_eq!(
        payload,
        format!("{PASSPHRASE}\n"),
        "the passphrase must be delivered through the sealed memfd"
    );
}

// The sshpass prompt is chosen per auth slot: a password and a key-file
// passphrase answer different OpenSSH prompts, so one shared indicator would
// silently drop one of the two secrets.
#[test]
#[cfg(target_os = "linux")]
fn sshpass_matches_the_prompt_for_each_auth_slot() {
    let resolve = |bin: &str| -> Option<std::path::PathBuf> {
        matches!(bin, "ssh" | "sshpass").then(|| std::path::PathBuf::from(format!("/bin/{bin}")))
    };
    let host = |auth| SshHostRecord {
        id: "h".into(),
        name: "H".into(),
        destination: "prod.example".into(),
        port: 22,
        username: String::new(),
        path: String::new(),
        auth,
        notes: String::new(),
    };
    let prompt_for = |auth| {
        let (cmd, _channel) = pty::build_ssh_command_with(&host(auth), Some("secret"), resolve)
            .expect("build the sshpass command");
        sshpass_prompt_arg(&cmd).expect("the sshpass branch sets -P")
    };

    let password = prompt_for(crate::model::SshAuthMode::Password);
    let passphrase = prompt_for(crate::model::SshAuthMode::KeyFile {
        key_path: "~/.ssh/id_ed25519".into(),
    });
    assert_eq!(password, "assword");
    assert_eq!(passphrase, "passphrase");
    assert_ne!(
        password, passphrase,
        "each auth slot must match its own prompt"
    );
}

// A resumed connection never writes the secret into a live PTY: the secret
// reaches `ssh` only through the sealed channel, so neither the session writer
// nor the transcript projection carries a secret byte. Both absence assertions
// pair with a benign positive control so neither can pass vacuously.
#[test]
fn a_resumed_connection_never_writes_the_secret_into_the_pty() {
    const SECRET: &str = "pty-never-1896";
    const PROBE: &str = "probe-1896";

    let daemon = FakeSecretService::new();
    let mut app = app_with(store_over(&daemon));
    let _ = save_profile(&mut app, "Production", SshAuthModeKind::Password, SECRET);
    let _ = app.drive_credentials();

    let log = capture_spawns_live(&mut app);
    let _ = app.update(Message::ConnectSshHost(0));
    let _ = app.drive_credentials();

    let calls = log.calls();
    assert_eq!(calls.len(), 1, "one connect spawns one child: {calls:?}");
    assert_eq!(calls[0].password.as_deref(), Some(SECRET));
    let pane = calls[0].pane.clone();
    let session = app
        .ptys
        .get(&pane)
        .expect("the resumed connection installed a live PTY");

    let written = session.take_recorded_input();
    let text = String::from_utf8_lossy(&written);
    assert!(
        !text.contains(SECRET),
        "the secret was written into the PTY: {text:?}"
    );

    assert!(
        !transcript_text(session).contains(SECRET),
        "the secret reached the transcript"
    );
    // Positive control: a benign marker fed to the same projection is rendered,
    // so the transcript absence above was not vacuous.
    session.feed_screen_for_test(PROBE.as_bytes());
    assert!(
        transcript_text(session).contains(PROBE),
        "the transcript projection rendered nothing: {:?}",
        transcript_text(session)
    );

    // Positive control: a benign keystroke reaches the writer, so the writer
    // absence above was not vacuous.
    app.write_active_pty(PROBE.as_bytes());
    let probe = app
        .ptys
        .get(&pane)
        .expect("pty still present")
        .take_recorded_input();
    assert_eq!(
        probe,
        PROBE.as_bytes(),
        "the writer tap must record a benign write"
    );
}

// ---------------------------------------------------------------------------
// Routing: a secret belongs to one profile
// ---------------------------------------------------------------------------

// A profile deletion while the keyring is thinking shifts every later row up.
// The connect captures its host record and reads by stable profile id, and the
// completion resumes the pending connection matched on that id, so Alpha's
// password reaches Alpha's spawn even though Bravo now sits at Alpha's old row.
#[test]
fn a_row_mutation_between_connect_and_resume_never_cross_routes_a_secret() {
    const ALPHA_PASSWORD: &str = "alpha-secret-1896";
    const BRAVO_PASSWORD: &str = "bravo";

    let daemon = FakeSecretService::new();
    let mut app = app_with(store_over(&daemon));
    let _zulu = save_profile(&mut app, "Zulu", SshAuthModeKind::Password, "zulu");
    let _ = app.drive_credentials();
    let alpha = save_profile(&mut app, "Alpha", SshAuthModeKind::Password, ALPHA_PASSWORD);
    let _ = app.drive_credentials();
    let bravo = save_profile(&mut app, "Bravo", SshAuthModeKind::Password, BRAVO_PASSWORD);
    let _ = app.drive_credentials();
    assert_eq!(daemon.item_count(), 3);
    assert_ne!(ALPHA_PASSWORD, BRAVO_PASSWORD);

    let log = capture_spawns_failing(&mut app);
    // The read for Alpha (row 1) is in flight...
    let _ = app.update(Message::ConnectSshHost(1));
    // ...and a row above it is deleted before the completion arrives, so
    // Bravo now sits at Alpha's old row. (Deleting Alpha itself retires its
    // connect: a removed profile has nothing to resume.)
    app.ssh_hosts.remove(0);
    assert_eq!(
        app.ssh_hosts[1].id, bravo,
        "the mutation moved Bravo to Alpha's row"
    );
    let _ = app.drive_credentials();

    let calls = log.calls();
    assert_eq!(
        calls.len(),
        1,
        "the pending connection spawns once: {calls:?}"
    );
    assert_eq!(
        calls[0].host_id, alpha,
        "the spawn must be the host the user connected, not the row's new tenant"
    );
    assert_eq!(
        calls[0].password.as_deref(),
        Some(ALPHA_PASSWORD),
        "Alpha's password must reach Alpha's spawn"
    );
    assert_ne!(
        calls[0].host_id, bravo,
        "Alpha's password was routed to Bravo"
    );
    assert_ne!(calls[0].password.as_deref(), Some(BRAVO_PASSWORD));
}

// The completion message carries no plaintext copy and resumes once. `Message`
// is `Clone` and a `{:?}` on one can reach a log, so the retrieved value stays
// in the framework `Secret`'s wipe-on-drop custody, redacts under `Debug`, and
// is takeable exactly once: a duplicated completion cannot spawn a second child
// or type the password again.
#[test]
fn a_retrieved_secret_is_redacted_and_resumed_once() {
    const SECRET: &str = "once-only-1896";

    let daemon = FakeSecretService::new();
    let mut app = app_with(store_over(&daemon));
    let id = save_profile(&mut app, "Production", SshAuthModeKind::Password, SECRET);
    let _ = app.drive_credentials();

    let log = capture_spawns_failing(&mut app);
    let _ = app.update(Message::ConnectSshHost(0));
    // The reducer parked exactly the load future for this connect; take it so
    // the completion message can be inspected and duplicated.
    let fut = app
        .credential_futures
        .pop()
        .expect("connect issued a credential read");
    let message = iced_test::futures::futures::executor::block_on(fut);

    let handle = match &message {
        Message::SshCredentialLoaded(_, owner, _, _, SshCredentialResult::Retrieved(handle)) => {
            assert_eq!(owner, &id, "the completion names its owner, not a row");
            handle.clone()
        }
        other => panic!("expected a retrieval: {other:?}"),
    };
    assert_eq!(
        handle.with_bytes(<[u8]>::to_vec),
        Some(SECRET.as_bytes().to_vec())
    );

    let printed = format!("{message:?}");
    assert!(
        !printed.contains(SECRET),
        "a secret reached a debug string: {printed}"
    );
    assert!(
        printed.contains("redacted"),
        "the redaction must be visible, not an empty print: {printed}"
    );

    // A cloned completion resumes the connection exactly once.
    let _ = app.update(message.clone());
    let _ = app.update(message);
    assert_eq!(
        log.calls().len(),
        1,
        "a cloned completion must not spawn the connection twice: {:?}",
        log.calls()
    );
    assert!(
        app.pending_ssh_connections.is_empty(),
        "the resumed connection releases its pending slot"
    );
    assert_eq!(
        handle.with_bytes(<[u8]>::to_vec),
        None,
        "the taken secret must no longer be readable from the message"
    );
}

// A read that returns no secret spawns the typed degraded plain `ssh`. Each of
// the three no-secret outcomes — the item is missing, the unlock is denied, or
// the store is unavailable — still spawns the child with no password, so `ssh`
// prompts in-terminal. The connection is a normal continuation, not abandoned.
#[test]
fn a_missing_secret_spawns_the_typed_degraded_plain_ssh() {
    // NotFound: an available store that never stored this profile's item.
    {
        let memory = Arc::new(MemoryCredentialStore::new());
        let mut app = app_with(Arc::clone(&memory) as Arc<dyn CredentialStore>);
        let host = push_bare_password_host(&mut app, "not-found-1896");
        let log = capture_spawns_failing(&mut app);
        let _ = app.update(Message::ConnectSshHost(0));
        let _ = app.drive_credentials();
        assert_degraded_spawn(&log, &host, &app);
    }

    // PermissionDenied: the unlock prompt was refused by policy.
    {
        let daemon = FakeSecretService::with_mode(FakeMode::PermissionDenied);
        let mut app = app_with(store_over(&daemon));
        let host = save_profile(&mut app, "Denied", SshAuthModeKind::Password, PASSWORD);
        let _ = app.drive_credentials();
        let log = capture_spawns_failing(&mut app);
        let _ = app.update(Message::ConnectSshHost(0));
        let _ = app.drive_credentials();
        assert_degraded_spawn(&log, &host, &app);
    }

    // Unavailable: no session bus at all.
    {
        let mut app = app_with(Arc::new(UnavailableCredentialStore));
        let host = save_profile(&mut app, "NoBus", SshAuthModeKind::Password, PASSWORD);
        let _ = app.drive_credentials();
        let log = capture_spawns_failing(&mut app);
        let _ = app.update(Message::ConnectSshHost(0));
        let _ = app.drive_credentials();
        assert_degraded_spawn(&log, &host, &app);
    }
}

// A superseded credential read cannot spawn its stale secret once a second
// connect for the same profile is in flight. Two reads race with the same
// owner and slot, so only the per-connect token distinguishes them: matching
// on owner and slot alone would let the first read's old password spawn the
// connection the user most recently asked for.
#[test]
fn a_superseded_concurrent_read_never_spawns_its_stale_secret() {
    const OLD_PASSWORD: &str = "old-password-1896";
    const NEW_PASSWORD: &str = "new-password-1896";

    let daemon = FakeSecretService::new();
    let mut app = app_with(store_over(&daemon));
    let id = save_profile(
        &mut app,
        "Production",
        SshAuthModeKind::Password,
        OLD_PASSWORD,
    );
    let _ = app.drive_credentials();
    assert_ne!(OLD_PASSWORD, NEW_PASSWORD);

    let log = capture_spawns_failing(&mut app);

    // First connect reads the old password; hold its completion so it can
    // arrive after a second connect has superseded it.
    let _ = app.update(Message::ConnectSshHost(0));
    let stale = block_on(
        app.credential_futures
            .pop()
            .expect("the first connect issued a read"),
    );

    // A profile cannot be edited while its read is live, so the user first
    // cancels the slow keyring wait; the held completion is now superseded.
    let _ = app.update(Message::CancelSshCredentialOps);

    // Rotate the stored password behind a fresh save on the same slot.
    let _ = app.update(Message::OpenSshEditForm(0));
    let _ = app.update(Message::SshFormPasswordEdited(NEW_PASSWORD.into()));
    let save = app.update(Message::SaveSshConnection);
    app.settle_for_test(save);
    let _ = app.drive_credentials();

    // Second connect supersedes the first and reads the new password.
    let _ = app.update(Message::ConnectSshHost(0));
    let fresh = block_on(
        app.credential_futures
            .pop()
            .expect("the second connect issued a read"),
    );

    // The superseded completion arrives late; only the current read may spawn.
    let _ = app.update(stale);
    let _ = app.update(fresh);

    let calls = log.calls();
    assert_eq!(
        calls.len(),
        1,
        "the superseded read must not spawn: {calls:?}"
    );
    assert_eq!(calls[0].host_id, id);
    assert_eq!(
        calls[0].password.as_deref(),
        Some(NEW_PASSWORD),
        "the current connect must spawn with the current secret"
    );
    assert!(
        calls
            .iter()
            .all(|c| c.password.as_deref() != Some(OLD_PASSWORD)),
        "a superseded read spawned its stale secret: {calls:?}"
    );
    assert!(app.pending_ssh_connections.is_empty());
}

// A stale password read cannot satisfy a later passphrase route for the same
// profile. The profile is reconfigured from password to key-file auth between
// two connects, so a completion routed by owner alone would seal the old
// password into the passphrase connection.
#[test]
fn a_stale_password_read_never_satisfies_a_later_passphrase_route() {
    const PW: &str = "cross-password-1896";
    const PP: &str = "cross-passphrase-1896";

    let daemon = FakeSecretService::new();
    let mut app = app_with(store_over(&daemon));
    let id = save_profile(&mut app, "Production", SshAuthModeKind::Password, PW);
    let _ = app.drive_credentials();

    let log = capture_spawns_failing(&mut app);

    // Connect the password profile; hold the password read's completion.
    let _ = app.update(Message::ConnectSshHost(0));
    let stale = block_on(
        app.credential_futures
            .pop()
            .expect("the password connect issued a read"),
    );

    // A profile cannot be edited while its read is live, so the user first
    // cancels the slow keyring wait; the held completion is now superseded.
    let _ = app.update(Message::CancelSshCredentialOps);

    // Reconfigure the same profile to key-file auth with a passphrase.
    let _ = app.update(Message::OpenSshEditForm(0));
    let _ = app.update(Message::SshFormAuthModeSelected(1));
    let _ = app.update(Message::SshFormKeyPathEdited("~/.ssh/id_ed25519".into()));
    let _ = app.update(Message::SshFormPassphraseEdited(PP.into()));
    let save = app.update(Message::SaveSshConnection);
    app.settle_for_test(save);
    let _ = app.drive_credentials();

    // Connect again; this read resolves the passphrase slot.
    let _ = app.update(Message::ConnectSshHost(0));
    let fresh = block_on(
        app.credential_futures
            .pop()
            .expect("the passphrase connect issued a read"),
    );

    let _ = app.update(stale);
    let _ = app.update(fresh);

    let calls = log.calls();
    assert!(
        calls.iter().all(|c| c.password.as_deref() != Some(PW)),
        "a stale password read satisfied the passphrase route: {calls:?}"
    );
    assert_eq!(calls.len(), 1, "only the current read spawns: {calls:?}");
    assert_eq!(calls[0].host_id, id);
    assert_eq!(
        calls[0].password.as_deref(),
        Some(PP),
        "the passphrase route must spawn with its own secret"
    );
    assert!(app.pending_ssh_connections.is_empty());
}

// Every label the real SSH-manager tree publishes at desktop size.
fn manager_labels(app: &App) -> Vec<String> {
    use enzui_core::collect::SemanticsCollector;
    use iced::advanced::renderer::Headless;
    use iced::advanced::widget::Operation;

    let size = iced::Size {
        width: 1400.0,
        height: 900.0,
    };
    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        app.view(),
        size,
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    let mut collector = SemanticsCollector::new();
    ui.operate(&renderer, {
        let op: &mut dyn Operation = &mut collector;
        op
    });
    collector
        .into_collected()
        .into_iter()
        .filter_map(|node| node.semantics.label)
        .collect()
}

#[path = "ssh_credential_persistence_tests/outcome_tests.rs"]
mod outcome_tests;
