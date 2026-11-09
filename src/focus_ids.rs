//! Terminal widget-id and focus-task helpers.

use crate::{FIND_PLACEHOLDER, Message};
use iced::Task;

/// Decode a key press into a **chrome focus traversal** message, or `None` to
/// let the key fall through to `Message::KeyPressed` (and thence to the PTY).
///
/// `Ctrl+F6` → [`Message::FocusNext`], `Ctrl+Shift+F6` →
/// [`Message::FocusPrevious`]. See the `FocusNext` doc-comment for why the
/// chord is F6 rather than Tab in a terminal.
///
/// A free function, not an inline `match` in the subscription closure: the
/// subscription is unreachable from a test, so an inline decision would be an
/// untestable island.
/// Stable id of the titlebar "Settings" gear — the invoker and focus-restore
/// target of the Settings modal. At the compact rung the gear folds
/// into the overflow kebab and is not in the tree; `RestoreFocus` probes
/// first, so a restore then leaves focus where it is (a safe no-op).
pub(crate) fn settings_button_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("terminal.titlebar.settings")
}

/// Stable caller-owned namespace for the compact titlebar menu's keyed rows.
/// Automation and routed accessibility requests derive row ids from this plus
/// each command descriptor.
pub(crate) const TITLEBAR_OVERFLOW_MENU_ID_PREFIX: &str = "terminal-titlebar-overflow";

/// Stable id of the compact titlebar overflow trigger. The framework retains
/// this focusable node while its menu is open and after either dismissal path,
/// so keyboard focus has a durable return target.
pub(crate) fn titlebar_overflow_button_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("terminal.titlebar.overflow")
}

/// The collapsed trigger the titlebar find field answers to at the narrow rung.
///
/// Read from the framework accessor (`collapsed_trigger_id_for`) rather than
/// re-deriving the id here, so the placeholder and prefix this app passes are
/// the ones the id is built from.
pub(crate) fn find_trigger_id() -> iced::advanced::widget::Id {
    enzui_widgets::search_field::EnzSearchField::<Message>::collapsed_trigger_id_for(
        enzui_widgets::search_field::DEFAULT_ID_PREFIX,
        crate::i18n::label(FIND_PLACEHOLDER),
    )
}

/// What "focus the find field" means, in order, as REAL operations rather than
/// an opaque `Task`.
///
/// Two, because the field has two presentations: at the narrow rung the visible
/// surface is the collapsed trigger, so the activation opens the popover, and
/// the focus then lands on the field the popover carries. At the wide rung the
/// activation finds no trigger and the focus alone does the work. Exposed as
/// operations so a gate can run them over a rendered `UserInterface` and read
/// the landed focus back.
pub(crate) fn find_focus_operations<T>() -> Vec<Box<dyn iced::advanced::widget::Operation<T>>>
where
    T: Send + 'static,
{
    vec![
        Box::new(enzui_widgets::activation::activate(find_trigger_id())),
        Box::new(iced::advanced::widget::operation::focusable::focus(
            find_field_id(),
        )),
    ]
}

pub(crate) fn focus_find_task() -> Task<Message> {
    Task::batch(
        find_focus_operations::<Message>()
            .into_iter()
            .map(|op| iced::advanced::widget::operate(op).discard()),
    )
}

/// The sidebar's inline editor for a session's name.
pub(crate) fn session_rename_field_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("term-session-rename-field")
}

/// Focus the session-name editor with its text selected, so typing replaces
/// the old name and Escape or Enter is the only other step.
pub(crate) fn focus_session_rename_task() -> Task<Message> {
    let id = session_rename_field_id();
    Task::batch([
        iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::focus::<()>(
            id.clone(),
        ))
        .discard(),
        iced::advanced::widget::operate(
            iced::advanced::widget::operation::text_input::select_all::<()>(id),
        )
        .discard(),
    ])
}

/// Clear keyboard focus from every focusable in the window.
///
/// The operation itself is the framework's `release_all_focus`, so the gate
/// runs that same operation over a rendered `UserInterface` and reads the
/// landed focus back instead of asserting on an opaque `Task`.
pub(crate) fn release_chrome_focus_task() -> Task<Message> {
    iced::advanced::widget::operate(enzui_core::focus::release_all_focus()).discard()
}

/// Stable id of the titlebar find field's query input.
///
/// One literal, shared by `App::view` (which stamps it on the
/// `enz_titlebar_search` via `.id(..)`) and the `Message::OpenFind` reducer arm
/// (which hands it to `iced::widget::text_input::focus`). This is the wiring
/// `docs/widgets.md` documents for focusing a search surface on its opening
/// transition.
pub(crate) fn find_field_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("terminal-emulator.find.query")
}
