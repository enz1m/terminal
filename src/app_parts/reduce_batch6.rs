//! Terminal reducer batch 6: link and jump arms.

use super::*;

impl App {
    pub(crate) fn reduce6_uri_portal_wake_0(&mut self, wake: link_open::UriWake) -> Task<Message> {
        let origin = self
            .pending_links
            .get(wake.owner())
            .map(|link| link.origin.clone());
        if let link_open::UriWakeVerdict::Finished(result) = self.uri_portal.pump(&wake)
            && let Some(origin) = origin
        {
            self.finish_owned_link(origin, result);
        }

        Task::none()
    }
    pub(crate) fn reduce6_jump_to_latest_1(&mut self) -> Task<Message> {
        self.clear_find_scroll_override();
        self.wheel_row_residue = 0.0;
        if let Some(pane) = self.active_pane_mut() {
            pane.scroll.jump_to_tail();
        }

        Task::none()
    }
}
