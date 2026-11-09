//! Terminal active-pane methods.

use super::*;

impl App {
    pub(crate) fn active_pane(&self) -> Option<&model::Pane> {
        let session = self.sessions.get(self.active)?;
        session.panes.get(session.active_pane)
    }

    pub(crate) fn active_pane_mut(&mut self) -> Option<&mut model::Pane> {
        let session = self.sessions.get_mut(self.active)?;
        let index = session.active_pane;
        session.panes.get_mut(index)
    }

    /// Whether the active pane's shell is on the ALTERNATE screen — the gate
    /// that keeps a TUI's wheel and paging out of the primary scrollback.
    pub(crate) fn active_pane_is_alt_screen(&self) -> bool {
        self.active_pane().is_some_and(|p| p.alt_screen)
    }

    /// The active pane's wheel-routing state: which screen it is on and which
    /// cursor-key encoding its program is listening for.
    pub(crate) fn active_wheel_target(&self) -> scrollback::WheelTarget {
        match self.active_pane() {
            Some(pane) => scrollback::WheelTarget {
                alt_screen: pane.alt_screen,
                application_cursor_keys: pane.application_cursor_keys,
                mouse_protocol: pane.mouse_protocol,
                mouse_shift: self.mods.shift(),
                mouse_cell: self.mouse_last_cell,
            },
            None => scrollback::WheelTarget::default(),
        }
    }

    /// The `ScrollGeometry` for the pane the user is looking at.
    pub(crate) fn active_scroll_geometry(&self) -> scrollback::ScrollGeometry {
        let total = self.active_pane().map_or(0, |p| p.transcript.len());
        self.scroll_geometry(total, self.geometry())
    }
}
