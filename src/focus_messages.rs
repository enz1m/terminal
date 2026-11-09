//! Terminal session and chrome focus-message mappers.

use crate::Message;
#[cfg(test)]
use crate::keymap;
use iced::Task;
use iced::keyboard::{self, Modifiers};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionRowFocusTarget {
    pub(crate) tree: iced::advanced::widget::Id,
    pub(crate) row: iced::advanced::widget::Id,
}

/// Focus the tree's one real focus stop and move its roving cursor to the
/// surviving session row. A row id alone is an embedded descendant and cannot
/// be focused directly.
pub(crate) fn restore_session_row_focus_operation_for<T>(
    target: &SessionRowFocusTarget,
) -> Box<dyn iced::advanced::widget::Operation<T>>
where
    T: Send + 'static,
{
    use enzui_widgets::tree_view::{TreeTarget, focus_tree_row};
    Box::new(focus_tree_row(
        TreeTarget::widget(target.tree.clone()),
        Some(target.row.clone()),
    ))
}

pub(crate) fn restore_session_row_focus_task(target: SessionRowFocusTarget) -> Task<Message> {
    iced::advanced::widget::operate(restore_session_row_focus_operation_for(&target))
}

pub(crate) fn window_focus_message(
    window: iced::window::Id,
    event: iced::window::Event,
) -> Option<Message> {
    match event {
        iced::window::Event::Focused => Some(Message::WindowFocusChanged(window, true)),
        iced::window::Event::Unfocused => Some(Message::WindowFocusChanged(window, false)),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn window_key_pressed_message<S: AsRef<str>>(
    window: iced::window::Id,
    key: keyboard::Key,
    modified_key: keyboard::Key,
    text: Option<S>,
    mods: Modifiers,
    captured: bool,
) -> Message {
    Message::WindowKeyPressed {
        window,
        key: keymap::terminal_input_key(
            &key,
            &modified_key,
            text.as_ref().map(AsRef::as_ref),
            mods,
        ),
        mods,
        captured,
    }
}

pub(crate) fn chrome_focus_message(key: &keyboard::Key, mods: Modifiers) -> Option<Message> {
    if !mods.control() {
        return None;
    }
    match key {
        keyboard::Key::Named(keyboard::key::Named::F6) => Some(if mods.shift() {
            Message::FocusPrevious
        } else {
            Message::FocusNext
        }),
        _ => None,
    }
}
