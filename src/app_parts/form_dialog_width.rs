//! Terminal dialog-width method.

use super::*;

impl App {
    /// Build the settings dialog *surface* only. The scrim, centring, inert
    /// background and dismissal are owned by the `enz_modal_host` in `view`.
    ///
    /// # The framework sizes the form dialog
    ///
    /// Before this, the dialog was `container(column![enz_text("Terminal
    /// settings"), settings_row(..) × 4, done_row]).style(surface_elevated +
    /// radius 12 + 1px border)`, where `settings_row` was a local free
    /// function — `row![container(label).width(Fixed(140)), control]` — whose
    /// *name is the framework widget's name*. Every leaf was already a
    /// framework widget (`enz_text`, `enz_combo_box`, `enz_button`), which is
    /// exactly why no token detector could see it: it is the second form of
    /// hand-roll, a composite rebuilt from our own primitives.
    ///
    /// It is now `enz_preferences_group` + one `enz_preferences_row` per
    /// setting, each with `.control(enz_combo_box(..))`, on an
    /// `enz_modal_surface`. What that GAINS — none of it visible in a
    /// screenshot:
    ///
    /// * **Section semantics.** The group publishes one labelled
    ///   `Role::Group` ("Terminal settings"), so AT announces the four
    ///   dropdowns as one titled section instead of four unrelated combo
    ///   boxes next to four loose text runs.
    /// * **Row semantics.** Each `.control(..)` row publishes its own
    ///   `Role::Group` named by the row title, wrapping label column and
    ///   control *together* — the association a `row![text, control]` cannot
    ///   express, and the reason the hand-rolled version left a screen-reader
    ///   user hearing an unnamed control beside a floating text node.
    /// * **The control's accessible name is ours to give** (`.control` docs,
    ///   contract 1): unlike `.switch(..)`, the row builds no control and
    ///   cannot name an opaque `Element`. `enz_combo_box` has no
    ///   `aria_label`, and a non-empty `label` would draw a second *visible*
    ///   caption beside the one the row already paints — so the name is
    ///   carried on `.placeholder(title)`, which reaches the `ComboBox` node
    ///   and renders nothing while a selection exists. That is the form the
    ///   framework's own `measured_combo_box_naming_options_in_the_control_slot`
    ///   pins.
    /// * **The label column belongs to the row, not to us.** The hand-roll's
    ///   `Length::Fixed(140.0)` existed solely to stop a `Length::Fill` combo
    ///   halving the row; the row's fixed `inline_field_width` trailing slot
    ///   does that for every row at once, so sibling rows cannot drift apart.
    ///   That deletes the last `Length::Fixed(<literal>)` in this file bar the
    ///   dialog width.
    /// * **The 42 px Adw row rhythm and the body/label type ramp** resolve
    ///   from the forwarded theme scales, not from this app's hand-picked
    ///   12 px row spacing.
    /// * **`enz_modal_surface`** replaces the hand-written `container::Style`
    ///   closure — the `surface_elevated` fill, the `radius 12` and the 1 px
    ///   `border` are the surface's own tokens, plus a theme-resolved
    ///   elevation shadow the hand-roll never painted.
    ///
    /// The group is left **flat** (the default): it sits inside a modal
    /// surface that already draws card chrome, and a bordered group would
    /// double it.
    /// Modal-surface width for this app's single-column **form dialogs**
    /// (settings, SSH manager). Both are short forms, the role
    /// `SizingTokens::dialog_width_md` (460) names in `sizing.rs`, so both read
    /// the one token instead of drifting apart on two taste-literals. Named so
    /// a guard test can pin that it *reads the token*, not that it merely
    /// clears a threshold — the calendar `swatch_hit_px()` pattern.
    pub(crate) fn form_dialog_width(&self) -> f32 {
        self.enz_theme().sizing.dialog_width_md
    }
}
