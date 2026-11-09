use crate::{
    App, Message,
    operation_origin::{OperationOrigin, RequestGeneration},
    pane_endpoint::PaneEndpoint,
    ssh_config::OpenSshHost,
    ssh_discovery::{Completion, Phase},
};
use iced::{Subscription, Task};

pub(crate) enum Intent {
    Refresh,
    Connect {
        host: OpenSshHost,
        origin: Option<OperationOrigin>,
    },
    Reconnect {
        origin: OperationOrigin,
        endpoint: PaneEndpoint,
    },
}
impl App {
    pub(crate) fn openssh_paint_subscription(&self) -> Subscription<Message> {
        if !self.openssh_discovery.enabled
            || self.openssh_discovery.redraw_seen
            || self
                .startup
                .as_ref()
                .is_some_and(|startup| startup.redraw_seen)
        {
            return Subscription::none();
        }
        // listen_with excludes redraws; this raw subscription retires at the main
        // redraw boundary instead of remaining an idle-frame poll.
        iced::event::listen_raw(|event, _, window| match event {
            iced::Event::Window(iced::window::Event::RedrawRequested(_)) => {
                Some(Message::OpenSshFirstFrame(window))
            }
            _ => None,
        })
    }
    pub(crate) fn openssh_first_frame(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.main_window != Some(window)
            || !self.windows.is_open(window)
            || self.openssh_discovery.redraw_seen
            || !self.openssh_discovery.enabled
        {
            return Task::none();
        }
        if let Some(startup) = self.startup.as_mut() {
            startup.redraw_seen = true;
            return self.finish_startup();
        }
        self.openssh_discovery.redraw_seen = true;
        let intent = self.openssh_intent.take().unwrap_or(Intent::Refresh);
        self.begin_openssh_reload(intent)
    }
    pub(crate) fn retire_openssh_choice(&mut self) {
        self.openssh_intent = None;
        self.openssh_discovery.invalidate();
    }
    pub(crate) fn begin_openssh_reload(&mut self, intent: Intent) -> Task<Message> {
        self.openssh_intent = Some(intent);
        self.openssh_hosts.clear();
        self.openssh_load_error = None;
        if !self.openssh_discovery.enabled {
            return Task::none();
        }
        match self.openssh_discovery.request() {
            Ok(Some(receiver)) => Task::run(receiver, Message::OpenSshDiscoveryFinished),
            Ok(None) => Task::none(),
            Err(error) => self.apply_openssh_discovery_answer(Err(error)),
        }
    }
    pub(crate) fn choose_openssh_host(
        &mut self,
        index: usize,
        generation: RequestGeneration,
    ) -> Task<Message> {
        if !self.openssh_discovery.is_current(&generation) {
            if matches!(
                &self.openssh_intent,
                Some(Intent::Connect { .. } | Intent::Reconnect { .. })
            ) {
                self.retire_openssh_choice();
            }
            return Task::none();
        }
        if self.openssh_discovery.enabled && self.openssh_discovery.phase != Phase::Ready {
            return Task::none();
        }
        let Some(host) = self.openssh_hosts.get(index).cloned() else {
            return Task::none();
        };
        if !self.openssh_discovery.enabled {
            return self.connect_openssh_host(host);
        }
        self.begin_openssh_reload(Intent::Connect {
            host,
            origin: self.operation_origin(),
        })
    }
    pub(crate) fn finish_openssh_discovery(&mut self, completion: Completion) -> Task<Message> {
        let Some(answer) = self.openssh_discovery.settle(completion) else {
            return Task::none();
        };
        self.apply_openssh_discovery_answer(answer)
    }
    fn openssh_origin_is_current(&self, origin: Option<&OperationOrigin>) -> bool {
        let active = self.sessions.get(self.active).and_then(|session| {
            session
                .panes
                .get(session.active_pane)
                .map(|pane| (session, pane))
        });
        match (origin, active) {
            (None, None) => true,
            (Some(origin), Some((session, pane))) => {
                self.operation_origin_is_live(origin)
                    && origin.session == session.id
                    && origin.pane == pane.id
            }
            _ => false,
        }
    }
    fn apply_openssh_discovery_answer(
        &mut self,
        answer: crate::ssh_discovery::Answer,
    ) -> Task<Message> {
        let intent = self.openssh_intent.take();
        let succeeded = answer.is_ok();
        self.apply_openssh_config_result(answer);
        match intent {
            Some(Intent::Connect { host, origin })
                if succeeded && self.openssh_origin_is_current(origin.as_ref()) =>
            {
                return self.connect_openssh_host(host);
            }
            Some(Intent::Reconnect { origin, endpoint })
                if self.operation_origin_is_live(&origin)
                    && self
                        .sessions
                        .iter()
                        .flat_map(|s| &s.panes)
                        .any(|p| p.id == origin.pane && p.endpoint == endpoint)
                    && !self.ptys.contains_key(&origin.pane) =>
            {
                if succeeded {
                    return self.start_pane_endpoint_current(&origin.session, &origin.pane);
                }
                self.pane_endpoint_error(
                    &origin.pane,
                    "OpenSSH discovery is unavailable. Reload Remote before reconnecting.",
                );
            }
            _ => {}
        }
        Task::none()
    }
}
