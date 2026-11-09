//! Typing behind a starved PTY writer.
//!
//! Each key is its own write, and the queue holds
//! [`pty::MAX_INPUT_QUEUE_CHUNKS`] chunks. A writer thread starved for a few
//! hundred milliseconds used to refuse ordinary typing at the ninth key. Typed
//! input now joins the newest unstarted typed chunk, so the bound is bytes,
//! not keys; pastes keep a chunk each and keep their refusal.

use super::*;
use iced::keyboard::{self, Modifiers};

fn press(app: &mut App, text: &str) {
    dispatch(
        app,
        Message::KeyPressed {
            key: keyboard::Key::Character(text.into()),
            mods: Modifiers::default(),
            captured: false,
        },
    );
}
