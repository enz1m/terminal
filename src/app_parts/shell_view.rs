//! Terminal shell-view method.

use super::*;

impl App {
    pub(crate) fn shell_view(&self) -> Element<'_, Message> {
        // Force the sidebar presentation from `self.content_size`, which is
        // tracked through `iced::window::resize_events()`.  The resize
        // subscription fires on the initial compositor configure, so
        // `content_size` carries the real window geometry within one event
        // cycle.
        //
        // Why force from application state instead of letting the nav split
        // widget decide from its layout constraints:
        //
        // The widget's `Widget::layout` reads `limits.max().width`, which
        // comes from iced's internal window size.  On the FIRST layout pass,
        // before the compositor's configure event has been processed, that
        // internal size is winit's default (1024×768) — not the compositor's
        // actual tiled size.  An `iced::time::every(33ms)` poll subscription
        // (needed for live PTY output) triggers a render BEFORE the configure
        // arrives, so the first painted frame uses the stale 1024px width and
        // shows the sidebar inline.  The capture harness photographs that
        // stale frame.
        //
        // By forcing the presentation from `content_size` — which starts at
        // ZERO and updates to the real geometry on the very first `Resized`
        // event — the first frame defaults to Hidden (no sidebar), and the
        // sidebar appears only after the resize event confirms there is room.
        // At desktop (1400×900) the resize event arrives within one poll tick
        // (~33ms), so the sidebar appears before the capture harness polls
        // (every ~150ms).
        //
        // An earlier approach wrapped the view in `iced::widget::responsive`,
        // which has the same stale-first-frame problem and also re-invokes its
        // closure on every layout pass.
        let page: Element<'_, Message> = self.shell_composition();

        // The right-click session menu. Deliberately not one of the modal hosts
        // below: those make their background inert, which is right for a dialog
        // and wrong for a menu, which dismisses on an outside press while the
        // page behind it stays live.
        //
        // `enz_point_context_menu_host`: the trigger-anchored host cannot
        // express this, because it resolves its anchor from its own child's
        // layout and a row inside a grouped, scrolled sidebar is not a static
        // child it can measure. The point-anchored host takes exactly the
        // anchor `enz_context_target` hit-tested, and the bare
        // `enz_positioned_overlay` is the placement primitive beneath it.
        //
        // Kept from the bare layer: the viewport clamp against the surface's
        // MEASURED size, and the consumed dismissing press so the click that
        // closes the menu does not also select the row it landed on.
        //
        // Gained: focus restoration to the invoking row, the routable
        // `ShowContextMenu` AT node, and Escape claimed by the menu itself
        // rather than by hand-maintained precedence in the key handler.
        //
        // Mounted INSIDE the modal hosts (they wrap it), so a menu can never
        // float over an open dialog.
        //
        // Mount the host UNCONDITIONALLY through the optional entry
        // point. Previously the closed frame returned `page` and the open
        // frame wrapped it in `enz_point_context_menu_host`, which changed
        // the widget-tree tag above the page on every open/close cycle —
        // iced then discarded the state tree below it (sidebar scroll
        // offset, focus). `enz_optional_point_context_menu_host` keeps the
        // tree shape stable frame to frame; the `None` arm is a
        // layout-transparent pass-through (its dormant model and closed
        // state make `is_available` false, so no right-press or Escape is
        // claimed, no `ShowContextMenu` node is advertised, and no menu
        // surface reaches the frame).
        let base: Element<'_, Message> = {
            let menu = self
                .context_menu
                .as_ref()
                .map(|open| (&open.model, &open.state));
            let mut menu_rows = enzui_widgets::menu_row::MenuRowTokens::from_palette(
                &enzui_widgets::theme_lookup::palette(&self.theme()),
            );
            menu_rows.row_height = context_menu::MENU_ROW_HEIGHT;
            menu_rows.separator_inset = context_menu::MENU_SEPARATOR_INSET;
            let mut host =
                enz_optional_point_context_menu_host(page, menu, Message::ContextMenuEvent)
                    .menu_width(context_menu::MENU_WIDTH)
                    .menu_row_tokens(menu_rows)
                    .id_prefix("terminal-session-context-menu");
            if let Some(label) = context_menu::menu_label(self) {
                host = host.label(label);
            }
            // Focus returns here on close. Minted from the session's
            // STABLE id, not its title — titles are renameable (by this
            // very menu) and not unique between tabs. The builder is inert
            // while the host is closed, so no `if` is needed around it.
            if let Some(invoker) = context_menu::menu_invoker_id(self) {
                host = host.invoker(invoker);
            }
            host.into_element()
        };

        // Only ONE modal host at a time — either the SSH manager or the
        // settings dialog. The SSH manager takes priority when both are open
        // (which shouldn't happen in practice since opening one closes the
        // other).
        let dialog: Option<Element<'_, Message>>;
        let on_event: Box<dyn Fn(ModalHostEvent) -> Message + '_>;
        let label: &str;
        // While the retained settings surface is closing, the logical
        // `settings_open` flag is already false but the framework's exit
        // motion still needs the outgoing element rebuilt for one more paint.
        // `settings_surface::placement` follows the same adaptive resolver so
        // a sheet slides down to the bottom edge while a dialog scales in
        // place; `presentation_open(false)` withdraws focus/AT and makes the
        // background inert in the same frame the exit begins.
        let settings_active = self.settings_open || settings_surface::is_closing(self);
        let settings_placement = settings_surface::placement(self);

        if self.ssh_manager_open {
            dialog = Some(self.ssh_manager_dialog());
            if let Some(form) = self.ssh_form.as_ref() {
                on_event =
                    Box::new(|event| Message::SshFormEvent(FormDialogEvent::from_host(event)));
                label = if form.editing_id.is_some() {
                    crate::i18n::label("Edit SSH host")
                } else {
                    crate::i18n::label("Add connection")
                };
            } else {
                on_event = Box::new(Message::SshManagerModalEvent);
                label = SSH_MANAGER_TITLE;
            }
        } else if settings_active {
            dialog = Some(settings_surface::settings_dialog(self));
            on_event = Box::new(Message::SettingsModalEvent);
            label = crate::i18n::label(SETTINGS_TITLE);
        } else {
            dialog = None;
            on_event = Box::new(Message::SettingsModalEvent);
            label = crate::i18n::label(SETTINGS_TITLE);
        }
        let mut host = enz_modal_host(base, dialog, on_event).label(label);
        if settings_active && !self.ssh_manager_open {
            host = host.placement(settings_placement);
        }
        if self.ssh_manager_open {
            if let Some(lifecycle) = self.ssh_manager_focus.as_ref() {
                host = host.scoped_to(lifecycle);
            }
        } else if self.settings_open
            && let Some(lifecycle) = self.settings_focus.as_ref()
        {
            host = host.scoped_to(lifecycle);
        }
        if settings_active && !self.ssh_manager_open {
            // Withdraw the outgoing settings subtree from input/focus/AT in
            // the same frame the exit begins while the pixels still animate.
            host = host.presentation_open(self.settings_open);
            if let Some(key) = settings_surface::presentation_key(self) {
                // Generation-keyed: a settlement published by the surface this
                // frame painted cannot retire a settings generation opened
                // after it (rapid reopen).
                host = host.on_exit_settled_with(move || Message::SettingsExitSettled(key));
            }
        }
        let modal_base: Element<'_, Message> = host.into();

        // The destructive SSH-host confirmation renders only when its
        // pending flag is raised and stacks above the manager that raised it.
        // The stack root and child 0 stay stable; the confirmation is appended
        // as child 1 only while present, so no inert placeholder is retained.
        let layers = if self.pending_delete_ssh_host.is_some() {
            vec![
                modal_base,
                self.confirm_overlay(
                    crate::i18n::label(CONFIRM_DELETE_SSH_TITLE),
                    crate::i18n::label(CONFIRM_DELETE_SSH_BODY),
                    crate::i18n::label("Delete"),
                    Message::ConfirmDeleteSshHostEvent,
                ),
            ]
        } else {
            // Child 0 remains the modal host on both sides of the confirmation
            // edge, so its state is stable without retaining an inert full-size
            // placeholder in every normal frame.
            vec![modal_base]
        };
        let presentation: Element<'_, Message> = iced::widget::stack(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        // Toast feedback is outside the modal/context chain. A modal
        // makes its background inert, including its AccessKit subtree; placing
        // the host here keeps "Command copied" both visible and announced if a
        // dialog happens to open before its dwell expires.
        // The terminal paints content to its bottom edge (no bottom nav or
        // status strip), so both float flush with the window's bottom.
        let transient = (!self.toasts.is_empty()).then(|| self.toasts.view().map(Message::Toast));
        self.notice_layer(self.main_window, presentation, transient)
    }
}
