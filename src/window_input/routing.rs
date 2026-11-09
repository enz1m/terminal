use super::*;

impl App {
    pub(crate) fn input_subscription(&self) -> iced::Subscription<Message> {
        let opening = self
            .opening_window
            .filter(|window| !self.windows.is_open(*window));
        iced::Subscription::batch(
            self.windows
                .open_windows()
                .chain(opening)
                .filter_map(|window| {
                    let origin = self
                        .session_index_for_window(window)
                        .and_then(|index| self.sessions.get(index))
                        .and_then(|session| {
                            session
                                .panes
                                .get(session.active_pane)
                                .and_then(|pane| self.input_origin(&session.id, &pane.id))
                        })
                        .filter(|origin| origin.window == Some(window));
                    let state = self.window_inputs.get(&window)?;
                    let epoch = state.focus_generation.clone();
                    let clock = state.focus_clock.clone();
                    Some(
                iced::event::listen_with(|event, status, window| Some((window, event, status)))
                    .with((window, origin, epoch, clock))
                    .filter_map(|((owner_window, origin, epoch, clock), (window, event, status))| {
                        if owner_window != window {
                            return None;
                        }
                        let focused = match event {
                            iced::Event::Window(iced::window::Event::Focused) => Some(true),
                            iced::Event::Window(iced::window::Event::Unfocused) => Some(false),
                            _ => None,
                        };
                        if let Some(focused) = focused {
                            return Some(Message::InputFocusEdge(window, clock.edge(focused)));
                        }
                        let captured = matches!(status, iced::event::Status::Captured);
                        let stamped = |message: Message| {
                            let stamp = clock.stamp()?;
                            Some(Message::StampedInput(window, stamp, Box::new(message)))
                        };
                        let message = match event {
                            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                                key,
                                modified_key,
                                text,
                                modifiers,
                                ..
                            }) => {
                                let key = crate::keymap::terminal_input_key(
                                    &key,
                                    &modified_key,
                                    text.as_deref(),
                                    modifiers,
                                );
                                if origin.is_none() {
                                    return stamped(Message::WindowChromeKey(
                                        window, epoch, key, modifiers, captured,
                                    ));
                                }
                                Message::InputKey(epoch, key, modifiers, captured)
                            }
                            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
                                mods,
                            )) => Message::ModifiersChanged(mods),
                            iced::Event::InputMethod(event) => {
                                Message::InputIme(epoch, event, captured)
                            }
                            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                                Message::InputCursor(window, position)
                            }
                            _ => return None,
                        };
                        let routed = Message::PaneInput(origin?, Box::new(message));
                        if matches!(
                            &routed,
                            Message::PaneInput(_, inner)
                                if matches!(**inner, Message::InputKey(..) | Message::InputIme(..))
                        ) {
                            stamped(routed)
                        } else {
                            Some(routed)
                        }
                    }),
            )
                }),
        )
    }

    pub(crate) fn input_text_size(
        &self,
        window: iced::window::Id,
        session: &model::Session,
        pane: &model::Pane,
    ) -> iced::Size {
        let size = self
            .detached_windows
            .get(&window)
            .map_or(iced::Size::ZERO, |window| window.size);
        let surface = iced::Size::new(
            size.width,
            (size.height - self.detached_top_strip() - self.input_find_height(window)).max(0.0),
        );
        let rect = if session.panes.len() == 1
            || crate::pane_layout::is_narrow_layout_with(
                size.width,
                session.panes.len(),
                &self.enz_theme().sizing,
            ) {
            surface
        } else {
            session
                .layout
                .pane_grid_state()
                .pane_rects(surface)
                .into_iter()
                .find(|(id, _)| *id == &pane.id)
                .map_or(surface, |(_, rect)| rect.size())
        };
        let rect = self.restored_surface_size(&pane.id).unwrap_or(rect);
        let inset = self.pane_inset(pane.alt_screen);
        iced::Size::new(
            (rect.width - 2.0 * inset).max(0.0),
            (rect.height - 2.0 * inset).max(0.0),
        )
    }

    pub(crate) fn scoped_grid_window(&self, origin: &OperationOrigin) -> Option<(usize, usize)> {
        let session = self
            .sessions
            .iter()
            .find(|session| session.id == origin.session)?;
        let pane = session.panes.iter().find(|pane| pane.id == origin.pane)?;
        let window = origin.window?;
        if self.main_window == Some(window) {
            let rows = self
                .geometry()
                .rows_for(self.pane_text_size(&pane.id).height) as usize;
            return Some(self.pane_transcript_window(
                pane,
                rows,
                self.find_owner.as_deref().is_none_or(|id| id == pane.id),
            ));
        }
        let rows =
            self.geometry()
                .rows_for(self.input_text_size(window, session, pane).height) as usize;
        let position = self
            .detached_windows
            .get(&window)?
            .viewports
            .get(&pane.id)
            .map_or(pane.scroll, |viewport| viewport.scroll);
        let find = if self
            .input_context
            .as_ref()
            .is_some_and(|owner| owner.window == Some(window))
        {
            Some((&self.find, &self.find_owner))
        } else {
            self.window_inputs
                .get(&window)
                .map(|state| (&state.find, &state.find_owner))
        };
        let row = find.and_then(|(find, owner)| {
            (find.open && owner.as_deref() == Some(&pane.id))
                .then_some(find.scroll_to_row)
                .flatten()
        });
        Some(Self::transcript_window(
            pane.transcript.len(),
            rows,
            self.geometry().cell_height,
            position,
            pane.alt_screen,
            row,
        ))
    }

    pub(super) fn open_input_menu(&mut self, origin: OperationOrigin) {
        use enzui_core::context_menu::{InvocationKind, Point};
        let Some(window) = origin.window else {
            return;
        };
        let mut actions = HashMap::new();
        let (entries, submenus) = self.pane_menu_entries(&origin, &mut actions);
        let surface = origin;
        let Some(origin) = self.operation_origin_for(&surface.session, &surface.pane) else {
            return;
        };
        let input = self.window_inputs.entry(window).or_default();
        input.retire_input_focus();
        input.terminal_focus = false;
        let mut state = ContextMenuState::closed();
        state.open(InvocationKind::Pointer(Point::new(
            input.cursor.x,
            input.cursor.y,
        )));
        input.menu = Some(InputMenu {
            origin,
            surface,
            model: MenuModel::new(entries),
            state,
            actions,
            submenus: submenus
                .into_iter()
                .map(|(parent, sub)| (parent.to_owned(), sub))
                .collect(),
        });
    }

    /// The pane's right-click menu: clipboard, find, then splitting and
    /// grouping each folded into a submenu, and closing the pane once there is
    /// more than one. Closing the session is on its sidebar row.
    fn pane_menu_entries(
        &self,
        origin: &OperationOrigin,
        actions: &mut HashMap<String, Message>,
    ) -> (Vec<MenuEntryModel>, Vec<(&'static str, MenuModel)>) {
        let hint = |entry: MenuEntryModel, action: &str| match self.shortcut_chord(action) {
            Some(chord) => entry.shortcut(chord),
            None => entry,
        };
        let session = self.sessions.iter().find(|s| s.id == origin.session);
        let has_selection = self.selection_for(&origin.pane).is_some();
        let mut entries = vec![
            hint(
                MenuEntryModel::command("copy", crate::i18n::label("Copy"))
                    .icon("copy")
                    .enabled(has_selection),
                "clipboard.copy",
            ),
            hint(
                MenuEntryModel::command("paste", crate::i18n::label("Paste"))
                    .icon("clipboard-paste"),
                "clipboard.paste",
            ),
            MenuEntryModel::separator("clipboard-end"),
            hint(
                MenuEntryModel::command("find", crate::i18n::label("Find…")).icon("search"),
                "overlay.find",
            ),
            MenuEntryModel::separator("find-end"),
            MenuEntryModel::submenu("split", crate::i18n::label("Split pane")).icon("columns-2"),
        ];
        actions.insert("copy".to_owned(), Message::RequestCopy);
        actions.insert("paste".to_owned(), Message::RequestPaste);
        actions.insert("find".to_owned(), Message::OpenFind);
        actions.insert("split-right".to_owned(), Message::NewPane);
        actions.insert("split-down".to_owned(), Message::NewPaneDown);
        let mut submenus = vec![(
            "split",
            MenuModel::new(vec![
                hint(
                    MenuEntryModel::command("split-right", crate::i18n::label("Right"))
                        .icon("columns-2"),
                    "pane.split-horizontal",
                ),
                hint(
                    MenuEntryModel::command("split-down", crate::i18n::label("Down"))
                        .icon("rows-2"),
                    "pane.split-vertical",
                ),
            ]),
        )];

        let targets: Vec<MenuEntryModel> = self
            .sessions
            .iter()
            .filter(|session| session.id != origin.session && !session.detached)
            .map(|target| {
                let id = format!("group:{}", target.id);
                actions.insert(id.clone(), Message::InputGroupInto(target.id.clone()));
                MenuEntryModel::command(id, target.title.clone())
            })
            .collect();
        if !targets.is_empty() {
            entries.push(
                MenuEntryModel::submenu("group", crate::i18n::label("Group with")).icon("group"),
            );
            submenus.push(("group", MenuModel::new(targets)));
        }

        let pinned = session.is_some_and(|session| session.pinned);
        let split = session.is_some_and(|session| session.panes.len() > 1);
        if pinned || split {
            entries.push(MenuEntryModel::separator("session-end"));
        }
        if pinned {
            entries.push(
                MenuEntryModel::command(
                    "clear-saved-output",
                    crate::i18n::label("Clear saved output"),
                )
                .icon("eraser"),
            );
            actions.insert("clear-saved-output".to_owned(), Message::ClearPinnedOutput);
        }
        if split {
            entries.push(
                hint(
                    MenuEntryModel::command("close-pane", crate::i18n::label("Close pane"))
                        .icon("x"),
                    "pane.close",
                )
                .destructive(true),
            );
            actions.insert("close-pane".to_owned(), Message::ClosePane);
        }
        (entries, submenus)
    }

    pub(crate) fn input_menu_live(&self, origin: &OperationOrigin) -> bool {
        self.operation_origin_is_live(origin)
            && origin
                .window
                .and_then(|window| self.window_inputs.get(&window))
                .and_then(|state| state.menu.as_ref())
                .is_some_and(|menu| menu.origin == *origin && self.input_origin_live(&menu.surface))
    }

    pub(crate) fn reduce_input_menu(
        &mut self,
        origin: OperationOrigin,
        event: ContextMenuEvent,
    ) -> Task<Message> {
        if !self.input_menu_live(&origin) {
            return Task::none();
        }
        let Some(window) = origin.window else {
            return Task::none();
        };
        let Some(menu) = self
            .window_inputs
            .get_mut(&window)
            .and_then(|state| state.menu.as_mut())
        else {
            return Task::none();
        };
        let submenus = &menu.submenus;
        let resolver = |_depth: usize, parent: &enzui_core::menu_model::EntryId| {
            submenus.get(parent.as_str()).cloned()
        };
        let update = enzui_widgets::context_menu_host::apply_event_restoring_with::<Message>(
            &mut menu.model,
            &mut menu.state,
            &event,
            &resolver,
        );
        let selected = update
            .activated()
            .and_then(|id| menu.actions.get(id.as_str()))
            .cloned();
        let origin = menu.surface.clone();
        if !menu.state.is_open()
            && let Some(input) = self.window_inputs.get_mut(&window)
        {
            input.menu = None;
            input.retire_input_focus();
            input.terminal_focus = true;
        }
        let task = selected.map_or_else(Task::none, |message| {
            self.reduce_pane_input(origin, message)
        });
        update.finish(task)
    }

    pub(crate) fn retire_link(&mut self, owner: &link_open::UriOwner) {
        if let Some(link) = self.pending_links.remove(owner) {
            link.origin.generation.retire(CancelCause::Cancelled);
            if let Some(generation) = link.activation {
                self.activation_broker.cancel(generation);
            }
        }
        self.uri_portal
            .cancel_owner(owner, enzui_core::portal::CancelReason::ParentClosed);
    }

    pub(crate) fn begin_owned_link(&mut self, target: link_open::OpenTarget) -> Task<Message> {
        let Some(origin) = self.operation_origin() else {
            return Task::none();
        };
        let owner = link_open::UriOwner {
            session: origin.session.clone(),
            pane: origin.pane.clone(),
        };
        self.retire_link(&owner);
        self.pending_links.insert(
            owner.clone(),
            PendingLink {
                origin: origin.clone(),
                activation: None,
            },
        );
        match target {
            link_open::OpenTarget::Uri(uri) => {
                if let Some(window) = origin.window
                    && let Ok((generation, task)) =
                        self.activation_broker.request(window, Instant::now())
                {
                    self.pending_links.get_mut(&owner).unwrap().activation = Some(generation);
                    return task.map(move |response| {
                        Message::OwnedUriActivation(
                            origin.clone(),
                            uri.clone(),
                            enzui_parent_lease::Handoff::new(response),
                        )
                    });
                }
                self.start_owned_uri(origin, uri, None)
            }
            link_open::OpenTarget::Path(path) => {
                let opener = self.link_opener.clone();
                let task_origin = origin.clone();
                Task::perform(
                    async move {
                        if !task_origin.generation.is_live() {
                            return None;
                        }
                        Some(opener.open_path(path).await)
                    },
                    move |result| Message::OwnedLinkFinished(origin, result),
                )
            }
        }
    }

    pub(crate) fn owned_link_live(&self, origin: &OperationOrigin) -> bool {
        let owner = link_open::UriOwner {
            session: origin.session.clone(),
            pane: origin.pane.clone(),
        };
        self.operation_origin_is_live(origin)
            && self
                .pending_links
                .get(&owner)
                .is_some_and(|link| link.origin == *origin)
    }

    pub(crate) fn start_owned_uri(
        &mut self,
        origin: OperationOrigin,
        uri: String,
        activation: Option<enzui_core::activation::ActivationContext>,
    ) -> Task<Message> {
        if !self.owned_link_live(&origin) {
            return Task::none();
        }
        let owner = link_open::UriOwner {
            session: origin.session.clone(),
            pane: origin.pane.clone(),
        };
        match self
            .uri_portal
            .start_with_activation(owner, uri, activation)
        {
            Ok(_) => Task::none(),
            Err(error) => Task::done(Message::OwnedLinkFinished(origin, Some(Err(error)))),
        }
    }

    pub(crate) fn finish_owned_link(
        &mut self,
        origin: OperationOrigin,
        result: Result<(), link_open::LinkOpenError>,
    ) {
        if !self.owned_link_live(&origin) {
            return;
        }
        let feedback = result.err().map(|error| {
            eprintln!(
                "terminal link-open failure: kind={} diagnostic=\"{}\"",
                error.kind().as_str(),
                error.diagnostic()
            );
            error.user_summary().to_owned()
        });
        if origin.window == self.main_window {
            self.pane_feedback = feedback;
        } else if let Some(window) = origin.window {
            self.window_inputs.entry(window).or_default().feedback = feedback;
        }
        self.retire_link(&link_open::UriOwner {
            session: origin.session,
            pane: origin.pane,
        });
    }

    pub(crate) fn retire_window_input(&mut self, window: iced::window::Id) {
        let links: Vec<_> = self
            .pending_links
            .iter()
            .filter(|(_, link)| link.origin.window == Some(window))
            .map(|(owner, _)| owner.clone())
            .collect();
        for owner in links {
            self.retire_link(&owner);
        }
        self.input_origins.retain(|_, origin| {
            if origin.window == Some(window) {
                origin.generation.retire(CancelCause::Disconnected);
                false
            } else {
                true
            }
        });
        if let Some(state) = self.window_inputs.remove(&window) {
            state.focus_generation.retire(CancelCause::Disconnected);
        }
        self.credential_ops
            .retain(|op| op.origin.window != Some(window));
    }
}
