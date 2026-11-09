//! Terminal jump and transcript surface methods.

use super::*;

impl App {
    /// The `Jump to latest` affordance, as a button only (placement is
    /// [`Self::transcript_surface`]'s).
    pub(crate) fn jump_to_latest_button(&self, message: Message) -> Element<'_, Message> {
        enz_button(crate::i18n::label("Jump to latest"), message)
            .kind(ButtonKind::Secondary)
            .icon("arrow-down")
            .aria_label(crate::i18n::label("Jump to latest output"))
            .into_element()
    }

    /// A transcript surface: the scrollbar chrome with the transient
    /// `Jump to latest` affordance floated over its trailing-bottom corner.
    ///
    /// The stack is mounted unconditionally with the chrome as its first child.
    /// A conditional stack would change the widget at that tree position when
    /// the affordance toggles, and `Tree::diff` then discards the chrome's
    /// `ChromeState` — the live drag origin — mid-gesture. The second layer
    /// swaps between the button and an inert `Space`; the chrome keeps its
    /// identity across the toggle. The layer is `Fill`/`Fill` with the button
    /// in the corner and right padding clearing the scrollbar grab lane, and a
    /// `container` declines pointer events so the wheel and lane presses reach
    /// the chrome below.
    pub(crate) fn transcript_surface<'a>(
        &self,
        chrome: Element<'a, Message>,
        jump: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let sp = self.enz_theme().spacing;
        let layer: Element<'a, Message> = match jump {
            Some(button) => button,
            None => Space::new().width(0.0).height(0.0).into(),
        };
        iced::widget::stack![
            chrome,
            container(layer)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::End)
                .align_y(Alignment::End)
                .padding(iced::Padding::from(sp.sm()).right(scrollback::hit_width_px() + sp.xs())),
        ]
        .into()
    }
}
