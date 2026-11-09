//! Terminal reducer batch 34: SSH form secret arms.

use super::*;

impl App {
    #[cfg(test)]
    pub(crate) fn reduce34_ssh_form_password_edited_0(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_PASSWORD, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce34_ssh_form_toggle_password_revealed_1(&mut self) -> Task<Message> {
        self.update(Message::SshFormEvent(FormDialogEvent::RevealToggled(
            SSH_FORM_PASSWORD.into(),
        )))
    }
    #[cfg(test)]
    pub(crate) fn reduce34_ssh_form_key_path_edited_2(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_KEY_PATH, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce34_ssh_form_passphrase_edited_3(&mut self, v: String) -> Task<Message> {
        self.edit_ssh_form_text(SSH_FORM_PASSPHRASE, v)
    }
    #[cfg(test)]
    pub(crate) fn reduce34_ssh_form_toggle_passphrase_revealed_4(&mut self) -> Task<Message> {
        self.update(Message::SshFormEvent(FormDialogEvent::RevealToggled(
            SSH_FORM_PASSPHRASE.into(),
        )))
    }
}
