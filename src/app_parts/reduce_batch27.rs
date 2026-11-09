//! Terminal reducer batch 27: group, paste and SSH-manager arms.

use super::*;

impl App {
    pub(crate) fn reduce27_ungroup_session_0(
        &mut self,
        group_id: String,
        member: usize,
    ) -> Task<Message> {
        if let Some(idx) = self.sessions.iter().position(|s| s.id == group_id) {
            self.apply_ungroup(idx, member);
        }

        Task::none()
    }
    pub(crate) fn reduce27_session_dropped_1(
        &mut self,
        request: enzui_core::drag::ItemDropRequest,
    ) -> Task<Message> {
        let active_id = self.sessions.get(self.active).map(|s| s.id.clone());
        if let Some(active_id) = active_id
            && let Some((source, target, edge, pane)) =
                pane_group::resolve_drop(&request, &active_id)
        {
            // Content-edge and tab-on-tab drops share the same model
            // mutation; only their target/edge resolution differs.
            self.apply_group_into(&target, &source, edge, pane.as_deref());
        }

        Task::none()
    }
    pub(crate) fn reduce27_paste_history_command_2(&mut self, command: String) -> Task<Message> {
        // The overlay is a sibling of the
        // terminal pane and takes height from it, so writing while it
        // is still open would put the echoed command on a line the
        // user cannot see.
        self.history_open = false;
        if command.is_empty() {
            // Nothing to send. Still closed — the button did its
            // visible half — but no empty bracketed-paste envelope is
            // pushed at the shell.
            return Task::none();
        }
        // The command is put on the input line, NOT executed: no
        // trailing newline. A history entry is a record of what ran,
        // not a promise that running it again now is safe (`rm -rf`,
        // a command from another directory), so the user reviews and
        // presses Enter themselves.
        //
        // Shell integration reports the command text, so it is
        // untrusted and goes through the same normalising `paste_bytes`
        // envelope the clipboard path uses.
        match escape_policy::normalized_paste(&command) {
            Ok(payload) if payload.is_empty() => {}
            Ok(payload) => {
                let bytes = paste_bytes(&payload, self.active_bracketed_paste());
                if let Some(id) = self.active_pane_id() {
                    self.exclude_pinned_input(&id);
                }
                self.write_active_pty(&bytes);
            }
            Err(refusal) => {
                self.pane_feedback = Some(refusal.user_summary().to_owned());
            }
        }
        // Chrome had focus (the button was just pressed), so hand the
        // keyboard back to the terminal or the pasted line cannot be
        // edited or submitted.
        self.release_input_focus()
    }
    pub(crate) fn reduce27_open_ssh_manager_3(&mut self) -> Task<Message> {
        self.ssh_manager_open = true;
        self.ssh_form = None;

        Task::none()
    }
    pub(crate) fn reduce27_close_ssh_manager_4(&mut self) -> Task<Message> {
        let focus = self.dismiss_ssh_manager_focus(enzui_core::modal::DismissCause::CloseButton);
        // Walking away abandons any keyring wait started by the form.
        Task::batch([focus, self.update(Message::CancelSshCredentialOps)])
    }
    pub(crate) fn reduce27_cancel_ssh_credential_ops_5(&mut self) -> Task<Message> {
        self.cancel_credential_operations();
        if self.credential_mutations.is_empty() {
            self.ssh_credential_notice = None;
        }

        Task::none()
    }
}
