//! Terminal reducer batch 35: SSH form path arms.

use super::*;

impl App {
    #[cfg(test)]
    pub(crate) fn reduce35_ssh_form_default_path_edited_0(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_DEFAULT_PATH, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce35_ssh_form_notes_edited_1(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_NOTES, v)
    }
    pub(crate) fn reduce35_redock_session_2(&mut self, session: String) -> Task<Message> {
        self.set_session_detached(&session, false)
    }
    pub(crate) fn reduce35_settings_modal_event_3(
        &mut self,
        event: ModalHostEvent,
    ) -> Task<Message> {
        let task = self.reduce_settings_host(event);
        settings_surface::sync(self);
        task
    }
    pub(crate) fn reduce35_confirm_host_key_4(
        &mut self,
        origin: operation_origin::OperationOrigin,
    ) -> Task<Message> {
        self.confirm_host_key(&origin)
    }
}
