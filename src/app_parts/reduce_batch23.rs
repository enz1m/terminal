//! Terminal reducer batch 23: save-completion arms.

use super::*;

impl App {
    pub(crate) fn reduce23_ssh_host_save_finished_0(
        &mut self,
        generation: operation_origin::RequestGeneration,
        outcome: ssh_host_save::Outcome,
    ) -> Task<Message> {
        self.finish_ssh_host_save(generation, outcome)
    }
    pub(crate) fn reduce23_preferences_worker_finished_1(
        &mut self,
        generation: operation_origin::RequestGeneration,
        result: Result<(), PersistError>,
    ) -> Task<Message> {
        if generation != self.preferences_save_generation || !generation.is_live() {
            return Task::none();
        }
        self.update_inner(Message::PreferencesSaved(result))
    }
    pub(crate) fn reduce23_preferences_saved_2(
        &mut self,
        result: Result<(), PersistError>,
    ) -> Task<Message> {
        self.preferences_write_failed = result.is_err();
        self.ssh_save_error = result
            .as_ref()
            .err()
            .map(enzui_core::persist::PersistError::user_summary);
        self.shortcuts.finish_save(result);

        Task::none()
    }
}
