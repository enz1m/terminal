//! Terminal update root.

use super::*;

impl App {
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        crate::i18n::localizer()
            .clone()
            .scope(|| self.localized_update(message))
    }

    pub(crate) fn localized_update(&mut self, message: Message) -> Task<Message> {
        // The crash breadcrumb comes first, so a message an early return
        // refuses or reroutes is still recorded (payload-free kind only).
        enzui_core::crash_report::observe(&message);
        let task = match message {
            Message::ActivityWindowTagged(window, tag, queued) => {
                self.shell_activity.confirm_tag(window, &tag, queued);
                Task::none()
            }
            Message::ActivityPrune(deadline) => {
                self.shell_activity.prune(deadline);
                Task::none()
            }
            Message::InputFocusEdge(window, edge) => {
                self.apply_focus_edge(window, edge);
                Task::none()
            }
            Message::StampedInput(window, stamp, message) => {
                self.reduce_stamped_input(window, stamp, *message)
            }
            message => self.update_message(message),
        };
        self.sync_shell_activity();
        self.bound_scrollback_expansion();
        self.prune_dismissed_notices();
        let task = Task::batch([task, self.look_up_git_branches()]);
        // While every pane's view is written by its reader, the drain needs no
        // display-rate cap.
        crate::pty::set_relaxed(
            !self.ptys.is_empty() && self.ptys.values().all(pty::PtySession::is_live),
        );
        if self.typeahead.is_empty() {
            task
        } else {
            Task::batch([task, self.drain_typeahead()])
        }
    }

    pub(crate) fn update_message(&mut self, message: Message) -> Task<Message> {
        if self.input_context.is_some() {
            return self.update_inner(message);
        }
        if matches!(
            message,
            Message::PanePointer(..)
                | Message::PaneInput(..)
                | Message::InputMenuEvent(..)
                | Message::RequestCopy
                | Message::GridCellPressed(_)
                | Message::GridCellDragged(_)
                | Message::GridCellReleased(_)
                | Message::FindNext
                | Message::FindPrev
                | Message::FindQueryChanged(_)
        ) && self
            .ptys
            .values()
            .any(pty::PtySession::coordinates_invalidated)
        {
            self.drain_ptys();
            self.reconcile_operation_origins();
        }
        if !self.accepts_operation_completion(&message) {
            return Task::none();
        }
        #[cfg(feature = "native-invalidation-acceptance")]
        if matches!(message, Message::NativeAcceptancePoll) {
            return self.native_acceptance_poll();
        }
        self.update_settled(message)
    }
}
