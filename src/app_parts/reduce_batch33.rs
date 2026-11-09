//! Terminal reducer batch 33: SSH form edit arms.

use super::*;

impl App {
    #[cfg(test)]
    pub(crate) fn reduce33_ssh_form_name_edited_0(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_NAME, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce33_ssh_form_dest_edited_1(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_DESTINATION, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce33_ssh_form_port_edited_2(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_PORT, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce33_ssh_form_username_edited_3(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_USERNAME, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce33_ssh_form_auth_mode_selected_4(&mut self, idx: usize) -> Task<Message> {
        if let Some(kind) = model::AUTH_MODES.get(idx) {
            return self.update(Message::SshFormEvent(FormDialogEvent::Edited(
                SSH_FORM_AUTH_MODE.into(),
                FieldValue::Choice(Some(kind.form_value().to_owned())),
            )));
        }

        Task::none()
    }
}
