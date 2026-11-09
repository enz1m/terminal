//! Terminal settings-dialog strings and row builder.

use crate::Message;
use enzui_widgets::preferences_row::enz_preferences_row;
use iced::Element;

// Deleting a saved SSH host goes through this confirmation dialog.
pub(crate) const CONFIRM_DELETE_SSH_TITLE: &str = "Delete SSH host?";
pub(crate) const CONFIRM_DELETE_SSH_BODY: &str =
    "This removes the saved connection. This cannot be undone.";

/// The appearance section title.
pub(crate) const APPEARANCE_TITLE: &str = "Appearance";

/// Settings-row titles. Cursor style is a live combo whose value is consumed
/// by `grid_view::terminal_grid`.
///
/// The fourth entry, `"Background"`, is gone with the Dark/Darker/Black
/// preset row it labelled.
pub(crate) const SETTINGS_ROWS: [&str; 3] = ["Color scheme", "Font size", "Cursor style"];

// The previous file kept an `APPEARANCE_ROWS: [&str; 4]` table of
// row titles, but only the removed `Font family (informational)` entry ever
// read from it. The Font size row builds its own radio-group label, the
// scrollback row's title is `model::SCROLLBACK_BOUNDS_LABEL`, and the
// background section is a `column!` of `enz_text` + three buttons. Deleting
// the table drops a table that promised structure it never had.

/// The accessible name of the informational font-family row that
/// this file used to render. The absence gate
/// `the_font_family_row_is_absent_from_the_rendered_tree` asserts NO row
/// with this name reaches the AT tree.
#[cfg(test)]
pub(crate) const REMOVED_FONT_FAMILY_ROW_TITLE: &str = "Font family (informational)";

/// One settings row: the framework `enz_preferences_row` with the dropdown in
/// its opaque-control slot.
///
/// This replaced a local `settings_row` free function whose body was
/// `row![container(enz_text(label)).width(Length::Fixed(140.0)), control]` — a
/// hand-rolled preferences row down to the hardcoded label column the real
/// row's trailing slot exists to make unnecessary. See `settings_dialog` for
/// the full gain list.
///
/// The caller is responsible for the control's accessible name (contract 1 on
/// `EnzPreferencesRow::control`): the row paints the title itself and cannot
/// rename an opaque `Element`, so every caller here passes
/// `.placeholder(title)` on its combo box.
pub(crate) fn settings_row<'a>(
    title: &'a str,
    control: enzui_widgets::combo_box::EnzComboBox<'a, Message>,
) -> Element<'a, Message> {
    enz_preferences_row(title)
        .control(control.into_element())
        .into_element()
}
