//! Terminal SSH dialog methods.

use super::*;

impl App {
    /// Build the SSH connection manager dialog surface.
    ///
    /// The host list keeps its manager surface. Add/edit switches to the
    /// shared form surface: fields, errors, secure reveal and the action row
    /// are rendered from the presenter's one authoritative session.
    ///
    /// # Dismissal policy (stacked-modal contract)
    ///
    /// When the form is on top, its own [`FormDialogEvent::Resolved`] is
    /// authoritative for every dismissal route: Cancel, Escape (routed via
    /// [`FormDialogEvent::from_host`] on the outer modal), and scrim outside
    /// press ALL peel the form back to the host list — the manager stays
    /// open. This is deliberate; a second Escape at the list level then
    /// closes the manager. `enz_modal_surface::on_dismiss` is NOT wired on
    /// the form surface so its automatic circular Close X is suppressed,
    /// leaving Cancel as the one visible affordance and preventing a
    /// silent fourth dismissal route from skipping the form level.
    ///
    /// The reverse mapping lives on `App::view_shell`: when `ssh_form` is
    /// `Some`, host-modal events are funneled through
    /// `Message::SshFormEvent(FormDialogEvent::from_host(..))`; otherwise
    /// they go to `Message::SshManagerModalEvent`, whose reducer arm closes
    /// the manager. Because the two paths NEVER run at the same time, one
    /// Escape peels one level and no dismissal route silently discards a
    /// different amount than another.
    pub(crate) fn ssh_manager_dialog(&self) -> Element<'_, Message> {
        if let Some(form) = self.ssh_form.as_ref() {
            let title = if form.editing_id.is_some() {
                crate::i18n::label("Edit SSH host")
            } else {
                crate::i18n::label("Add connection")
            };
            let viewport_height = if self.content_size.height > 0.5 {
                self.content_size.height
            } else {
                900.0
            };
            let surface_padding = enzui_widgets::modal_surface::DEFAULT_DIALOG_PADDING.px();
            let mut dialog =
                enz_form_dialog(title, form.presenter.session(), Message::SshFormEvent)
                    .editors(form.presenter.bridge())
                    .submit_label(if self.pending_ssh_host_save.is_some() {
                        crate::i18n::label("Saving connection…")
                    } else {
                        crate::i18n::label("Save connection")
                    })
                    .cancel_label(crate::i18n::label("Cancel"))
                    // The shared capped body measures its title/actions and
                    // scrolls only the fields. Subtract the modal surface's
                    // real vertical inset so the full card stays in view.
                    .max_height((viewport_height - surface_padding * 2.0).max(1.0));
            if let Some(error) = &form.save_error {
                dialog = dialog.description(crate::i18n::text(error));
            } else if self.ssh_hosts.is_empty() && form.editing_id.is_none() {
                dialog = dialog.description(crate::i18n::label(
                    "Fill in the form below to add your first connection.",
                ));
            }
            return dialog.into_surface();
        }

        let enz = self.enz_theme();
        let sp = enz.spacing;
        enz_modal_surface(self.ssh_host_list_view(&sp, enz))
            .on_dismiss(|| Message::CloseSshManager)
            .width(Length::Fixed(self.form_dialog_width()))
            .padding(sp.space9)
            .into_element()
    }

    /// The destructive confirmation ("are you sure?"), hosted in its own
    /// modal so it stacks ABOVE whatever dialog raised it (the settings dialog
    /// for reset-shortcuts, the SSH manager for delete-host).
    ///
    /// # Uses the shipped confirm preset
    ///
    /// It used to build the surface itself — `enz_modal_surface` around a
    /// `column![headline, muted body, row![Space, Ghost "Cancel",
    /// Destructive confirm]]` — which is `enz_confirm_dialog` rewritten out of
    /// the primitives it is made of, and rewritten with two exits: the buttons
    /// went to `on_confirm(bool)` while Escape and the scrim went to a
    /// separate `ModalHostEvent` mapper. Two places ending one dialog is the
    /// bug the preset's docs name explicitly, and the six peers that already
    /// call `enz_confirm_dialog` were all getting the one-exit version.
    ///
    /// The preset also supplies what the hand-roll silently dropped: the
    /// dialog's `Semantics`/`DismissPolicy` pairing, stable button ids for
    /// focus operations, and a single `ConfirmDecision::from_cause` mapping
    /// from *how* the dialog ended to *whether the user consented*.
    ///
    /// `alert` is deliberately left FALSE: both of these are Escape-closable
    /// (an alert refuses Escape), and cancelling by Escape is the behaviour
    /// `declining_delete_ssh_host_preserves_the_host` pins.
    pub(crate) fn confirm_overlay(
        &self,
        title: &str,
        body_text: &str,
        confirm_label: &str,
        on_event: fn(ConfirmDialogEvent) -> Message,
    ) -> Element<'_, Message> {
        // Host over an empty base so the scrim covers the whole window and
        // Escape / a scrim press funnel through `on_event` (-> cancel).
        let base: Element<'_, Message> =
            Space::new().width(Length::Fill).height(Length::Fill).into();
        enz_confirm_dialog(
            title.to_owned(),
            body_text.to_owned(),
            &self.confirm_state,
            on_event,
        )
        .confirm_label(confirm_label.to_owned())
        .cancel_label(crate::i18n::label("Cancel"))
        .destructive(true)
        .into_host(base)
    }
}
