//! Terminal reducer batch 32: credential-result arms.

use super::*;

impl App {
    pub(crate) fn reduce32_ssh_credential_loaded_0(
        &mut self,
        origin: operation_origin::OperationOrigin,
        owner: String,
        slot: CredentialSlot,
        token: u64,
        result: SshCredentialResult,
    ) -> Task<Message> {
        if !self.finish_credential_op(&origin, &owner, slot, CredentialIntent::Load) {
            return Task::none();
        }
        self.ssh_credential_notice = result.notice(CredentialIntent::Load, slot);
        match self.resume_pending_pane_connection(&origin, result.clone()) {
            Some(task) => task,
            None => self.resume_pending_ssh_connection(&origin, &owner, slot, token, result),
        }
    }
    pub(crate) fn reduce32_ssh_credential_deleted_1(
        &mut self,
        origin: operation_origin::OperationOrigin,
        owner: String,
        slot: CredentialSlot,
        result: SshCredentialResult,
    ) -> Task<Message> {
        if self.credential_mutation_store.is_some() {
            return self.finish_owner_mutation(&origin, &owner, slot, result);
        }
        if !self.finish_credential_op(&origin, &owner, slot, CredentialIntent::Forget) {
            return Task::none();
        }
        if let Some(notice) = result.notice(CredentialIntent::Forget, slot) {
            self.ssh_credential_notice = Some(notice);
        }
        self.resolve_pending_deletion(&origin, &owner, &result)
    }
    pub(crate) fn reduce32_ssh_credential_reconciled_2(
        &mut self,
        origin: operation_origin::OperationOrigin,
        owner: String,
        slot: CredentialSlot,
        retry_mismatch: bool,
        result: SshCredentialResult,
    ) -> Task<Message> {
        self.finish_credential_reconciliation(&origin, &owner, slot, retry_mismatch, result)
    }
}
