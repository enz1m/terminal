//! Terminal reducer batch 30: SSH connect arms.

use super::*;

impl App {
    pub(crate) fn reduce30_connect_ssh_host_0(&mut self, idx: usize) -> Task<Message> {
        // The password is retrieved before the child is spawned so the
        // secret can ride the sealed `memfd` sshpass channel rather
        // than a plaintext PTY write into a live session. A password
        // or passphrase host defers its spawn until the keyring
        // answers; an agent host has no secret and spawns at once.
        let Some(host) = self.ssh_hosts.get(idx).cloned() else {
            return Task::none();
        };
        self.retire_connection_requests(&host.id);
        match Self::secret_slot_for(&host.auth) {
            Some(slot) => {
                let owner = host.id.clone();
                let token = self.next_ssh_credential_token();
                let Some((origin, fut)) = self.load_secret_op(owner.clone(), slot, token) else {
                    return Task::none();
                };
                self.pending_ssh_connections.push(PendingSshConnection {
                    origin,
                    token,
                    owner,
                    slot,
                    host,
                });
                self.ssh_manager_open = false;
                self.ssh_form = None;
                self.spawn_credential(fut)
            }
            None => {
                // Agent auth: no secret to retrieve, spawn now.
                self.connect_ssh_record(host, None)
            }
        }
    }
    pub(crate) fn reduce30_connect_openssh_host_1(
        &mut self,
        idx: usize,
        generation: operation_origin::RequestGeneration,
    ) -> Task<Message> {
        self.choose_openssh_host(idx, generation)
    }
    pub(crate) fn reduce30_reload_openssh_config_2(&mut self) -> Task<Message> {
        self.begin_openssh_reload(ssh_discovery_app::Intent::Refresh)
    }
}
