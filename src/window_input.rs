use crate::operation_origin::{OperationOrigin, RequestGeneration};
use crate::{App, GridPointerProbe, Message, link_open, model};
use enzui_core::cancellation::CancelCause;
use enzui_core::click::{ClickDetector, SingleClickPolicy};
use enzui_core::menu_model::{MenuEntryModel, MenuModel};
use enzui_widgets::context_menu_host::{ContextMenuEvent, ContextMenuState};
use iced::Task;
use iced::advanced::input_method;
use iced::keyboard::{Key, Modifiers};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

pub(super) fn release_focus_in(window: iced::window::Id) -> Task<Message> {
    enzui_core::app_runtime::window_operation_task(
        window,
        iced::advanced::widget::operation::map(enzui_core::focus::release_all_focus(), |_| ()),
    )
}

/// Counts one window's keyboard focus edges inside its input stream.
///
/// Focus edges and keys are stamped by the same ordered stream, so a key is
/// current exactly when no focus edge was mapped between it and the edge the
/// app last applied. The identity is stable for the window's lifetime; a focus
/// change therefore does not rebuild the stream and lose events in flight.
#[derive(Clone, Debug, Default)]
pub(crate) struct FocusClock(Arc<FocusClockState>);

#[derive(Debug)]
struct FocusClockState {
    edges: AtomicU64,
    focused: AtomicBool,
}

impl Default for FocusClockState {
    fn default() -> Self {
        Self {
            edges: AtomicU64::new(0),
            focused: AtomicBool::new(true),
        }
    }
}

impl FocusClock {
    pub(crate) fn edge(&self, focused: bool) -> u64 {
        self.0.focused.store(focused, Ordering::SeqCst);
        self.0.edges.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The stamp for an input event, or `None` while the stream has seen the
    /// window lose keyboard focus.
    pub(crate) fn stamp(&self) -> Option<u64> {
        self.0
            .focused
            .load(Ordering::SeqCst)
            .then(|| self.0.edges.load(Ordering::SeqCst))
    }
}

impl PartialEq for FocusClock {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for FocusClock {}

impl std::hash::Hash for FocusClock {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}

pub(crate) struct InputMenu {
    pub(crate) origin: OperationOrigin,
    pub(crate) surface: OperationOrigin,
    pub(crate) model: MenuModel,
    pub(crate) state: ContextMenuState,
    actions: HashMap<String, Message>,
    /// Child menus by the entry that opens them.
    pub(crate) submenus: HashMap<String, MenuModel>,
}

impl Drop for InputMenu {
    fn drop(&mut self) {
        self.origin.generation.retire(CancelCause::Disconnected);
    }
}

pub(crate) struct WindowInput {
    pub(crate) find: model::FindState,
    pub(crate) find_owner: Option<String>,
    pointer: GridPointerProbe,
    clicks: ClickDetector,
    mouse_cell: (usize, usize),
    pub(crate) mods: Modifiers,
    pub(crate) cursor: iced::Point,
    pub(crate) feedback: Option<String>,
    rename: Option<String>,
    pub(crate) menu: Option<InputMenu>,
    pub(crate) terminal_focus: bool,
    pub(crate) blurred: bool,
    composing: bool,
    modal_active: bool,
    pub(crate) focus_generation: RequestGeneration,
    pub(crate) focus_clock: FocusClock,
    /// The latest focus edge the app applied from `focus_clock`.
    pub(crate) focus_mark: u64,
    bound_pane: Option<String>,
    bound_pty: bool,
    pub(crate) ime: Option<(OperationOrigin, input_method::Preedit)>,
}

impl Default for WindowInput {
    fn default() -> Self {
        Self {
            find: model::FindState::default(),
            find_owner: None,
            pointer: GridPointerProbe::default(),
            clicks: ClickDetector::new(SingleClickPolicy::Immediate),
            mouse_cell: (1, 1),
            mods: Modifiers::default(),
            cursor: iced::Point::ORIGIN,
            feedback: None,
            rename: None,
            menu: None,
            terminal_focus: true,
            blurred: false,
            composing: false,
            modal_active: false,
            focus_generation: RequestGeneration::new(),
            focus_clock: FocusClock::default(),
            focus_mark: 0,
            bound_pane: None,
            bound_pty: false,
            ime: None,
        }
    }
}

impl WindowInput {
    pub(crate) fn retire_input_focus(&mut self) {
        self.focus_generation.retire(CancelCause::Cancelled);
        self.focus_generation = RequestGeneration::new();
        self.ime = None;
        self.composing = false;
    }

    /// A window focus edge ends composition but keeps the input target; keys
    /// already delivered to the focused surface remain that target's input.
    pub(crate) fn end_composition(&mut self) {
        self.ime = None;
        self.composing = false;
    }

    pub(crate) fn bound_pty(&self) -> bool {
        self.bound_pty
    }

    fn swap_app(&mut self, app: &mut App) {
        std::mem::swap(&mut self.find, &mut app.find);
        std::mem::swap(&mut self.find_owner, &mut app.find_owner);
        std::mem::swap(&mut self.pointer, &mut app.grid_pointer);
        std::mem::swap(&mut self.clicks, &mut app.click_detector);
        std::mem::swap(&mut self.mouse_cell, &mut app.mouse_last_cell);
        std::mem::swap(&mut self.mods, &mut app.mods);
        std::mem::swap(&mut self.feedback, &mut app.pane_feedback);
        std::mem::swap(&mut self.rename, &mut app.pane_rename_draft);
    }
}

/// With `ENZUI_TERMINAL_KEY_TRACE=1`, says on stderr what became of each key
/// a pane received, naming the gate that dropped it, and when a window's
/// focus changed. Off otherwise.
pub(crate) fn trace_key(outcome: &str) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ENABLED.get_or_init(|| {
        std::env::var_os("ENZUI_TERMINAL_KEY_TRACE").is_some_and(|value| value == "1")
    }) {
        eprintln!("terminal input: {outcome}");
    }
}

pub(crate) struct PendingLink {
    pub(crate) origin: OperationOrigin,
    activation: Option<enzui_core::activation::ActivationGeneration>,
}

#[path = "window_input/reduce.rs"]
mod reduce;

/// A key typed before any pane could take input.
pub(crate) struct Typeahead {
    window: iced::window::Id,
    stamp: u64,
    key: Key,
    mods: Modifiers,
    captured: bool,
    at: Instant,
}

// Bounds type-ahead to a short command line and to the start-up interval.
const TYPEAHEAD_KEYS: usize = 256;
const TYPEAHEAD_TTL: std::time::Duration = std::time::Duration::from_secs(10);

impl App {
    /// Whether `window` holds keyboard focus for the message being reduced.
    /// A current stamp from the window's ordered input stream is authoritative;
    /// otherwise the separately delivered window focus state decides.
    pub(crate) fn expect_main_window(&mut self, window: iced::window::Id) {
        self.opening_window = Some(window);
        self.window_inputs.entry(window).or_default();
    }

    pub(crate) fn window_holds_keyboard(&self, window: iced::window::Id) -> bool {
        self.ordered_focus == Some(window)
            || (self
                .window_inputs
                .get(&window)
                .is_some_and(|state| !state.blurred)
                && self.focused_window.is_none_or(|focused| focused == window))
    }

    pub(crate) fn apply_focus_edge(&mut self, window: iced::window::Id, edge: u64) {
        let state = self.window_inputs.entry(window).or_default();
        state.focus_mark = state.focus_mark.max(edge);
    }

    pub(crate) fn reduce_stamped_input(
        &mut self,
        window: iced::window::Id,
        stamp: u64,
        message: Message,
    ) -> Task<Message> {
        let has_session = self.session_index_for_window(window).is_some();
        let pane_starting = has_session && !self.window_has_pane_input(window);
        // A stream built before the pane existed maps keys as chrome keys;
        // once the window has a session they belong to its active pane.
        let late_chrome = has_session && matches!(message, Message::WindowChromeKey(..));
        if self.startup.is_some() || pane_starting || late_chrome {
            let typed = match &message {
                Message::WindowChromeKey(_, _, key, mods, captured) => Some((key, mods, captured)),
                Message::PaneInput(_, inner) => match &**inner {
                    Message::InputKey(_, key, mods, captured) => Some((key, mods, captured)),
                    _ => None,
                },
                _ => None,
            };
            if let Some((key, mods, captured)) = typed
                && self.is_typeahead(key, *mods)
            {
                if self.typeahead.len() < TYPEAHEAD_KEYS {
                    self.typeahead.push(Typeahead {
                        window,
                        stamp,
                        key: key.clone(),
                        mods: *mods,
                        captured: *captured,
                        at: Instant::now(),
                    });
                    trace_key("key held: typed before the pane accepts input");
                } else {
                    trace_key("key dropped: type-ahead is full");
                }
                return Task::none();
            }
            if self.startup.is_some() {
                return Task::none();
            }
        }
        if self
            .window_inputs
            .get(&window)
            .is_none_or(|state| state.focus_mark != stamp)
        {
            trace_key("key dropped: a focus change intervened");
            return Task::none();
        }
        self.ordered_focus = Some(window);
        let task = self.update_message(message);
        self.ordered_focus = None;
        task
    }

    fn window_has_pane_input(&self, window: iced::window::Id) -> bool {
        self.session_index_for_window(window).is_some()
            && self
                .window_inputs
                .get(&window)
                .is_some_and(WindowInput::bound_pty)
    }

    fn is_typeahead(&self, key: &Key, mods: Modifiers) -> bool {
        crate::chrome_focus_message(key, mods).is_none()
            && self.shortcut_action_for_key(key, mods).is_none()
    }

    /// Replays held type-ahead into the window's active pane once it accepts
    /// input. Each key keeps its original focus stamp, so a focus change since
    /// it was typed still drops it.
    pub(crate) fn drain_typeahead(&mut self) -> Task<Message> {
        if self.typeahead.is_empty() || self.startup.is_some() {
            return Task::none();
        }
        let now = Instant::now();
        self.typeahead
            .retain(|held| now.duration_since(held.at) < TYPEAHEAD_TTL);
        let mut tasks = Vec::new();
        let mut waiting = Vec::new();
        for held in std::mem::take(&mut self.typeahead) {
            let origin = self
                .session_index_for_window(held.window)
                .and_then(|index| self.sessions.get(index))
                .and_then(|session| {
                    let pane = session.panes.get(session.active_pane)?;
                    self.input_origin(&session.id, &pane.id)
                });
            let (Some(origin), true) = (origin, self.window_has_pane_input(held.window)) else {
                waiting.push(held);
                continue;
            };
            let Some(epoch) = self
                .window_inputs
                .get(&held.window)
                .map(|state| state.focus_generation.clone())
            else {
                continue;
            };
            let message = Message::PaneInput(
                origin,
                Box::new(Message::InputKey(epoch, held.key, held.mods, held.captured)),
            );
            trace_key("key replayed: typed before the pane accepted input");
            tasks.push(self.reduce_stamped_input(held.window, held.stamp, message));
        }
        self.typeahead = waiting;
        Task::batch(tasks)
    }

    pub(crate) fn input_origin_live(&self, origin: &OperationOrigin) -> bool {
        self.operation_origin_is_live(origin)
            && self.input_origins.get(&origin.pane) == Some(origin)
    }

    pub(crate) fn release_input_focus(&mut self) -> Task<Message> {
        let window = self
            .input_context
            .as_ref()
            .and_then(|origin| origin.window)
            .or(self.main_window);
        if let Some(window) = window {
            if let Some(state) = self.window_inputs.get_mut(&window) {
                if !state.terminal_focus {
                    state.retire_input_focus();
                }
                state.terminal_focus = true;
            }
            release_focus_in(window)
        } else {
            crate::release_chrome_focus_task()
        }
    }

    pub(crate) fn accepts_pane_input(&self, origin: &OperationOrigin, message: &Message) -> bool {
        if !self.input_origin_live(origin) {
            return false;
        }
        if origin.window == self.main_window
            && !self
                .sessions
                .get(self.active)
                .is_some_and(|session| session.id == origin.session)
        {
            return false;
        }
        match message {
            Message::InputKey(epoch, ..) | Message::InputIme(epoch, ..) => {
                self.input_is_active(origin)
                    && epoch.is_live()
                    && origin.window.is_some_and(|window| {
                        self.window_inputs
                            .get(&window)
                            .is_some_and(|state| state.focus_generation == *epoch)
                            && self.window_holds_keyboard(window)
                    })
            }
            _ => true,
        }
    }

    pub(crate) fn input_origin(&self, session: &str, pane: &str) -> Option<OperationOrigin> {
        self.input_origins
            .get(pane)
            .filter(|origin| origin.session == session && self.input_origin_live(origin))
            .cloned()
    }

    pub(crate) fn input_is_active(&self, origin: &OperationOrigin) -> bool {
        let Some(window) = origin.window else {
            return false;
        };
        self.session_index_for_window(window)
            .and_then(|index| self.sessions.get(index))
            .is_some_and(|session| {
                session.id == origin.session
                    && session
                        .panes
                        .get(session.active_pane)
                        .is_some_and(|pane| pane.id == origin.pane)
            })
    }

    pub(crate) fn pointer_epoch_is_current(
        &self,
        origin: &OperationOrigin,
        epoch: &RequestGeneration,
    ) -> bool {
        if epoch.is_live() && self.transcript_epochs.get(&origin.pane) == Some(epoch) {
            return true;
        }
        let mods = if origin.window == self.main_window {
            self.mods
        } else {
            origin
                .window
                .and_then(|window| self.window_inputs.get(&window))
                .map_or(Modifiers::default(), |state| state.mods)
        };
        // PTY mouse reports use screen cells, not retained-history coordinates; repaint must not swallow button-up.
        !mods.shift()
            && self
                .sessions
                .iter()
                .filter(|session| session.id == origin.session)
                .flat_map(|session| &session.panes)
                .any(|pane| pane.id == origin.pane && pane.mouse_protocol.tracks())
    }

    pub(crate) fn invalidate_pane_coordinates(&mut self, pane: &str) {
        if let Some(epoch) = self
            .transcript_epochs
            .insert(pane.to_owned(), RequestGeneration::new())
        {
            epoch.retire(CancelCause::Cancelled);
        }
        self.selections.remove(pane);
        if self.grid_pointer.pane.as_deref() == Some(pane) {
            self.grid_pointer = GridPointerProbe::default();
            self.click_detector = ClickDetector::new(SingleClickPolicy::Immediate);
        }
        let reset_find = |find: &mut model::FindState| {
            let old = std::mem::take(find);
            find.query = old.query;
            find.open = old.open;
        };
        if self.find_owner.as_deref() == Some(pane)
            || (self.find_owner.is_none() && self.active_pane_id().as_deref() == Some(pane))
        {
            reset_find(&mut self.find);
        }
        for state in self.window_inputs.values_mut() {
            if state.pointer.pane.as_deref() == Some(pane) {
                state.pointer = GridPointerProbe::default();
                state.clicks = ClickDetector::new(SingleClickPolicy::Immediate);
            }
            if state.find_owner.as_deref() == Some(pane) {
                reset_find(&mut state.find);
            }
        }
        for session in &mut self.sessions {
            if let Some(pane) = session.panes.iter_mut().find(|item| item.id == pane) {
                pane.scroll.jump_to_tail();
            }
        }
        for window in self.detached_windows.values_mut() {
            window.viewports.remove(pane);
        }
    }

    pub(crate) fn reconcile_window_inputs(&mut self) {
        self.transcript_epochs.retain(|pane, epoch| {
            let live = self
                .sessions
                .iter()
                .any(|session| session.panes.iter().any(|p| p.id == *pane));
            if !live {
                epoch.retire(CancelCause::Disconnected);
            }
            live
        });
        for pane in self.sessions.iter().flat_map(|session| &session.panes) {
            self.transcript_epochs
                .entry(pane.id.clone())
                .or_insert_with(RequestGeneration::new);
        }
        let dead: Vec<_> = self
            .input_origins
            .values()
            .filter(|origin| !self.operation_origin_is_live(origin))
            .cloned()
            .collect();
        for origin in dead {
            origin.generation.retire(CancelCause::Disconnected);
            self.input_origins.remove(&origin.pane);
            self.selections.remove(&origin.pane);
            self.retire_link(&link_open::UriOwner {
                session: origin.session,
                pane: origin.pane,
            });
        }
        let panes: Vec<_> = self
            .sessions
            .iter()
            .flat_map(|session| {
                session
                    .panes
                    .iter()
                    .map(|pane| (session.id.clone(), pane.id.clone()))
            })
            .collect();
        for (session, pane) in panes {
            if !self.input_origins.contains_key(&pane)
                && let Some(origin) = self.operation_origin_for(&session, &pane)
            {
                if let Some(window) = origin.window {
                    self.window_inputs.entry(window).or_default();
                }
                self.input_origins.insert(pane, origin);
            }
        }
        let focus: Vec<_> = self
            .windows
            .open_windows()
            .map(|window| {
                let pane = self
                    .session_index_for_window(window)
                    .and_then(|index| self.sessions.get(index))
                    .and_then(|session| session.panes.get(session.active_pane))
                    .map(|pane| pane.id.clone());
                let live = pane
                    .as_ref()
                    .is_some_and(|pane| self.ptys.contains_key(pane));
                (window, pane, live)
            })
            .collect();
        for (window, pane, live) in focus {
            let state = self.window_inputs.entry(window).or_default();
            if state.bound_pane != pane || state.bound_pty != live {
                state.retire_input_focus();
                state.bound_pane = pane;
                state.bound_pty = live;
            }
        }
        let stale_ime: Vec<_> = self
            .window_inputs
            .iter()
            .filter_map(|(window, state)| {
                state
                    .ime
                    .as_ref()
                    .filter(|(origin, _)| {
                        !self.input_origin_live(origin) || !self.input_is_active(origin)
                    })
                    .map(|_| *window)
            })
            .collect();
        for window in stale_ime {
            if let Some(state) = self.window_inputs.get_mut(&window) {
                state.ime = None;
            }
        }
        self.window_inputs
            .retain(|window, _| self.windows.is_open(*window));
        let dead_links: Vec<_> = self
            .pending_links
            .iter()
            .filter(|(_, link)| !self.operation_origin_is_live(&link.origin))
            .map(|(owner, _)| owner.clone())
            .collect();
        for owner in dead_links {
            self.retire_link(&owner);
        }
        if let Some(owner) = self.find_owner.as_deref() {
            if let Some(pane) = self
                .sessions
                .iter()
                .flat_map(|session| &session.panes)
                .find(|pane| pane.id == owner)
            {
                if self.find.open {
                    self.find.search(&pane.transcript);
                }
            } else {
                self.find.close();
                self.find_owner = None;
            }
        }
        for state in self.window_inputs.values_mut() {
            if let Some(owner) = state.find_owner.as_deref() {
                if let Some(pane) = self
                    .sessions
                    .iter()
                    .flat_map(|session| &session.panes)
                    .find(|pane| pane.id == owner)
                {
                    if state.find.open {
                        state.find.search(&pane.transcript);
                    }
                } else {
                    state.find.close();
                    state.find_owner = None;
                }
            }
        }
        if let Some(window) = self.main_window
            && let Some(state) = self.window_inputs.get_mut(&window)
        {
            let modal = self.settings_open || self.ssh_manager_open;
            if state.modal_active != modal {
                state.modal_active = modal;
                state.retire_input_focus();
                state.terminal_focus = !modal;
            }
            if modal {
                state.menu = None;
            }
        }
    }

    pub(crate) fn window_chrome_live(
        &self,
        window: iced::window::Id,
        epoch: &RequestGeneration,
    ) -> bool {
        self.main_window == Some(window)
            && self.windows.is_open(window)
            && self.session_index_for_window(window).is_none()
            && (self.ordered_focus == Some(window) || self.focused_window == Some(window))
            && epoch.is_live()
            && self.window_holds_keyboard(window)
            && self
                .window_inputs
                .get(&window)
                .is_some_and(|state| state.focus_generation == *epoch)
    }

    pub(crate) fn reduce_window_chrome(
        &mut self,
        window: iced::window::Id,
        key: Key,
        mods: Modifiers,
        captured: bool,
    ) -> Task<Message> {
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
            && let Some(action) = self.shortcut_action_for_key(&key, mods)
            && (matches!(
                action.as_str(),
                "tab.new"
                    | "tab.next"
                    | "tab.next-alt"
                    | "tab.prev"
                    | "tab.prev-alt"
                    | "overlay.settings"
                    | "overlay.ssh"
                    | "ui.toggle-sidebar"
            ) || action.starts_with("tab.goto-"))
            && let Some(message) = Self::action_message(&action)
        {
            return self.update_inner(message);
        }
        Task::none()
    }
}

#[cfg(test)]
#[path = "window_input_tests.rs"]
mod tests;

mod routing;
