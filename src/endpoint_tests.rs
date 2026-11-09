use super::*;
use crate::model::{SshAuthMode, SshHostRecord};
use crate::operation_origin::tests::complete;
use crate::pane_group::GroupEdge;
use crate::session_store::{SessionState, hex};
use crate::ssh_credentials::CredentialBackend;
use crate::{context_menu, session_store, settings_store};
use enzui_core::persist::{InMemoryBackend, Persistable, Store};
use enzui_services::credential_store::{
    CredentialOutcome, CredentialStore, MemoryCredentialStore, Secret,
};
use iced::futures::StreamExt;
use std::sync::{Arc, Mutex};

type Calls = Arc<Mutex<Vec<(String, PaneEndpoint, String, Option<String>)>>>;

fn profile(id: &str, auth: SshAuthMode) -> SshHostRecord {
    SshHostRecord {
        id: id.into(),
        name: format!("Profile {id}"),
        destination: format!("{id}.invalid"),
        port: 2222,
        username: format!("user-{id}"),
        path: format!("/srv/{id}"),
        auth,
        notes: "not endpoint metadata".into(),
    }
}

fn fixture() -> (App, Arc<MemoryCredentialStore>, Calls) {
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
        |_, _| panic!("fixture must not launch the user's shell"),
    );
    app.sessions.truncate(1);
    let store = Arc::new(MemoryCredentialStore::new());
    dispatch(
        &mut app,
        Message::CredentialBackendReady(CredentialBackend::new(store.clone())),
    );
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&calls);
    app.endpoint_spawn_override = Some(Box::new(move |id, endpoint, cwd, password| {
        log.lock().unwrap().push((
            id.into(),
            endpoint.clone(),
            cwd.into(),
            password.map(str::to_owned),
        ));
        Err(pty::PtyError::Spawn(
            "generated endpoint launch refusal".into(),
        ))
    }));
    (app, store, calls)
}

fn dispatch(app: &mut App, message: Message) {
    let mut tasks = vec![app.update(message)];
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
                        iced_test::runtime::Action::Output(message) => {
                            next.push(app.update(message));
                        }
                        iced_test::runtime::Action::Widget(_)
                        | iced_test::runtime::Action::WidgetIn { .. } => {}
                        iced_test::runtime::Action::Window(
                            iced_test::runtime::window::Action::Open(id, _, reply),
                        ) => {
                            reply.send(id).expect("window task receiver");
                        }
                        other => panic!("unexpected endpoint fixture task: {other:?}"),
                    }
                }
                assert!(exhausted, "endpoint fixture task exceeded its action bound");
            }
        }
        if next.is_empty() {
            return;
        }
        tasks = next;
    }
    panic!("endpoint fixture did not settle");
}

fn credentials(app: &mut App) {
    for future in std::mem::take(&mut app.credential_futures) {
        dispatch(app, complete(future));
    }
}

fn add_profile(app: &mut App, host: &SshHostRecord) -> String {
    app.ssh_hosts.push(host.clone());
    app.connect_ssh_record_with(host.clone(), None, |_, _, _| {
        Err(pty::PtyError::Spawn("source fixture".into()))
    });
    app.sessions[app.active].id.clone()
}

fn snapshot(app: &App) -> SessionState {
    let mut sessions = app.sessions.clone();
    for session in &mut sessions {
        session.pinned = true;
    }
    SessionState::snapshot(&sessions, app.active, &app.title_overrides)
}

fn roundtrip(app: &App) -> SessionState {
    let state = snapshot(app);
    let store = Store::new(
        InMemoryBackend::new(),
        session_store::FORMAT_ID,
        session_store::VERSION,
    );
    store.prepare_save(&state).unwrap().execute().unwrap();
    let restored = store.load().into_value();
    assert_eq!(restored, state);
    restored
}

fn open_main_and_reconnect(app: &mut App) {
    dispatch(app, Message::MainWindowOpened(iced::window::Id::unique()));
    if app.endpoint_credentials_ready {
        crate::pinned_sessions::tests::reconnect_all(app);
    }
}

#[test]
fn ssh_split_uses_a_fresh_credential_read_and_keeps_its_endpoint_and_cwd() {
    let (mut app, store, calls) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    add_profile(&mut app, &host);
    let source = app.active;
    app.sessions[source].panes[0].cwd = "alpha.invalid:/work/colon:/kept".into();
    assert_eq!(
        complete(store.set(
            crate::ssh_credentials::CredentialSlot::Password.key(&host.id),
            Secret::from_string("generated split secret".into())
        )),
        CredentialOutcome::Stored
    );
    dispatch(&mut app, Message::NewPane);
    assert_eq!(store.get_call_count(), 1);
    assert_eq!(app.pending_pane_connections.len(), 1);
    assert!(calls.lock().unwrap().is_empty());
    let new_id = app.sessions[source].panes[1].id.clone();
    dispatch(&mut app, Message::SelectSession(0));
    credentials(&mut app);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        (
            new_id,
            PaneEndpoint::profile(&host),
            "alpha.invalid:/work/colon:/kept".into(),
            Some("generated split secret".into())
        )
    );
    assert_eq!(
        app.sessions[source].panes[1].endpoint,
        PaneEndpoint::profile(&host)
    );
    assert!(
        app.sessions[source].panes[1]
            .transcript
            .iter()
            .any(|row| row.text.contains("SSH connection failed"))
    );
    let body = snapshot(&app).persist_serialize().unwrap();
    for secret in ["generated split secret", "not endpoint metadata"] {
        assert!(!body.contains(secret));
        assert!(!body.contains(&hex(secret.as_bytes())));
        assert!(!body.contains(&hex(hex(secret.as_bytes()).as_bytes())));
    }
    assert_eq!(
        PaneEndpoint::profile(&host).remote_directory("2001:db8::1:/work/colon:/kept", "/fallback"),
        "/work/colon:/kept"
    );
}

#[test]
fn two_panes_on_one_profile_have_independent_credential_generations() {
    let (mut app, store, calls) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    add_profile(&mut app, &host);
    dispatch(&mut app, Message::NewPane);
    let first = app.active_pane_id().unwrap();
    dispatch(&mut app, Message::NewPaneDown);
    let second = app.active_pane_id().unwrap();
    assert_eq!(store.get_call_count(), 2);
    assert_eq!(app.pending_pane_connections.len(), 2);
    assert_ne!(
        app.pending_pane_connections[0].origin.generation,
        app.pending_pane_connections[1].origin.generation
    );
    let mut futures = std::mem::take(&mut app.credential_futures);
    let late = complete(futures.pop().unwrap());
    dispatch(&mut app, Message::ClosePane);
    assert_eq!(app.pending_pane_connections.len(), 1);
    let before = snapshot(&app);
    dispatch(&mut app, late);
    assert_eq!(snapshot(&app), before);
    assert!(calls.lock().unwrap().is_empty());
    dispatch(&mut app, complete(futures.pop().unwrap()));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, first);
    assert_ne!(calls[0].0, second);
}

#[test]
fn cancelling_a_split_after_retrieval_cannot_launch_or_retarget() {
    let (mut app, store, calls) = fixture();
    add_profile(&mut app, &profile("alpha", SshAuthMode::Password));
    assert_eq!(
        complete(store.set(
            crate::ssh_credentials::CredentialSlot::Password.key("alpha"),
            Secret::from_string("generated cancelled secret".into())
        )),
        CredentialOutcome::Stored
    );
    dispatch(&mut app, Message::NewPane);
    let future = app.credential_futures.pop().unwrap();
    let late = complete(future);
    assert!(matches!(
        late,
        Message::SshCredentialLoaded(_, _, _, _, SshCredentialResult::Retrieved(_))
    ));
    dispatch(&mut app, Message::CancelSshCredentialOps);
    assert!(app.pending_pane_connections.is_empty());
    let before = snapshot(&app);
    let task = app.update(late);
    assert!(iced_test::runtime::task::into_stream(task).is_none());
    assert_eq!(snapshot(&app), before);
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        app.active_pane()
            .unwrap()
            .transcript
            .iter()
            .any(|row| row.text.contains("cancelled"))
    );
    dispatch(&mut app, Message::NewPane);
    credentials(&mut app);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[test]
fn mixed_group_restart_and_ungroup_use_each_panes_own_profile() {
    let (mut app, _, _) = fixture();
    let local = app.sessions[0].id.clone();
    let alpha = profile("alpha", SshAuthMode::Agent);
    let bravo = profile("bravo", SshAuthMode::Password);
    let a = add_profile(&mut app, &alpha);
    let b = add_profile(&mut app, &bravo);
    app.active = 0;
    dispatch(
        &mut app,
        Message::ContextRowActivated(a, context_menu::RowKind::GroupInto(GroupEdge::Right)),
    );
    dispatch(
        &mut app,
        Message::ContextRowActivated(b, context_menu::RowKind::GroupInto(GroupEdge::Down)),
    );
    let saved = roundtrip(&app);
    assert_eq!(saved.sessions.len(), 1);
    let (mut restored, store, calls) = fixture();
    restored.ssh_hosts = vec![bravo.clone(), alpha.clone()];
    assert!(restored.install_restored_sessions(&saved, true));
    assert!(calls.lock().unwrap().is_empty());
    open_main_and_reconnect(&mut restored);
    assert_eq!(calls.lock().unwrap()[0].1, PaneEndpoint::Local);
    assert_eq!(store.get_call_count(), 1);
    credentials(&mut restored);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    for pane in &saved.sessions[0].panes {
        let call = calls.iter().find(|call| call.0 == pane.id).unwrap();
        assert_eq!(call.1, pane.endpoint);
        assert_eq!(call.2, pane.cwd);
    }
    drop(calls);
    dispatch(&mut restored, Message::UngroupSession(local, 1));
    let session = &restored.sessions[restored.active];
    assert_eq!(session.panes[0].endpoint, PaneEndpoint::profile(&bravo));
    assert!(roundtrip(&restored).is_sound());
}

#[test]
fn splitting_a_group_member_follows_that_member_on_ungroup() {
    let (mut app, _, calls) = fixture();
    let local = app.sessions[0].id.clone();
    let host = profile("alpha", SshAuthMode::Agent);
    let remote = add_profile(&mut app, &host);
    app.active = 0;
    dispatch(
        &mut app,
        Message::ContextRowActivated(remote, context_menu::RowKind::GroupInto(GroupEdge::Right)),
    );
    let remote_pane = app.sessions[0].panes[1].id.clone();
    dispatch(&mut app, Message::FocusPane(remote_pane));
    dispatch(&mut app, Message::NewPaneDown);
    let new_id = app.active_pane_id().unwrap();
    assert_eq!(calls.lock().unwrap()[0].1, PaneEndpoint::profile(&host));
    assert!(roundtrip(&app).is_sound());
    dispatch(&mut app, Message::UngroupSession(local, 0));
    let session = &app.sessions[app.active];
    assert_eq!(session.panes.len(), 2);
    assert!(session.panes.iter().any(|pane| pane.id == new_id));
    assert!(
        session
            .panes
            .iter()
            .all(|pane| pane.endpoint == PaneEndpoint::profile(&host))
    );
}

#[test]
fn edited_or_deleted_profiles_remain_disconnected_without_reading_secrets() {
    let (mut source, _, _) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    add_profile(&mut source, &host);
    let saved = roundtrip(&source);
    for edit in ["delete", "host", "port", "user", "auth", "cwd"] {
        let (mut restored, store, calls) = fixture();
        let mut changed = host.clone();
        match edit {
            "host" => changed.destination = "different.invalid".into(),
            "port" => changed.port += 1,
            "user" => changed.username = "different".into(),
            "auth" => changed.auth = SshAuthMode::Agent,
            "cwd" => changed.path = "/different".into(),
            _ => {}
        }
        if edit != "delete" {
            restored.ssh_hosts.push(changed);
        }
        restored
            .ssh_hosts
            .push(profile("unrelated", SshAuthMode::Password));
        assert!(restored.install_restored_sessions(&saved, true));
        open_main_and_reconnect(&mut restored);
        assert_eq!(store.get_call_count(), 0, "{edit}");
        assert!(
            calls
                .lock()
                .unwrap()
                .iter()
                .all(|call| call.1 == PaneEndpoint::Local),
            "{edit}"
        );
        let pane = &restored.sessions[1].panes[0];
        assert_eq!(pane.endpoint, PaneEndpoint::profile(&host));
        assert_eq!(pane.status, SessionStatus::Error);
        assert!(
            pane.transcript
                .iter()
                .any(|row| row.text.contains("deleted") || row.text.contains("changed"))
        );
    }
}

#[test]
fn profile_changes_while_split_credentials_wait_retire_the_request() {
    let (mut app, _, calls) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    add_profile(&mut app, &host);
    dispatch(&mut app, Message::NewPane);
    let late = complete(app.credential_futures.pop().unwrap());
    // A profile cannot be edited while its read is live, so the user cancels
    // the slow keyring wait first; `late` is then a superseded completion.
    dispatch(&mut app, Message::CancelSshCredentialOps);
    dispatch(&mut app, Message::OpenSshManager);
    dispatch(&mut app, Message::OpenSshEditForm(0));
    dispatch(
        &mut app,
        Message::SshFormDestEdited("different.invalid".into()),
    );
    dispatch(&mut app, Message::SaveSshConnection);
    assert_eq!(
        app.ssh_hosts[0].destination, "different.invalid",
        "the edit must be accepted: {:?}",
        app.ssh_save_error
    );
    assert!(app.pending_pane_connections.is_empty());
    let before = snapshot(&app);
    let task = app.update(late);
    assert!(iced_test::runtime::task::into_stream(task).is_none());
    assert_eq!(snapshot(&app), before);
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        app.active_pane().unwrap().endpoint,
        PaneEndpoint::profile(&host)
    );
}

#[test]
fn legacy_mixed_workspace_migrates_local_but_never_guesses_an_ssh_snapshot() {
    let (mut app, _, _) = fixture();
    let host = profile("alpha", SshAuthMode::Agent);
    let remote = add_profile(&mut app, &host);
    app.active = 0;
    dispatch(
        &mut app,
        Message::ContextRowActivated(remote, context_menu::RowKind::GroupInto(GroupEdge::Right)),
    );
    let body = snapshot(&app).persist_serialize().unwrap();
    let old = body
        .lines()
        .map(|line| {
            if line.starts_with("pane\t") {
                line.split('\t').take(5).collect::<Vec<_>>().join("\t")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let migrated = SessionState::persist_deserialize(&old).unwrap();
    assert_eq!(migrated.sessions[0].panes[0].endpoint, PaneEndpoint::Local);
    assert_eq!(
        migrated.sessions[0].panes[1].endpoint,
        PaneEndpoint::LegacySsh(Some(host.id.clone()))
    );
    let (mut restored, store, calls) = fixture();
    restored.ssh_hosts.push(host);
    assert!(restored.install_restored_sessions(&migrated, true));
    open_main_and_reconnect(&mut restored);
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert_eq!(store.get_call_count(), 0);
    assert!(
        restored.sessions[0].panes[1]
            .transcript
            .iter()
            .any(|row| row.text.contains("cannot be inferred safely"))
    );
    assert_eq!(
        SessionState::persist_deserialize(&migrated.persist_serialize().unwrap()).unwrap(),
        migrated
    );
}

#[test]
fn malformed_endpoint_metadata_cannot_fall_back_to_local_or_agent() {
    let (mut app, _, _) = fixture();
    add_profile(
        &mut app,
        &profile(
            "alpha",
            SshAuthMode::KeyFile {
                key_path: "/generated/key".into(),
            },
        ),
    );
    let original = snapshot(&app).persist_serialize().unwrap();
    for endpoint in [
        "unknown-source",
        "profile-1\tzz",
        "openssh-1\t61\t62\t75\t22\tzz\t1",
    ] {
        let bad = original
            .lines()
            .map(|line| {
                if line.starts_with("pane\t") {
                    let mut fields = line
                        .split('\t')
                        .take(5)
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    fields.push(hex(endpoint.as_bytes()));
                    fields.join("\t")
                } else {
                    line.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(SessionState::persist_deserialize(&bad).is_err());
    }
    assert!(
        SessionState::persist_deserialize(&original)
            .unwrap()
            .is_sound()
    );
}

#[test]
fn detached_split_targets_its_window_and_retains_endpoint_after_restart() {
    let (mut app, _, calls) = fixture();
    let host = profile("alpha", SshAuthMode::Agent);
    let session = add_profile(&mut app, &host);
    let pane = app.active_pane_id().unwrap();
    let window = iced::window::Id::unique();
    app.sessions[1].detached = true;
    dispatch(
        &mut app,
        Message::DetachedWindowOpened {
            session: session.clone(),
            window,
        },
    );
    dispatch(&mut app, Message::SelectSession(0));
    dispatch(
        &mut app,
        Message::DetachedSplitPane {
            window,
            pane,
            direction: SplitDirection::Vertical,
        },
    );
    assert_eq!(app.active, 0);
    assert_eq!(app.sessions[0].panes.len(), 1);
    assert_eq!(app.sessions[1].panes.len(), 2);
    assert_eq!(calls.lock().unwrap()[0].1, PaneEndpoint::profile(&host));
    let saved = roundtrip(&app);
    assert!(saved.sessions[1].detached);
    let (mut restored, _, restored_calls) = fixture();
    restored.ssh_hosts.push(host.clone());
    assert!(restored.install_restored_sessions(&saved, true));
    open_main_and_reconnect(&mut restored);
    assert!(restored.detached_window_of(&session).is_some());
    assert_eq!(
        restored_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.1 == PaneEndpoint::profile(&host))
            .count(),
        2
    );
}

#[path = "endpoint_tests/routing_tests.rs"]
mod routing_tests;
