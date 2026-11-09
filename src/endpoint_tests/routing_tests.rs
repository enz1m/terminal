//! Endpoint aliasing, cwd identity, and restore routing.

use super::*;

struct ConfigFixture(std::path::PathBuf);
impl ConfigFixture {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".endpoint-fixture-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        std::fs::create_dir(root.join(".ssh")).unwrap();
        Self(root)
    }
    fn load(&self, hostname: &str, proxy: &str) -> Vec<crate::ssh_config::OpenSshHost> {
        std::fs::write(self.0.join(".ssh/config"), "Include team.conf\n").unwrap();
        std::fs::write(self.0.join(".ssh/team.conf"), format!("Host team\n HostName {hostname}\n User deploy\n Port 2202\n IdentityFile ~/.ssh/team-key\n ProxyJump {proxy}\n")).unwrap();
        crate::ssh_config::load_user_config(&self.0.join(".ssh/config"), &self.0).unwrap()
    }
}
impl Drop for ConfigFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn openssh_alias_restarts_from_config_and_rejects_changed_includes() {
    let config = ConfigFixture::new();
    let aliases = config.load("alpha.invalid", "jump-a");
    let endpoint = PaneEndpoint::OpenSsh(aliases[0].clone());
    let (mut source, _, _) = fixture();
    source.openssh_hosts = aliases.clone();
    source.ssh_spawn_override = Some(Box::new(|_, _, _| {
        Err(pty::PtyError::Spawn("alias fixture".into()))
    }));
    let generation = source.openssh_discovery.generation();
    dispatch(&mut source, Message::ConnectOpenSshHost(0, generation));
    assert_eq!(source.active_pane().unwrap().endpoint, endpoint);
    let saved = roundtrip(&source);
    for case in ["unchanged", "hostname", "proxy", "deleted"] {
        let current = match case {
            "hostname" => config.load("different.invalid", "jump-a"),
            "proxy" => config.load("alpha.invalid", "jump-b"),
            "deleted" => Vec::new(),
            _ => aliases.clone(),
        };
        let (mut restored, store, calls) = fixture();
        dispatch(&mut restored, Message::OpenSshConfigLoaded(Ok(current)));
        assert!(restored.install_restored_sessions(&saved, true));
        open_main_and_reconnect(&mut restored);
        assert_eq!(
            store.get_call_count(),
            0,
            "OpenSSH does not borrow another profile's secret"
        );
        let remote_count = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.1 == endpoint)
            .count();
        assert_eq!(remote_count, usize::from(case == "unchanged"), "{case}");
        assert_eq!(restored.sessions[1].panes[0].endpoint, endpoint);
    }
}

#[test]
fn ssh_alias_command_keeps_alias_options_but_pins_the_selected_hostname() {
    let config = ConfigFixture::new();
    let aliases = config.load("alpha.invalid", "jump-a");
    let endpoint = PaneEndpoint::OpenSsh(aliases[0].clone());
    let mut host = endpoint.resolve(&[], &aliases).unwrap();
    host.path = endpoint.remote_directory("alpha.invalid:/srv/a ' quoted", "");
    let (mut cmd, channel) =
        pty::build_ssh_command_with(&host, None, |_| Some("/usr/bin/ssh".into())).unwrap();
    assert!(channel.is_none());
    pty::pin_ssh_hostname(&mut cmd, &aliases[0].hostname).unwrap();
    let argv = cmd
        .get_argv()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let boundary = argv.iter().position(|arg| arg == "--").unwrap();
    assert_eq!(argv[boundary + 1], "team");
    assert!(
        argv[..boundary]
            .windows(2)
            .any(|pair| pair == ["-o", "HostName=alpha.invalid"])
    );
    assert!(argv.iter().any(|arg| arg.contains("/srv/a")));
    assert!(argv.iter().any(|arg| arg == "2202"));
    assert!(argv.iter().any(|arg| arg == "~/.ssh/team-key"));
    // Pinning the hostname splices into the same option run that carries the
    // host-key policy, and the pinned host is the one whose key is confirmed.
    assert_eq!(
        argv[..boundary]
            .windows(2)
            .filter(|pair| pair[0] == "-o" && pair[1].starts_with("StrictHostKeyChecking="))
            .map(|pair| pair[1].as_str())
            .collect::<Vec<_>>(),
        ["StrictHostKeyChecking=ask"],
        "the pinned alias must still require host-key confirmation; argv={argv:?}"
    );
}

#[test]
fn cwd_updates_and_new_local_tabs_use_pane_identity_not_group_chrome() {
    let (mut app, _, _) = fixture();
    let host = profile("alpha", SshAuthMode::Agent);
    let remote = add_profile(&mut app, &host);
    app.active = 0;
    dispatch(
        &mut app,
        Message::ContextRowActivated(remote, context_menu::RowKind::GroupInto(GroupEdge::Right)),
    );
    app.apply_terminal_effects(
        0,
        1,
        crate::emulator::TerminalEffects {
            cwd: Some("file://localhost/remote/work".into()),
            ..Default::default()
        },
    );
    app.apply_terminal_effects(
        0,
        0,
        crate::emulator::TerminalEffects {
            cwd: Some("file://localhost/local/work".into()),
            ..Default::default()
        },
    );
    assert_eq!(app.sessions[0].panes[1].cwd, "alpha.invalid:/remote/work");
    assert_eq!(app.sessions[0].panes[0].cwd, "/local/work");
    app.sessions[0].active_pane = 1;
    assert_eq!(app.active_local_cwd(), None);
    app.sessions[0].active_pane = 0;
    assert_eq!(app.active_local_cwd().as_deref(), Some("/local/work"));
}

#[test]
fn profile_connection_captures_snapshot_but_metadata_renames_do_not_retarget_it() {
    let (mut app, store, _) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    app.ssh_hosts.push(host.clone());
    assert_eq!(
        complete(store.set(
            crate::ssh_credentials::CredentialSlot::Password.key("alpha"),
            Secret::from_string("generated connection secret".into())
        )),
        CredentialOutcome::Stored
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    app.ssh_spawn_override = Some(Box::new(move |_, host, password| {
        capture
            .lock()
            .unwrap()
            .push((host.clone(), password.map(str::to_owned)));
        Err(pty::PtyError::Spawn("generated refusal".into()))
    }));
    dispatch(&mut app, Message::ConnectSshHost(0));
    credentials(&mut app);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        seen.lock().unwrap()[0].1.as_deref(),
        Some("generated connection secret")
    );
    assert_eq!(
        app.active_pane().unwrap().endpoint,
        PaneEndpoint::profile(&host)
    );
    app.ssh_hosts[0].name = "renamed label".into();
    app.ssh_hosts[0].notes = "edited notes".into();
    assert_eq!(
        app.active_pane()
            .unwrap()
            .endpoint
            .resolve(&app.ssh_hosts, &[])
            .unwrap()
            .destination,
        host.destination
    );
    let mut defaults = profile("default-port", SshAuthMode::Agent);
    defaults.port = 22;
    let (command, _) =
        pty::build_ssh_command_with(&defaults, None, |_| Some("/usr/bin/ssh".into())).unwrap();
    assert!(command.get_argv().iter().any(|arg| arg == "Port=22"));
}

#[cfg(target_os = "linux")]
#[test]
fn restored_endpoints_install_distinct_live_ptys_and_route_input_to_the_selected_pane() {
    use std::os::unix::fs::DirBuilderExt;
    use std::time::{Duration, Instant};
    let root = ConfigFixture::new();
    let (mut source, _, _) = fixture();
    let host = profile("alpha", SshAuthMode::Agent);
    add_profile(&mut source, &host);
    let saved = roundtrip(&source);
    let (mut restored, _, _) = fixture();
    restored.ssh_hosts.push(host.clone());
    let base = root.0.clone();
    let launches = Arc::new(Mutex::new(Vec::new()));
    let observed = launches.clone();
    restored.endpoint_spawn_override = Some(Box::new(move |id, endpoint, cwd, password| {
        assert!(password.is_none());
        observed
            .lock()
            .unwrap()
            .push((id.to_owned(), endpoint.clone(), cwd.to_owned()));
        let path = base.join(id);
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
        pty::PtySession::spawn_history_probe(id, &path)
    }));
    assert!(restored.install_restored_sessions(&saved, true));
    open_main_and_reconnect(&mut restored);
    assert_eq!(restored.ptys.len(), 2);
    assert_eq!(launches.lock().unwrap().len(), 2);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        dispatch(&mut restored, Message::Poll);
        if restored
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
            "private PTYs did not reach their prompts"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    dispatch(&mut restored, Message::SelectSession(1));
    restored.sessions[1].panes[0].bracketed_paste = false;
    let target = restored.sessions[1].panes[0].id.clone();
    let other = restored.sessions[0].panes[0].id.clone();
    let command = "printf endpoint-ok > endpoint-result\n";
    let reply = crate::operation_origin::tests::clipboard_reply(&mut restored, Some(command));
    dispatch(&mut restored, reply);
    assert_eq!(
        restored.ptys[&target].take_recorded_input(),
        command.as_bytes()
    );
    assert!(restored.ptys[&other].take_recorded_input().is_empty());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if std::fs::read(root.0.join(&target).join("endpoint-result"))
            .ok()
            .as_deref()
            == Some(b"endpoint-ok".as_slice())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "selected endpoint's private process received no command"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!root.0.join(&other).join("endpoint-result").exists());
    assert_eq!(
        restored.sessions[1].panes[0].endpoint,
        PaneEndpoint::profile(&host)
    );
}

#[test]
fn restoration_waits_for_both_credential_backend_and_window_ownership() {
    let (mut source, _, _) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    add_profile(&mut source, &host);
    let saved = roundtrip(&source);
    for window_first in [false, true] {
        let (mut restored, store, calls) = fixture();
        restored.ssh_hosts.push(host.clone());
        restored.endpoint_credentials_ready = false;
        assert!(restored.install_restored_sessions(&saved, true));
        if window_first {
            open_main_and_reconnect(&mut restored);
        } else {
            dispatch(
                &mut restored,
                Message::CredentialBackendReady(CredentialBackend::new(store.clone())),
            );
        }
        assert_eq!(store.get_call_count(), 0);
        assert_eq!(calls.lock().unwrap().len(), 0);
        if window_first {
            dispatch(
                &mut restored,
                Message::CredentialBackendReady(CredentialBackend::new(store.clone())),
            );
        } else {
            open_main_and_reconnect(&mut restored);
        }
        if window_first {
            crate::pinned_sessions::tests::reconnect_all(&mut restored);
        }
        assert_eq!(store.get_call_count(), 1);
        credentials(&mut restored);
        assert_eq!(calls.lock().unwrap().len(), 2);
    }
}

fn refuse_host_key(app: &mut App, session: &str, pane: &str, code: i32, secret_delivery: bool) {
    app.note_host_trust_exit(
        session,
        pane,
        &pty::ChildExit {
            detail: format!("exit code {code}"),
            code: Some(code),
        },
        secret_delivery,
    );
}

fn labels(nodes: &[enzui_core::collect::CollectedNode]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| node.semantics.label.clone())
        .collect()
}

// The keyring holds a secret throughout, so a re-read or a re-send is
// observable as a store read or a non-`None` password argument.
#[test]
fn an_unconfirmed_host_key_earns_endpoint_bound_host_trust_recovery_with_no_secret() {
    let (mut app, store, calls) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    add_profile(&mut app, &host);
    assert_eq!(
        complete(store.set(
            crate::ssh_credentials::CredentialSlot::Password.key(&host.id),
            Secret::from_string("generated host trust secret".into())
        )),
        CredentialOutcome::Stored
    );
    dispatch(
        &mut app,
        Message::MainWindowOpened(iced::window::Id::unique()),
    );
    let session = app.sessions[app.active].id.clone();
    let pane = app.sessions[app.active].panes[0].id.clone();
    let cwd = app.sessions[app.active].panes[0].cwd.clone();
    calls.lock().unwrap().clear();
    let reads = store.get_call_count();

    refuse_host_key(&mut app, &session, &pane, 5, true);
    refuse_host_key(&mut app, &session, &pane, 6, false);
    assert!(
        app.host_trust.is_empty(),
        "only status 6 from a secret-delivering child may earn a trust offer"
    );

    refuse_host_key(&mut app, &session, &pane, 6, true);
    dispatch(&mut app, Message::Poll);
    assert_eq!(app.host_trust.len(), 1);

    let size = iced::Size::new(1100.0, 800.0);
    app.content_size = size;
    let nodes = crate::review::probe::nodes_of(app.single_terminal_pane(), size);
    let rendered = labels(&nodes);
    let announcement = rendered
        .iter()
        .find(|label| {
            label.starts_with(&format!(
                "Error: {}.",
                crate::host_trust::HOST_KEY_UNCONFIRMED
            ))
        })
        .unwrap_or_else(|| panic!("no host-trust announcement: {rendered:?}"));
    assert!(
        announcement.contains("user-alpha@alpha.invalid:2222"),
        "the prose must name the exact refused endpoint: {announcement}"
    );
    assert!(
        rendered
            .iter()
            .all(|label| !label.contains("generated host trust secret")),
        "no rendered prose may carry the secret: {rendered:?}"
    );
    let button = nodes
        .iter()
        .find(|node| node.semantics.label.as_deref() == Some("Confirm host key"))
        .unwrap_or_else(|| panic!("no confirmation action: {rendered:?}"));
    assert!(button.bounds.width > 0.0 && button.bounds.height > 0.0);

    let messages =
        crate::review::probe::click_at(app.single_terminal_pane(), size, button.bounds.center());
    assert!(
        !messages.is_empty(),
        "the confirmation action published nothing"
    );
    for message in messages {
        dispatch(&mut app, message);
    }

    let launched = calls.lock().unwrap();
    assert_eq!(
        launched.len(),
        1,
        "the confirmation must launch exactly one session: {launched:?}"
    );
    assert_eq!(
        launched[0],
        (pane, PaneEndpoint::profile(&host), cwd, None),
        "the same endpoint snapshot must be launched with no password"
    );
    drop(launched);
    assert_eq!(
        store.get_call_count(),
        reads,
        "confirming host trust must read no secret"
    );
    assert!(app.host_trust.is_empty(), "a spent offer is consumed");
}

#[test]
fn a_reconfigured_profile_suppresses_a_stale_host_trust_offer() {
    let (mut app, store, calls) = fixture();
    let host = profile("alpha", SshAuthMode::Password);
    add_profile(&mut app, &host);
    dispatch(
        &mut app,
        Message::MainWindowOpened(iced::window::Id::unique()),
    );
    let session = app.sessions[app.active].id.clone();
    let pane = app.sessions[app.active].panes[0].id.clone();
    refuse_host_key(&mut app, &session, &pane, 6, true);
    dispatch(&mut app, Message::Poll);

    let size = iced::Size::new(1100.0, 800.0);
    app.content_size = size;
    let nodes = crate::review::probe::nodes_of(app.single_terminal_pane(), size);
    let button = nodes
        .iter()
        .find(|node| node.semantics.label.as_deref() == Some("Confirm host key"))
        .unwrap_or_else(|| {
            panic!(
                "positive control: the offer must render its action before the profile changes: {:?}",
                labels(&nodes)
            )
        });
    let stale =
        crate::review::probe::click_at(app.single_terminal_pane(), size, button.bounds.center());
    assert!(!stale.is_empty());

    calls.lock().unwrap().clear();
    let reads = store.get_call_count();
    dispatch(&mut app, Message::OpenSshManager);
    dispatch(&mut app, Message::OpenSshEditForm(0));
    dispatch(
        &mut app,
        Message::SshFormDestEdited("different.invalid".into()),
    );
    dispatch(&mut app, Message::SaveSshConnection);
    assert!(
        app.host_trust.is_empty(),
        "a retargeted profile must retire its host-trust offer"
    );

    let rendered = labels(&crate::review::probe::nodes_of(
        app.single_terminal_pane(),
        size,
    ));
    assert!(
        rendered
            .iter()
            .all(|label| !label.contains(crate::host_trust::HOST_KEY_UNCONFIRMED)),
        "a retired offer must render no host-trust prose: {rendered:?}"
    );
    for message in stale {
        dispatch(&mut app, message);
    }
    assert!(
        calls.lock().unwrap().is_empty(),
        "a stale confirmation must launch nothing"
    );
    assert_eq!(store.get_call_count(), reads);
}
