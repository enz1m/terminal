//! Terminal reducer batch 1: input and link arms.

use super::*;

impl App {
    #[expect(
        clippy::boxed_local,
        reason = "The reducer consumes the boxed recursive message payload"
    )]
    pub(crate) fn reduce1_pane_pointer_0(
        &mut self,
        origin: operation_origin::OperationOrigin,
        start: usize,
        message: Box<Message>,
    ) -> Task<Message> {
        self.input_frame_start = Some(start);
        let task = self.reduce_pane_input(origin, *message);
        self.input_frame_start = None;
        task
    }
    pub(crate) fn reduce1_input_sink_1(&mut self) -> Task<Message> {
        Task::none()
    }
    pub(crate) fn reduce1_input_cursor_2(
        &mut self,
        window: iced::window::Id,
        point: iced::Point,
    ) -> Task<Message> {
        if self.windows.is_open(window) {
            self.window_inputs.entry(window).or_default().cursor = point;
        }

        Task::none()
    }
    pub(crate) fn reduce1_input_group_into_3(&mut self, target: String) -> Task<Message> {
        if let Some(origin) = self.input_context.clone()
            && target != origin.session
            && self
                .sessions
                .iter()
                .any(|session| session.id == target && !session.detached)
        {
            if let Some(window) = origin
                .window
                .filter(|window| self.detached_windows.contains_key(window))
            {
                self.retire_window_input(window);
                self.detached_windows.remove(&window);
                self.orphaned_windows.push(window);
                if let Some(session) = self
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == origin.session)
                {
                    session.detached = false;
                }
            }
            self.apply_group_into(&target, &origin.session, pane_group::GroupEdge::Right, None);
        }

        Task::none()
    }
    pub(crate) fn reduce1_owned_uri_activation_4(
        &mut self,
        origin: operation_origin::OperationOrigin,
        uri: String,
        response: enzui_parent_lease::Handoff<ActivationResponse>,
    ) -> Task<Message> {
        if let Some(response) = response.take() {
            let context = self.activation_broker.finish(response, Instant::now()).ok();
            return self.start_owned_uri(origin, uri, context);
        }

        Task::none()
    }
    pub(crate) fn reduce1_owned_link_finished_5(
        &mut self,
        origin: operation_origin::OperationOrigin,
        result: Option<Result<(), link_open::LinkOpenError>>,
    ) -> Task<Message> {
        if let Some(result) = result {
            self.finish_owned_link(origin, result);
        }

        Task::none()
    }
    pub(crate) fn reduce1_theme_6(&mut self, event: Box<ThemeEvent>) -> Task<Message> {
        let before = self.themes.theme_generation();
        let task = self.themes.update(*event);
        // Terminal ANSI identity stays in `color_scheme`; the retained
        // shell sidebar captures spacing and must rebuild only when the
        // published shared theme actually changes.
        if self.themes.theme_generation() != before {
            self.invalidate_sidebar();
            self.resolve_terminal_family();
        }
        task.map(|event| Message::Theme(Box::new(event)))
    }

    pub(crate) fn reduce1_modifiers_changed_8(&mut self, m: Modifiers) -> Task<Message> {
        self.mods = m;

        Task::none()
    }
}
