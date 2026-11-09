//! Terminal PTY drain method.

use super::*;

impl App {
    pub(crate) fn drain_ptys(&mut self) {
        // The executor rings the same bell after each round, so this is the
        // edge-triggered route that discharges settled teardown tickets.
        self.poll_teardown_debts();
        self.sync_pinned_capture();
        for si in 0..self.sessions.len() {
            let pane_ids: Vec<(usize, String)> = self.sessions[si]
                .panes
                .iter()
                .enumerate()
                .map(|(pi, pane)| (pi, pane.id.clone()))
                .collect();
            let mut exited: Vec<(usize, String, pty::ChildExit, bool)> = Vec::new();
            for (pi, pid) in pane_ids {
                let Some(pty) = self.ptys.get(&pid) else {
                    continue;
                };
                let Some(pty::PtyUpdate {
                    snapshot,
                    effects,
                    alt_screen: alt,
                    cursor_hidden,
                    application_cursor_keys,
                    bracketed_paste,
                    mouse_protocol,
                }) = pty.take_update()
                else {
                    continue;
                };
                let child_exit = pty.take_child_exit();
                let secret_delivery = pty.secret_delivery();
                if effects.coordinates_invalidated {
                    self.invalidate_pane_coordinates(&pid);
                }

                let mut switched = false;
                if let Some(pane) = self.sessions[si].panes.get_mut(pi) {
                    pane.application_cursor_keys = application_cursor_keys;
                    pane.cursor_hidden = cursor_hidden;
                    pane.bracketed_paste = bracketed_paste;
                    pane.mouse_protocol = mouse_protocol;
                    switched = pane.set_alt_screen(alt);
                }
                if switched {
                    self.wheel_row_residue = 0.0;
                    // Detached state is independent, so it must cross the
                    // screen edge independently too. Reset both the retained
                    // primary viewport and fractional residue: alternate-screen
                    // rows are not scrollback, and leaving a TUI must not spend
                    // half a wheel notch banked inside it.
                    let session_id = self.sessions[si].id.clone();
                    for detached in self
                        .detached_windows
                        .values_mut()
                        .filter(|detached| detached.session == session_id)
                    {
                        if let Some(viewport) = detached.viewports.get_mut(&pid) {
                            viewport.scroll.jump_to_tail();
                            viewport.wheel_row_residue = 0.0;
                        }
                    }
                    // Crossing the alternate-screen edge changes the geometry.
                    // An ordinary shell is inset; a TUI is full bleed, so the
                    // same window holds more columns and rows the moment `btop`
                    // starts. Without this resize the shell keeps the inset
                    // geometry and lays its right-hand column out past the pane
                    // edge.
                    self.resize_session_ptys(si);
                }
                Self::apply_transcript_snapshot(
                    &mut self.sessions,
                    &mut self.find,
                    self.active,
                    si,
                    pi,
                    snapshot,
                );
                self.apply_terminal_effects(si, pi, effects);
                if let Some(exit) = child_exit {
                    exited.push((pi, pid.clone(), exit, secret_delivery));
                }
            }
            for (pi, pid, exit, secret_delivery) in exited {
                if self.sessions[si].pinned
                    && let Some(output) = self.ptys.get(&pid).map(pty::PtySession::pinned_output)
                {
                    self.finished_outputs.insert(pid.clone(), output);
                }
                let message = format!("{SHELL_EXITED_PREFIX}{}", exit.detail);
                let successful = exit.code == Some(0);
                let status = if successful {
                    SessionStatus::Idle
                } else {
                    SessionStatus::Error
                };
                let tone = if successful {
                    model::LineTone::Default
                } else {
                    model::LineTone::Error
                };
                if let Some(pane) = self.sessions[si].panes.get_mut(pi) {
                    pane.status = status;
                    pane.transcript
                        .push(TranscriptRow::new(TranscriptLine::monochrome(
                            format!("{pid}-exit"),
                            message.clone(),
                            tone,
                            false,
                        )));
                }
                if self.sessions[si].panes.len() == 1 {
                    self.sessions[si].status = status;
                    self.sessions[si].connection_error = (!successful).then_some(message);
                    self.invalidate_sidebar();
                }
                let owner_id = self.sessions[si].id.clone();
                self.retire_pty(&owner_id, &pid);
                self.note_host_trust_exit(&owner_id, &pid, &exit, secret_delivery);
            }
        }
        self.reconcile_operation_origins();
        self.reflow_if_grid_area_changed();
    }
}
