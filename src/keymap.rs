//! Legacy xterm keyboard encoding. Enhanced keyboard protocols are not advertised.

use iced::keyboard::Modifiers;
use iced::keyboard::key::{Key, Named};

pub(crate) fn terminal_input_key(
    key: &Key,
    modified_key: &Key,
    text: Option<&str>,
    mods: Modifiers,
) -> Key {
    if matches!(key, Key::Named(_)) || mods.control() {
        return key.clone();
    }
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        return Key::Character(text.into());
    }
    modified_key.clone()
}

/// Encodes an unclaimed key in normal cursor mode; unsupported chords produce no input.
#[cfg(test)]
pub(crate) fn key_to_pty_bytes(key: &Key, mods: Modifiers) -> Option<Vec<u8>> {
    key_to_pty_bytes_with_mode(key, mods, false)
}

pub(crate) fn key_to_pty_bytes_with_mode(
    key: &Key,
    mods: Modifiers,
    application: bool,
) -> Option<Vec<u8>> {
    if mods.logo() {
        return None;
    }
    match key {
        Key::Character(text) => character_to_bytes(text, mods),
        Key::Named(named) => named_to_bytes(*named, mods, application),
        Key::Unidentified => None,
    }
}

fn character_to_bytes(text: &str, mods: Modifiers) -> Option<Vec<u8>> {
    if text.is_empty() {
        return None;
    }
    let bytes = if mods.control() {
        let mut chars = text.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let code = match c.to_ascii_lowercase() {
            c @ 'a'..='z' => u8::try_from(u32::from(c)).ok()? - b'a' + 1,
            ' ' | '@' | '2' => 0,
            '[' | '3' => 0x1b,
            '\\' | '4' => 0x1c,
            ']' | '5' => 0x1d,
            '^' | '6' => 0x1e,
            '_' | '7' => 0x1f,
            '?' | '8' => 0x7f,
            _ => return None,
        };
        vec![code]
    } else {
        text.as_bytes().to_vec()
    };
    Some(meta(bytes, mods))
}

fn meta(mut bytes: Vec<u8>, mods: Modifiers) -> Vec<u8> {
    if mods.alt() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Encodes unmodified arrows for DECCKM, including alternate-screen wheel forwarding.
pub(crate) fn cursor_key_bytes(named: Named, application: bool) -> Option<Vec<u8>> {
    match named {
        Named::ArrowUp | Named::ArrowDown | Named::ArrowLeft | Named::ArrowRight => {
            named_to_bytes(named, Modifiers::default(), application)
        }
        _ => None,
    }
}

fn named_to_bytes(named: Named, mods: Modifiers, application: bool) -> Option<Vec<u8>> {
    let parameter =
        1 + u8::from(mods.shift()) + 2 * u8::from(mods.alt()) + 4 * u8::from(mods.control());
    let final_byte = match named {
        Named::ArrowUp => Some('A'),
        Named::ArrowDown => Some('B'),
        Named::ArrowRight => Some('C'),
        Named::ArrowLeft => Some('D'),
        Named::Home => Some('H'),
        Named::End => Some('F'),
        Named::F1 => Some('P'),
        Named::F2 => Some('Q'),
        Named::F3 => Some('R'),
        Named::F4 => Some('S'),
        _ => None,
    };
    if let Some(last) = final_byte {
        return Some(if parameter != 1 {
            format!("\x1b[1;{parameter}{last}").into_bytes()
        } else if application || matches!(named, Named::F1 | Named::F2 | Named::F3 | Named::F4) {
            format!("\x1bO{last}").into_bytes()
        } else {
            format!("\x1b[{last}").into_bytes()
        });
    }
    let number = match named {
        Named::Insert => Some(2),
        Named::Delete => Some(3),
        Named::PageUp => Some(5),
        Named::PageDown => Some(6),
        Named::F5 => Some(15),
        Named::F6 => Some(17),
        Named::F7 => Some(18),
        Named::F8 => Some(19),
        Named::F9 => Some(20),
        Named::F10 => Some(21),
        Named::F11 => Some(23),
        Named::F12 => Some(24),
        _ => None,
    };
    if let Some(number) = number {
        return Some(
            if parameter == 1 {
                format!("\x1b[{number}~")
            } else {
                format!("\x1b[{number};{parameter}~")
            }
            .into_bytes(),
        );
    }
    let bytes = match named {
        Named::Enter => vec![b'\r'],
        Named::Tab if mods.shift() => b"\x1b[Z".to_vec(),
        Named::Tab => vec![b'\t'],
        Named::Backspace => vec![if mods.control() { 0x08 } else { 0x7f }],
        Named::Escape => vec![0x1b],
        Named::Space => return character_to_bytes(" ", mods),
        _ => return None,
    };
    Some(meta(bytes, mods))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::Key;

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    fn event_bytes(
        key: &Key,
        modified_key: &Key,
        text: Option<&str>,
        mods: Modifiers,
    ) -> Option<Vec<u8>> {
        key_to_pty_bytes(&terminal_input_key(key, modified_key, text, mods), mods)
    }
}
