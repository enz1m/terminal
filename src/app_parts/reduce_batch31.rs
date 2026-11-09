//! Terminal reducer batch 31: OpenSSH discovery arms.

use super::*;

impl App {
    pub(crate) fn reduce31_openssh_discovery_finished_0(
        &mut self,
        completion: ssh_discovery::Completion,
    ) -> Task<Message> {
        self.finish_openssh_discovery(completion)
    }
    #[cfg(test)]
    pub(crate) fn reduce31_openssh_config_loaded_1(
        &mut self,
        result: Result<Vec<ssh_config::OpenSshHost>, ssh_config::OpenSshConfigError>,
    ) -> Task<Message> {
        self.openssh_discovery.injected(result.is_ok());
        self.apply_openssh_config_result(result);

        Task::none()
    }
    pub(crate) fn reduce31_ssh_credential_stored_2(
        &mut self,
        origin: operation_origin::OperationOrigin,
        owner: String,
        slot: CredentialSlot,
        result: SshCredentialResult,
    ) -> Task<Message> {
        if self.credential_mutation_store.is_some() {
            return self.finish_owner_mutation(&origin, &owner, slot, result);
        }
        if !self.finish_credential_op(&origin, &owner, slot, CredentialIntent::Remember) {
            return Task::none();
        }
        self.ssh_credential_notice = result.notice(CredentialIntent::Remember, slot);

        Task::none()
    }
}
