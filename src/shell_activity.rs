use crate::{App, Message, shell_history::ShellEvent};
use enzui_activity::{CenterJobs, Publisher, Visibility};
use enzui_core::operation_activity::{
    Activity, ActivityCenter, ActivityKey, ActivityKind, ActivitySchedule, Generation, ProviderId,
    TerminalOutcome,
};
use iced::{Subscription, Task, window};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const PROVIDER: ProviderId = ProviderId("terminal-commands");

#[derive(Default)]
struct WindowJobs {
    jobs: CenterJobs,
    pending: Option<String>,
    token: Option<String>,
}

pub(crate) struct ShellActivity {
    publisher: Option<Publisher>,
    center: ActivityCenter,
    commands: HashMap<u64, String>,
    windows: HashMap<window::Id, WindowJobs>,
    clock: Instant,
    schedule: ActivitySchedule,
    timer: Option<(u64, Duration)>,
}

impl Default for ShellActivity {
    fn default() -> Self {
        let mut center = ActivityCenter::new();
        center.register_provider(PROVIDER, true);
        Self {
            publisher: None,
            center,
            commands: HashMap::new(),
            windows: HashMap::new(),
            clock: Instant::now(),
            schedule: ActivitySchedule::default(),
            timer: None,
        }
    }
}

impl ShellActivity {
    pub(crate) fn production() -> Self {
        Self {
            publisher: Publisher::new("io.github.enzui.Terminal").ok(),
            ..Self::default()
        }
    }

    fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    pub(crate) fn bind_window(&mut self, window: window::Id) -> Task<Message> {
        if self.publisher.is_none() || self.windows.contains_key(&window) {
            return Task::none();
        }
        let pending = enzui_activity::new_window_token().ok();
        self.windows.insert(
            window,
            WindowJobs {
                pending: pending.clone(),
                ..WindowJobs::default()
            },
        );
        let Some(tag) = pending else {
            return Task::none();
        };
        iced::window::set_toplevel_tag(window, tag.clone())
            .map(move |queued| Message::ActivityWindowTagged(window, tag.clone(), queued))
    }

    pub(crate) fn confirm_tag(&mut self, window: window::Id, tag: &str, queued: bool) {
        if let Some(state) = self.windows.get_mut(&window)
            && state.pending.as_deref() == Some(tag)
        {
            state.token = queued.then(|| tag.to_owned());
            state.pending = None;
        }
    }

    pub(crate) fn event(&mut self, pane: &str, event: &ShellEvent) {
        if self.publisher.is_none() {
            return;
        }
        let now = self.now_ms();
        match event {
            ShellEvent::Started { key, .. } => {
                let old: Vec<_> = self
                    .commands
                    .iter()
                    .filter(|(_, origin)| origin.as_str() == pane)
                    .map(|(key, _)| *key)
                    .collect();
                for old in old {
                    self.center.dismiss(ActivityKey::new(PROVIDER.0, old));
                    self.commands.remove(&old);
                }
                self.commands.insert(*key, pane.to_owned());
                self.center.publish(
                    Activity::new(
                        ActivityKey::new(PROVIDER.0, *key),
                        ActivityKind::Other("command"),
                        "",
                    ),
                    now,
                );
            }
            ShellEvent::Finished {
                key: Some(key),
                status,
            } if self.commands.get(key).is_some_and(|origin| origin == pane) => {
                let outcome = if *status == 0 {
                    TerminalOutcome::Completed
                } else {
                    TerminalOutcome::Failed
                };
                self.center.finish(
                    ActivityKey::new(PROVIDER.0, *key),
                    Generation::FIRST,
                    outcome,
                    now,
                );
            }
            ShellEvent::Unavailable => {
                let keys: Vec<_> = self
                    .commands
                    .iter()
                    .filter(|(_, origin)| origin.as_str() == pane)
                    .map(|(key, _)| *key)
                    .collect();
                for key in keys {
                    self.center.dismiss(ActivityKey::new(PROVIDER.0, key));
                    self.commands.remove(&key);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn prune(&mut self, deadline: u64) {
        if self.schedule.armed_deadline() == Some(deadline) {
            self.schedule.fired();
            self.center.prune(self.now_ms());
        }
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        self.timer
            .map_or_else(Subscription::none, |(deadline, wait)| {
                iced::time::every(wait)
                    .with(deadline)
                    .map(|(deadline, _)| Message::ActivityPrune(deadline))
            })
    }
}

impl App {
    pub(crate) fn sync_shell_activity(&mut self) {
        if self.shell_activity.publisher.is_none() {
            return;
        }
        let mut origins = HashMap::new();
        for (&key, pane) in &self.shell_activity.commands {
            if let Some(session) = self
                .sessions
                .iter()
                .find(|session| session.panes.iter().any(|item| &item.id == pane))
                && let Some(window) = self.detached_window_of(&session.id).or(self.main_window)
            {
                origins.insert(key, window);
            }
        }
        let state = &mut self.shell_activity;
        let missing: Vec<_> = state
            .commands
            .keys()
            .filter(|key| !origins.contains_key(key))
            .copied()
            .collect();
        for key in missing {
            state.center.dismiss(ActivityKey::new(PROVIDER.0, key));
        }
        state.commands.retain(|key, _| {
            state
                .center
                .get(ActivityKey::new(PROVIDER.0, *key))
                .is_some()
        });
        state.windows.retain(|window, _| {
            self.main_window == Some(*window) || self.detached_windows.contains_key(window)
        });
        if let Some(publisher) = &state.publisher {
            for (window, record) in &mut state.windows {
                record.jobs.sync(
                    publisher,
                    &state.center,
                    record.token.as_deref(),
                    Visibility::WhenUnfocused,
                    |row| {
                        (origins.get(&row.key.op) == Some(window))
                            .then(|| crate::i18n::label("Running command").to_owned())
                    },
                );
            }
        }
        if let Some(wait) = state.schedule.sync(&state.center, state.now_ms()) {
            state.timer = state
                .schedule
                .armed_deadline()
                .map(|deadline| (deadline, wait));
        } else if state.schedule.armed_deadline().is_none() {
            state.timer = None;
        }
    }
}
