//! Terminal reducer batch 22: shortcut-edit arms.

use super::*;

impl App {
    pub(crate) fn reduce22_edit_shortcut_0(
        &mut self,
        event: EditableShortcutsEvent,
    ) -> Task<Message> {
        match event {
            EditableShortcutsEvent::Capture { action, event } => {
                use enzui_widgets::key_capture_chip::KeyCapture;
                match event {
                    KeyCapture::Started => {
                        self.shortcut_armed = Some(action);
                        self.shortcut_feedback = None;
                    }
                    KeyCapture::Candidate(candidate) => {
                        match self.rebind_shortcut(&action, candidate) {
                            Ok(()) => {
                                self.shortcut_armed = None;
                                self.shortcut_feedback = None;
                                return self.save_preferences();
                            }
                            Err(refusal) => {
                                self.shortcut_armed = Some(action.clone());
                                self.shortcut_feedback = Some((action, refusal.message()));
                            }
                        }
                    }
                    KeyCapture::Cleared => {
                        self.shortcut_armed = Some(action.clone());
                        self.shortcut_feedback = Some((
                                action,
                                "A routed terminal action needs a shortcut; use Reset to restore its default."
                                    .into(),
                            ));
                    }
                    KeyCapture::Cancelled => {
                        self.shortcut_armed = None;
                        self.shortcut_feedback = None;
                    }
                }
            }
            EditableShortcutsEvent::Reset { action } => {
                self.shortcut_armed = None;
                self.shortcut_feedback = None;
                if self.shortcuts.reset(&action) {
                    return self.save_preferences();
                }
            }
            EditableShortcutsEvent::ResetAll => {
                self.shortcut_armed = None;
                self.shortcut_feedback = None;
                if self.shortcuts.reset_all() {
                    return self.save_preferences();
                }
            }
        }

        Task::none()
    }
}
