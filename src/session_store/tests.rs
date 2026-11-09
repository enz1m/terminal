use super::*;
use crate::model::{SessionStatus, SshAuthMode, SshHostRecord};
use crate::pane_group::{GroupEdge, group_into, ungroup};
use crate::{App, Message, failed_pane, scripted_pane, settings_store};
use enzui_core::persist::{InMemoryBackend, LoadOutcome};
use iced_test::futures::futures::StreamExt;

fn run_outputs(task: iced::Task<Message>) -> Vec<Message> {
    let Some(mut stream) = iced_test::runtime::task::into_stream(task) else {
        return Vec::new();
    };
    iced_test::futures::futures::executor::block_on(async move {
        let mut out = Vec::new();
        while let Some(action) = stream.next().await {
            match action {
                iced_test::runtime::Action::Output(message) => out.push(message),
                iced_test::runtime::Action::Window(iced_test::runtime::window::Action::Open(
                    id,
                    _,
                    sender,
                )) => {
                    // The headless task runner is the compositor for this
                    // test: acknowledge the requested id so the mapped
                    // `DetachedWindowOpened` output can finish.
                    let _ = sender.send(id);
                }
                _ => {}
            }
        }
        out
    })
}

fn app_with_state_backend(backend: InMemoryBackend) -> App {
    let settings = Store::new(
        InMemoryBackend::new(),
        settings_store::FORMAT_ID,
        settings_store::VERSION,
    );
    let state = Store::new(backend, FORMAT_ID, VERSION);
    App::new_with_stores_and_spawn(false, settings, state, false, |_, _| {
        Err(crate::pty::PtyError::Spawn(
            "isolated metadata fixture".into(),
        ))
    })
    .0
}

fn session_fixture() -> Vec<Session> {
    let mut layout = PaneLayout::leaf("pane-a");
    assert!(layout.split_leaf("pane-a", SplitDirection::Vertical, "pane-b"));
    vec![
        Session {
            id: "local-tab".into(),
            title: "Work & logs".into(),
            subtitle: "bash".into(),
            icon: "terminal".into(),
            status: SessionStatus::Idle,
            kind: SessionKind::Local,
            ssh_host_id: None,
            connection_error: None,
            pinned: true,
            detached: false,
            panes: vec![
                scripted_pane("pane-a", "ignored transcript", "/srv/project"),
                failed_pane("pane-b", "ignored failure"),
            ],
            active_pane: 1,
            pane_grid: layout.pane_grid_state(),
            layout,
            in_app_history: crate::model::CommandHistory::new(),
            group_members: Vec::new(),
        },
        Session {
            id: "ssh-tab".into(),
            title: "Production".into(),
            subtitle: "deploy@example.test".into(),
            icon: "server".into(),
            status: SessionStatus::Error,
            kind: SessionKind::Ssh,
            ssh_host_id: Some("host-1".into()),
            connection_error: Some("not persisted".into()),
            pinned: true,
            detached: true,
            panes: vec![scripted_pane("pane-c", "ignored", "example.test:/opt/app")],
            active_pane: 0,
            pane_grid: PaneLayout::leaf("pane-c").pane_grid_state(),
            layout: PaneLayout::leaf("pane-c"),
            in_app_history: crate::model::CommandHistory::new(),
            group_members: Vec::new(),
        },
    ]
}

#[test]
fn save_restore_round_trip_preserves_tabs_panes_cwds_and_layout() {
    let backend = InMemoryBackend::new();
    let writer = Store::new(backend.clone(), FORMAT_ID, VERSION);
    let expected = SessionState::snapshot(&session_fixture(), 1, &HashSet::new());
    writer.prepare_save(&expected).unwrap().execute().unwrap();

    let restarted = Store::<SessionState>::new(backend, FORMAT_ID, VERSION).load();
    assert!(matches!(&restarted, LoadOutcome::Loaded(_)));
    let restarted = restarted.into_value();
    assert_eq!(restarted, expected);
    assert_eq!(restarted.active_session_id.as_deref(), Some("ssh-tab"));
    assert!(restarted.sessions[0].panes[0].cwd.is_empty());
    assert_eq!(restarted.sessions[0].active_pane_id, "pane-b");
    assert!(matches!(
        restarted.sessions[0].layout,
        PaneLayout::Split { .. }
    ));

    // Positive control: secrets are not part of this format's type at all.
    let host = SshHostRecord {
        id: "host-1".into(),
        name: "Production".into(),
        destination: "example.test".into(),
        port: 22,
        username: "deploy".into(),
        path: "/opt/app".into(),
        auth: SshAuthMode::Password,
        notes: String::new(),
    };
    assert_eq!(host.auth, SshAuthMode::Password);
}

/// Group identity and the source's exact nested topology survive the store
/// and a fresh App, so restart does not erase the visible ungroup action.
/// Sabotage: omit `group_members` from snapshot or restore; the restarted
/// group has no member and this gate fails before ungrouping.
#[test]
fn grouped_workspace_round_trip_retains_ungroup_identity_and_topology() {
    let backend = InMemoryBackend::new();
    let mut sessions = session_fixture();
    let expected_layout = sessions[0].layout.clone();
    let expected_active = sessions[0].panes[sessions[0].active_pane].id.clone();
    let group_idx =
        group_into(&mut sessions, "local-tab", "ssh-tab", GroupEdge::Left).expect("group fixture");
    let snapshot = SessionState::snapshot(&sessions, group_idx, &HashSet::new());
    Store::new(backend.clone(), FORMAT_ID, VERSION)
        .prepare_save(&snapshot)
        .unwrap()
        .execute()
        .unwrap();

    let mut restarted = app_with_state_backend(backend);
    assert_eq!(restarted.sessions.len(), 1);
    assert_eq!(restarted.sessions[0].group_members.len(), 1);
    assert_eq!(
        restarted.sessions[0].group_members[0].session_id,
        "local-tab"
    );
    restarted.content_size = iced::Size::new(1400.0, 900.0);
    let nodes = crate::review::probe::nodes_of(restarted.view(), restarted.content_size);
    assert!(
        nodes.iter().any(|node| {
            node.semantics.label.as_deref() == Some("Ungroup Work & logs from Production")
        }),
        "restart must retain the rendered, routable ungroup affordance"
    );
    let restored_idx = ungroup(&mut restarted.sessions, 0, 0).expect("ungroup after restart");
    let restored = &restarted.sessions[restored_idx];
    assert_eq!(restored.layout, expected_layout);
    assert_eq!(restored.panes[restored.active_pane].id, expected_active);
    assert!(restarted.sessions[0].group_members.is_empty());
}

#[test]
fn contradictory_layout_is_rejected_instead_of_spawning_partial_state() {
    let mut state = SessionState::snapshot(&session_fixture(), 0, &HashSet::new());
    let valid_body = state.persist_serialize().unwrap();
    let old_layout = hex(encode_layout(&state.sessions[0].layout).as_bytes());
    state.sessions[0].layout = PaneLayout::leaf("missing-pane");
    assert!(!state.is_sound());
    assert!(state.persist_serialize().is_err());
    let body = valid_body.replace(
        &old_layout,
        &hex(encode_layout(&state.sessions[0].layout).as_bytes()),
    );
    assert!(SessionState::persist_deserialize(&body).is_err());
}

/// App adoption gate: a durable model change travels through the real
/// update post-pass, the async Store task, and a fresh App constructor.
/// Removing either the queue in `App::update` or the restore branch in
/// `App::new_with_stores` leaves the restarted app at its demo seed and
/// reddens this test.
#[test]
fn app_update_saves_and_next_process_restores_the_workspace() {
    let state_backend = InMemoryBackend::new();
    let settings = Store::new(
        InMemoryBackend::new(),
        settings_store::FORMAT_ID,
        settings_store::VERSION,
    );
    let state = Store::new(state_backend.clone(), FORMAT_ID, VERSION);
    let (mut app, _) = App::new_with_stores_and_spawn(false, settings, state, false, |_, _| {
        Err(crate::pty::PtyError::Spawn(
            "isolated metadata fixture".into(),
        ))
    });
    app.sessions[0].pinned = true;

    let first_id = app.sessions[0].panes[0].id.clone();
    let extra_id = "restart-extra-pane".to_owned();
    assert!(app.sessions[0].layout.split_leaf(
        &first_id,
        SplitDirection::Vertical,
        extra_id.clone()
    ));
    app.sessions[0].panes.push(scripted_pane(
        &extra_id,
        "not persisted",
        "/work/second-pane",
    ));
    app.sessions[0].active_pane = 1;
    app.sessions[0].title = "Restored workspace".into();
    app.title_overrides.insert(app.sessions[0].id.clone());
    app.active = 0;

    // Drive a real durable reducer route. Its post-pass captures the whole
    // workspace, including the second pane and cwd prepared above.
    app.pane_rename_draft = Some("older-in-flight-title".into());
    let first_write = app.update(Message::CommitPaneRename);

    // Change the same durable field before the first write is polled. The
    // second update must not race it; completion of the older write starts
    // the queued newest snapshot.
    app.pane_rename_draft = Some("restored-extra".into());
    let deferred = app.update(Message::CommitPaneRename);
    assert!(
        run_outputs(deferred).is_empty(),
        "a newer snapshot waits behind the in-flight writer"
    );
    let expected = SessionState::snapshot(&app.sessions, app.active, &app.title_overrides);
    let completions = run_outputs(first_write);
    assert_eq!(completions.len(), 1, "the first serialized save must run");
    let mut followups = Vec::new();
    for completion in completions {
        followups.extend(run_outputs(app.update(completion)));
    }
    assert_eq!(
        followups.len(),
        1,
        "completion must start the queued latest save"
    );
    for completion in followups {
        let _ = app.update(completion);
    }
    assert!(
        state_backend.peek(FORMAT_ID).is_some(),
        "positive control: update wrote the XDG-state backend"
    );

    let settings = Store::new(
        InMemoryBackend::new(),
        settings_store::FORMAT_ID,
        settings_store::VERSION,
    );
    let state = Store::new(state_backend, FORMAT_ID, VERSION);
    let (restarted, _) = App::new_with_stores(false, settings, state, false);
    assert_eq!(
        SessionState::snapshot(
            &restarted.sessions,
            restarted.active,
            &restarted.title_overrides,
        ),
        expected,
        "a fresh process must install the exact tabs/panes/cwds/layout it saved"
    );
}
