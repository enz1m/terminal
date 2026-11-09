//! Pane-input reduction: scoping, key routing, and IME.

use super::{release_focus_in, trace_key};
use crate::operation_origin::OperationOrigin;
use crate::{App, GridPointerProbe, Message};
use enzui_core::click::{ClickDetector, SingleClickPolicy};
use iced::advanced::input_method;
use iced::keyboard::{Key, Modifiers};
use iced::{Element, Task};

impl App {
    pub(crate) fn scope_element<'a>(
        &self,
        session: &str,
        pane: &str,
        element: Element<'a, Message>,
    ) -> Element<'a, Message> {
        if let Some(origin) = self
            .input_origin(session, pane)
            .filter(|origin| origin.window.is_some())
        {
            element.map(move |message| match message {
                message @ (Message::PaneInput(..)
                | Message::PanePointer(..)
                | Message::InputMenuEvent(..)) => message,
                message => Message::PaneInput(origin.clone(), Box::new(message)),
            })
        } else {
            element
        }
    }

    pub(crate) fn reduce_pane_input(
        &mut self,
        origin: OperationOrigin,
        mut message: Message,
    ) -> Task<Message> {
        // The terminal publishes every pointer move it does not act on; such
        // a move is no input to the pane and must not focus it.
        if matches!(message, Message::TerminalViewIgnored) {
            return Task::none();
        }
        let is_key = matches!(message, Message::InputKey(..));
        if !self.input_origin_live(&origin) {
            if is_key {
                trace_key("key dropped: the pane's input origin is no longer live");
            }
            return Task::none();
        }
        let Some(window) = origin.window else {
            return Task::none();
        };
        if let Message::InputKey(generation, key, mods, captured) = &message {
            if !self
                .window_inputs
                .get(&window)
                .is_some_and(|state| state.focus_generation == *generation && generation.is_live())
            {
                trace_key("key dropped: it carries a retired focus generation");
                return Task::none();
            }
            message = Message::KeyPressed {
                key: key.clone(),
                mods: *mods,
                captured: *captured,
            };
        }
        let Some(index) = self
            .sessions
            .iter()
            .position(|session| session.id == origin.session)
        else {
            return Task::none();
        };
        let Some(pane_index) = self.sessions[index]
            .panes
            .iter()
            .position(|pane| pane.id == origin.pane)
        else {
            return Task::none();
        };
        let main = self.main_window == Some(window);
        if main && index != self.active {
            if is_key {
                trace_key("key dropped: its session is not the active one");
            }
            return Task::none();
        }
        if matches!(
            message,
            Message::KeyPressed { .. }
                | Message::InputIme(..)
                | Message::GridCellDragged(_)
                | Message::GridCellReleased(_)
        ) {
            let refusal = if !self.input_is_active(&origin) {
                Some("key dropped: the pane is not the active input")
            } else if !self.window_holds_keyboard(window) {
                Some("key dropped: its window does not hold keyboard focus")
            } else {
                None
            };
            if let Some(refusal) = refusal {
                if is_key {
                    trace_key(refusal);
                }
                return Task::none();
            }
        }
        if is_key {
            trace_key("key delivered");
        }
        match &mut message {
            Message::RestoredGridResized(size) => {
                self.record_restored_grid_size(&origin.pane, *size);
                return Task::none();
            }
            Message::InputCursor(id, position) => {
                if *id == window {
                    self.window_inputs.entry(window).or_default().cursor = *position;
                }
                return Task::none();
            }
            Message::ModifiersChanged(mods) => {
                self.window_inputs.entry(window).or_default().mods = *mods;
                if main {
                    self.mods = *mods;
                }
                return Task::none();
            }
            Message::GridCellPressed(target)
            | Message::GridCellDragged(target)
            | Message::GridCellReleased(target) => target.pane = Some(pane_index),
            Message::FocusPane(id) => *id = origin.pane.clone(),
            Message::InputIme(generation, event, captured) => {
                if !self.window_inputs.get(&window).is_some_and(|state| {
                    state.focus_generation == *generation && generation.is_live()
                }) {
                    return Task::none();
                }
                return self.reduce_input_ime(origin, event.clone(), *captured);
            }
            Message::InputFocus(focused) => {
                if !*focused && !self.input_is_active(&origin) {
                    return Task::none();
                }
                if *focused {
                    self.sessions[index].active_pane = pane_index;
                }
                let state = self.window_inputs.entry(window).or_default();
                if state.terminal_focus != *focused {
                    state.retire_input_focus();
                }
                state.terminal_focus = *focused;
                return if *focused {
                    release_focus_in(window)
                } else {
                    Task::none()
                };
            }
            Message::InputMenuOpen => {
                self.open_input_menu(origin);
                return Task::none();
            }
            _ => {}
        }
        if matches!(message, Message::GridCellPressed(_)) {
            let state = self.window_inputs.entry(window).or_default();
            state.terminal_focus = true;
        }
        let terminal_focus = self
            .window_inputs
            .get(&window)
            .is_some_and(|state| state.terminal_focus);
        let composing = self
            .window_inputs
            .get(&window)
            .is_some_and(|state| state.ime.is_some() && state.composing);
        let withdraw_ime = matches!(
            &message,
            Message::FocusNext | Message::FocusPrevious | Message::OpenFind
        ) || matches!(&message, Message::KeyPressed { key, mods, .. } if self.shortcut_action_for_key(key, *mods).as_deref() == Some("overlay.find") || crate::chrome_focus_message(key, *mods).is_some());
        if withdraw_ime {
            let state = self.window_inputs.entry(window).or_default();
            state.terminal_focus = false;
            state.retire_input_focus();
        }
        // A pane's layout fit and its scrolling act on that pane, which the
        // reducers find as the active one, but they do not focus it: only a
        // click or a key does. The pane that had focus gets it back after.
        let keeps_focus = matches!(
            message,
            Message::TerminalFit { .. }
                | Message::TranscriptWheel(_)
                | Message::ScrollTranscriptToRow(_)
        )
        .then(|| {
            let session = &self.sessions[index];
            (
                session.id.clone(),
                session
                    .panes
                    .get(session.active_pane)
                    .map(|pane| pane.id.clone()),
            )
        });
        if !matches!(
            message,
            Message::GridCellPressed(_)
                | Message::GridCellDragged(_)
                | Message::GridCellReleased(_)
                | Message::PaneGridEvent(_)
        ) {
            self.sessions[index].active_pane = pane_index;
        }
        let previous = self
            .sessions
            .get(self.active)
            .map(|session| session.id.clone());
        let mut state = if main {
            None
        } else {
            Some(self.window_inputs.remove(&window).unwrap_or_default())
        };
        if let Some(state) = state.as_mut() {
            state.swap_app(self);
        }
        self.active = index;
        self.input_context = Some(origin.clone());
        if matches!(message, Message::GridCellPressed(_))
            && self
                .grid_pointer
                .pane
                .as_deref()
                .is_some_and(|id| id != origin.pane)
        {
            self.click_detector = ClickDetector::new(SingleClickPolicy::Immediate);
        }
        let find_was_open = self.find.open;
        let history_was_open = self
            .detached_windows
            .get(&window)
            .is_some_and(|state| state.history_open);
        let pressed = matches!(message, Message::GridCellPressed(_));
        let mut task = match message {
            Message::KeyPressed {
                key,
                mods,
                captured,
            } => self.scoped_key(&origin, key, mods, captured, (composing, terminal_focus)),
            Message::OpenFind => self.open_scoped_find(&origin),
            Message::FindQueryChanged(_) => {
                self.find_owner = Some(origin.pane.clone());
                self.update_inner(message)
            }
            Message::ContextRowActivated(_, row) => {
                self.update_inner(Message::ContextRowActivated(origin.session.clone(), row))
            }
            Message::FocusNext | Message::FocusPrevious => {
                let direction = if matches!(message, Message::FocusNext) {
                    enzui_core::focus::Traversal::Next
                } else {
                    enzui_core::focus::Traversal::Previous
                };
                enzui_core::app_runtime::window_operation_task(
                    window,
                    enzui_core::focus::traversal_operation::<()>(direction),
                )
            }
            other => self.update_inner(other),
        };
        let history_closed = history_was_open
            && self
                .detached_windows
                .get(&window)
                .is_some_and(|state| !state.history_open);
        if pressed || history_closed {
            task = Task::batch([task, release_focus_in(window)]);
        }
        if let Some((session_id, Some(pane_id))) = keeps_focus
            && let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id)
            && let Some(focused) = session.panes.iter().position(|pane| pane.id == pane_id)
        {
            session.active_pane = focused;
        }
        self.input_context = None;
        if let Some(mut state) = state {
            state.swap_app(self);
            let find_changed = state.find.open != find_was_open;
            if (find_changed && !state.find.open) || history_closed {
                state.retire_input_focus();
                state.terminal_focus = true;
            }
            if self.windows.is_open(window) {
                self.window_inputs.insert(window, state);
                if find_changed
                    && let Some(size) = self.detached_windows.get(&window).map(|window| window.size)
                {
                    self.resize_detached_pty(&origin.session, size);
                }
            }
            if let Some(index) =
                previous.and_then(|id| self.sessions.iter().position(|session| session.id == id))
            {
                self.active = index;
            } else {
                self.active = self.active.min(self.sessions.len().saturating_sub(1));
            }
        }
        task
    }

    fn open_scoped_find(&mut self, origin: &OperationOrigin) -> Task<Message> {
        self.find_owner = Some(origin.pane.clone());
        self.find.open();
        if let Some(pane) = self
            .sessions
            .iter()
            .flat_map(|session| &session.panes)
            .find(|pane| pane.id == origin.pane)
        {
            self.find.search(&pane.transcript);
        }
        let Some(window) = origin.window else {
            return Task::none();
        };
        let id = if self.main_window == Some(window) {
            crate::find_field_id()
        } else {
            crate::window_input_view::find_id(window)
        };
        enzui_core::app_runtime::window_focus_task(window, id)
    }

    pub(crate) fn clear_window_selection(&mut self, window: iced::window::Id) {
        if let Some(index) = self.session_index_for_window(window) {
            for pane in &self.sessions[index].panes {
                self.selections.remove(&pane.id);
            }
        }
        if let Some(state) = self.window_inputs.get_mut(&window) {
            state.pointer = GridPointerProbe::default();
        }
    }

    fn scoped_key(
        &mut self,
        origin: &OperationOrigin,
        key: Key,
        mods: Modifiers,
        captured: bool,
        (composing, terminal_focus): (bool, bool),
    ) -> Task<Message> {
        self.mods = mods;
        let Some(window) = origin.window else {
            return Task::none();
        };
        if let Some(command) = crate::chrome_focus_message(&key, mods) {
            if self.main_window == Some(window) && (self.settings_open || self.ssh_manager_open) {
                return Task::none();
            }
            let direction = if matches!(command, Message::FocusNext) {
                enzui_core::focus::Traversal::Next
            } else {
                enzui_core::focus::Traversal::Previous
            };
            return enzui_core::app_runtime::window_operation_task(
                window,
                enzui_core::focus::traversal_operation::<()>(direction),
            );
        }
        if !captured
            && (self.main_window != Some(window) || (!self.settings_open && !self.ssh_manager_open))
            && let Some(action) = self.shortcut_action_for_key(&key, mods)
        {
            match action.as_str() {
                "clipboard.copy" => return self.update_inner(Message::RequestCopy),
                "clipboard.paste" => return self.request_paste(),
                "overlay.find" => return self.open_scoped_find(origin),
                _ => {
                    if let Some(message) = Self::action_message(&action) {
                        // Detached shortcuts may mutate their own panes, never navigate or create a main-window tab.
                        if self.main_window == Some(window)
                            || matches!(
                                message,
                                Message::NewPane
                                    | Message::NewPaneDown
                                    | Message::ClosePane
                                    | Message::CloseActive
                                    | Message::ToggleHistory
                                    | Message::FocusPaneDir(_)
                            )
                        {
                            if self.main_window != Some(window)
                                && matches!(message, Message::ToggleHistory)
                            {
                                return self.update_inner(Message::DetachedToggleHistory(window));
                            }
                            return self.update_inner(message);
                        }
                        return Task::none();
                    }
                }
            }
        }
        if captured {
            return Task::none();
        }
        if composing {
            return Task::none();
        }
        if !terminal_focus {
            if key == Key::Named(iced::keyboard::key::Named::Escape) {
                if self.find.open {
                    return self.update_inner(Message::CloseFind);
                }
                if self
                    .detached_windows
                    .get(&window)
                    .is_some_and(|state| state.history_open)
                {
                    return self.update_inner(Message::DetachedCloseHistory(window));
                }
                if self.pane_rename_draft.is_some() {
                    return self.update_inner(Message::CancelPaneRename);
                }
                if self.main_window == Some(window) && self.session_rename.is_some() {
                    return self.update_inner(Message::CancelSessionRename);
                }
            }
            return Task::none();
        }
        if self.main_window != Some(window) {
            if self.find.open && key == Key::Named(iced::keyboard::key::Named::Escape) {
                return self.update_inner(Message::CloseFind);
            }
            return self.update_inner(Message::DetachedKeyPressed {
                window,
                pane: origin.pane.clone(),
                key,
                mods,
                captured,
            });
        }
        self.update_inner(Message::KeyPressed {
            key,
            mods,
            captured,
        })
    }

    fn reduce_input_ime(
        &mut self,
        origin: OperationOrigin,
        event: input_method::Event,
        captured: bool,
    ) -> Task<Message> {
        if captured || !self.ptys.contains_key(&origin.pane) {
            return Task::none();
        }
        let Some(window) = origin.window else {
            return Task::none();
        };
        let state = self.window_inputs.entry(window).or_default();
        if !state.terminal_focus {
            return Task::none();
        }
        match event {
            input_method::Event::Opened => {
                state.composing = false;
                state.ime = Some((origin, input_method::Preedit::new()));
            }
            input_method::Event::Preedit(content, selection) => {
                state.composing = !content.is_empty();
                if let Some((owner, preedit)) = state.ime.as_mut()
                    && *owner == origin
                {
                    preedit.selection = selection.filter(|range| {
                        range.start <= range.end
                            && range.end <= content.len()
                            && content.is_char_boundary(range.start)
                            && content.is_char_boundary(range.end)
                    });
                    preedit.content = content;
                }
            }
            input_method::Event::Commit(text) => {
                state.composing = false;
                if state
                    .ime
                    .as_ref()
                    .is_some_and(|(owner, _)| *owner == origin)
                    && !text.is_empty()
                {
                    if let Some((_, preedit)) = state.ime.as_mut() {
                        *preedit = input_method::Preedit::new();
                    }
                    self.type_pane_pty(&origin.session, &origin.pane, text.as_bytes());
                }
            }
            input_method::Event::Closed => {
                state.composing = false;
                state.ime = None;
            }
        }
        Task::none()
    }
}
