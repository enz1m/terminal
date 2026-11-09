fn signal(command: &str) -> Vec<u8> {
    let hex: String = command
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("\x1b]777;enzui-command;{hex}\x07").into_bytes()
}

#[cfg(target_os = "linux")]
mod native {
    use crate::model::{CommandHistory, SessionKind};
    use crate::session_store::{SavedPane, SavedSession, SessionState};
    use crate::{App, Message, PaneLayout, pty, session_store, settings_store};
    use enzui_core::collect::{CollectedNode, SemanticsCollector};
    use enzui_core::persist::{InMemoryBackend, Persistable, Store};
    use iced::advanced::renderer::Headless;
    use iced::keyboard::{Key, Modifiers, key::Named};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const SIZE: iced::Size = iced::Size::new(1400.0, 900.0);
    const ORDINARY: &str = "printf 'ordinary-output\\n'";
    const NO_ECHO_COMMAND: &str = "stty -echo; printf '\\033]2;hidden-ready\\007'; read -r -t 5 secret; stty echo; unset secret; printf '\\033]2;hidden-done\\007'";
    const CHILD_SCRIPT: &str =
        "printf '\\033]2;child-ready\\007'\nread -r -t 5 value\nprintf '\\033]2;child-done\\007'\n";
    const CHILD_COMMAND: &str = "bash --noprofile --norc child.sh";
    const DISABLE_COMMAND: &str =
        "PROMPT_COMMAND=; PS0=; printf '\\033]2;integration-disabled\\007'";
    const UNTRACKED_COMMAND: &str = "printf '\\033]2;untracked-command-complete\\007'";

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                ".history-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            let sandbox = Self(path);
            for name in ["config", "state", "data", "cache", "runtime"] {
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(sandbox.0.join(name))
                    .unwrap();
            }
            sandbox
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        app: App,
        root: Sandbox,
    }

    impl Fixture {
        fn new() -> Self {
            let root = Sandbox::new();
            let state = SessionState {
                pending_deletes: Vec::new(),
                active_session_id: Some("history-session".into()),
                sessions: vec![SavedSession {
                    archive: None,
                    id: "history-session".into(),
                    title: "History fixture".into(),
                    subtitle: "bash".into(),
                    kind: SessionKind::Local,
                    ssh_host_id: None,
                    pinned: true,
                    detached: false,
                    title_overridden: false,
                    panes: vec![SavedPane {
                        endpoint: crate::pane_endpoint::PaneEndpoint::Local,
                        id: "history-pane".into(),
                        title: "Shell".into(),
                        cwd: String::new(),
                    }],
                    active_pane_id: "history-pane".into(),
                    layout: PaneLayout::leaf("history-pane"),
                    group_members: vec![],
                }],
            };
            assert!(state.is_sound());
            let sessions = Store::new(
                InMemoryBackend::new(),
                session_store::FORMAT_ID,
                session_store::VERSION,
            );
            sessions.prepare_save(&state).unwrap().execute().unwrap();
            let settings = Store::new(
                InMemoryBackend::new(),
                settings_store::FORMAT_ID,
                settings_store::VERSION,
            );
            let (mut app, _) = App::new_with_stores(false, settings, sessions, false);
            assert!(app.ptys.is_empty());
            app.sessions[0].in_app_history = CommandHistory::new();
            let pty = pty::PtySession::spawn_history_probe("history-pane", &root.0).unwrap();
            app.install_pty("history-pane".into(), pty);
            let _ = app.update(Message::ContentResized(SIZE));
            let mut fixture = Self { app, root };
            fixture.wait_until("initial prompt", |app| {
                app.sessions[0].panes[0]
                    .transcript
                    .iter()
                    .any(|row| row.text.contains("fixture-prompt>"))
            });
            assert!(fixture.app.sessions[0].in_app_history.is_empty());
            assert_eq!(
                fixture.app.sessions[0].title, "History fixture",
                "fixture startup must not install a system or user title hook"
            );
            fixture
        }

        fn wait_until(&mut self, checkpoint: &str, condition: impl Fn(&App) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let _ = self.app.update(Message::Poll);
                if condition(&self.app) {
                    eprintln!("PTY checkpoint passed: {checkpoint}");
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "PTY fixture deadline at {checkpoint}: {}",
                    self.diagnostics()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn diagnostics(&self) -> String {
            let session = &self.app.sessions[0];
            let pane = &session.panes[0];
            let short = |text: &str| text.chars().take(256).collect::<String>();
            let transcript: Vec<_> = pane
                .transcript
                .iter()
                .rev()
                .filter(|row| !row.text.is_empty())
                .take(8)
                .map(|row| short(&row.text))
                .collect();
            let history: Vec<_> = (0..session.in_app_history.len().min(3))
                .map(|index| {
                    let item = session.in_app_history.newest_first(index).unwrap();
                    (short(&item.command), item.exit_code)
                })
                .collect();
            let pending = self.app.ptys.get(&pane.id).map(|pty| {
                let effects = pty.take_terminal_effects();
                (
                    pty.process_id(),
                    effects.title.map(|title| short(&title)),
                    effects.command_finished,
                    effects.commands_started.len(),
                )
            });
            format!(
                "title={:?}, title_override={}, status={:?}, pane_status={:?}, pending(pid/title/exit/commands)={pending:?}, history={history:?}, transcript(newest first)={transcript:?}",
                short(&session.title),
                self.app.title_overrides.contains(&session.id),
                session.status,
                pane.status,
            )
        }

        fn type_text(&mut self, text: &str) {
            self.app.ptys["history-pane"].take_recorded_input();
            for ch in text.chars() {
                let _ = self.app.update(Message::KeyPressed {
                    key: Key::Character(ch.to_string().into()),
                    mods: Modifiers::default(),
                    captured: false,
                });
                // Type at a keyboard's pace, not faster than the writer thread
                // can wake: the input queue is bounded (8 chunks) and refuses
                // visibly past that, which is not what these gates measure.
                let deadline = Instant::now() + Duration::from_secs(5);
                while self.app.ptys["history-pane"].queued_input() != (0, 0) {
                    assert!(
                        Instant::now() < deadline,
                        "the writer must drain one keystroke"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            assert_eq!(
                self.app.ptys["history-pane"].take_recorded_input(),
                text.as_bytes(),
                "keyboard route did not deliver the exact fixture input"
            );
        }

        fn enter(&mut self) {
            self.app.ptys["history-pane"].take_recorded_input();
            let _ = self.app.update(Message::KeyPressed {
                key: Key::Named(Named::Enter),
                mods: Modifiers::default(),
                captured: false,
            });
            assert_eq!(self.app.ptys["history-pane"].take_recorded_input(), b"\r");
        }

        fn submit(&mut self, command: &str) {
            self.type_text(command);
            self.enter();
        }

        fn wait_title(&mut self, title: &str) {
            self.wait_until(&format!("OSC title {title}"), |app| {
                app.sessions[0].title == title
            });
        }

        fn ordinary(&mut self) {
            let before = self.app.sessions[0].in_app_history.len();
            self.submit(ORDINARY);
            self.wait_until("fresh ordinary command completed", |app| {
                let history = &app.sessions[0].in_app_history;
                history.len() == before + 1
                    && history
                        .newest_first(0)
                        .is_some_and(|item| item.command == ORDINARY && item.exit_code == Some(0))
            });
        }

        fn history_surfaces(&mut self, excluded: &str) -> Vec<CollectedNode> {
            let _ = self.app.update(Message::OpenHistory);
            let history = &self.app.sessions[0].in_app_history;
            assert!(
                !history.is_empty(),
                "ordinary-command positive control is missing"
            );
            for index in 0..history.len() {
                let item = history.newest_first(index).unwrap();
                assert!(!item.command.contains(excluded));
                assert!(!crate::history_view::command_label(item).contains(excluded));
            }
            let serialized = SessionState::snapshot(
                &self.app.sessions,
                self.app.active,
                &self.app.title_overrides,
            )
            .persist_serialize()
            .unwrap();
            assert!(!serialized.contains(excluded));
            let encoded: String = excluded
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert!(!serialized.contains(&encoded));
            let mut renderer = iced_test::futures::futures::executor::block_on(
                <iced_test::renderer::Renderer as Headless>::new(
                    iced::advanced::renderer::Settings::default(),
                    std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
                ),
            )
            .expect("headless renderer");
            let mut ui = iced_test::runtime::user_interface::UserInterface::build(
                self.app.history_overlay(&self.app.sessions[0]),
                SIZE,
                iced_test::runtime::user_interface::Cache::default(),
                &mut renderer,
            );
            let mut collector = SemanticsCollector::new();
            ui.operate(&renderer, {
                let operation: &mut dyn iced::advanced::widget::Operation = &mut collector;
                operation
            });
            let nodes = crate::i18n::test_nodes(collector.into_collected());
            assert!(nodes.iter().any(|node| {
                node.semantics.label.as_deref() == Some(&format!("Copy command: {ORDINARY}"))
            }));
            for node in &nodes {
                assert!(
                    !node
                        .semantics
                        .label
                        .as_deref()
                        .unwrap_or("")
                        .contains(excluded)
                );
                assert!(
                    !node
                        .semantics
                        .value
                        .as_deref()
                        .unwrap_or("")
                        .contains(excluded)
                );
            }
            nodes
        }
    }
}
