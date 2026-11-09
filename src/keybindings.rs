// NO module-level `#![allow(dead_code)]`. A blanket allow hides a
// feature that is built, tested and unreachable, and this module WAS that
// feature: a 32-action catalog with a dispatcher (`action_for_chord`) that no
// key event ever reached, while `main.rs` hard-coded four Ctrl+letter chords
// inline. See `chord_from_iced_key`.
//! Configurable keyboard shortcuts.
//!
//! This module is deliberately dependency-free (only `std`), so it stays
//! purely testable and the reducer/UI layers can just look up chords by
//! action id.
//!
//! ## Terms
//!
//! - **Action id** — a stable string like `"tab.new"` used both by the
//!   settings UI and by the FocusScope-side dispatcher to identify a
//!   shortcut binding.
//! - **Chord** — a normalised keystroke string of the form
//!   `"[Ctrl+][Shift+][Alt+]KEY"` where KEY is either a single uppercase
//!   ASCII letter, a decimal digit, or a named key like `"Tab"`,
//!   `"PageUp"`, `"PageDown"`, `"Enter"`, `"Escape"`, `"Left"`, `"Right"`,
//!   `"Up"`, `"Down"`, `"Home"`, `"End"`, `"F1"`..`"F12"`, `"?"`,
//!   `"Space"`. Modifiers are always in the canonical order
//!   `Ctrl < Shift < Alt` regardless of how the user typed them.
//!
//! ## Design
//!
//! * `default_bindings()` returns the shipped default chord for every
//!   catalog action. `Settings::keybindings` stores ONLY OVERRIDES —
//!   entries that differ from the default. This keeps `settings.json`
//!   minimal and lets a new default be shipped in a future release
//!   without every user's file freezing an old chord.
//! * `resolved_bindings(overrides)` merges overrides on top of defaults
//!   and returns the final action_id -> chord table used at runtime.
//! * `parse_chord()` accepts many surface forms (Ctrl-T, C-T, ctrl+t,
//!   Ctrl+Shift+T, CTRL+SHIFT+T, PageDown, PgDn, etc.) and canonicalises
//!   them. Callers should always store the canonical form (from
//!   `format_chord()` or the `Chord::to_string()`) so the on-disk shape
//!   is stable.
//! * `conflicts_in(bindings)` returns any (chord -> `Vec<action>`) with
//!   two or more actions mapped to the same chord. The Settings UI uses
//!   this to warn the user; the runtime dispatcher just picks the first
//!   catalog-order match.

#[cfg(test)]
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Catalog of bindable actions.
// ---------------------------------------------------------------------------

/// Every bindable action shipped by the app. `id` is the stable key used
/// in `settings.json`; `label` and `category` power the Settings UI.
///
/// Kept as `&'static str` fields so `ActionEntry` is `Copy` — the
/// Settings tab iterates over `ALL_ACTIONS` many times per render and
/// avoiding a clone keeps things cheap.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActionEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub default_chord: &'static str,
}

/// Adopt the framework's `ShortcutCatalogRow`, so the Shortcuts
/// dialog is built through `shortcut_entries_with(&ALL_ACTIONS, resolver)`
/// instead of a hand-rolled `iter().map(ShortcutEntry::new)` map. The
/// runtime overload supplies the *resolved* chord (from
/// `settings.resolved_keybindings()`); `badge()` here is only ever the
/// **fallback** used when the resolver returns `None`, i.e. the shipped
/// `default_chord`. That is the same shape `main.rs` had inlined and it
/// preserves the rebind → dialog reflection pinned by
/// `a_rebound_shortcut_is_honoured_and_the_old_chord_stops_working`.
impl enzui_widgets::shortcut_list::ShortcutCatalogRow for ActionEntry {
    fn section(&self) -> &str {
        crate::i18n::label(self.category)
    }
    fn badge(&self) -> String {
        // Fallback only: the runtime resolver decides whether this or the
        // user's override is displayed. See `shortcut_entries_with`.
        self.default_chord.to_owned()
    }
    fn description(&self) -> &str {
        crate::i18n::label(self.label)
    }
    fn stable_key(&self) -> Option<String> {
        Some(self.id.to_owned())
    }
}

/// Full catalog of bindable actions, in stable display order (grouped
/// by category). Keeping this list ordered lets the Settings UI render
/// deterministic rows without a per-render sort.
///
/// Convention: `id` uses dotted namespaces (`tab.new`, `pane.split-h`,
/// `overlay.find`) so a future prefix-filter UX ("show only tab
/// shortcuts") is a one-line filter.
pub(crate) const ALL_ACTIONS: &[ActionEntry] = &[
    // ---- Tabs (sessions) --------------------------------------------
    ActionEntry {
        id: "tab.new",
        label: "New tab",
        category: "Tabs",
        default_chord: "Ctrl+Shift+T",
    },
    ActionEntry {
        id: "tab.close",
        label: "Close tab",
        category: "Tabs",
        default_chord: "Ctrl+Shift+W",
    },
    ActionEntry {
        id: "tab.next",
        label: "Next tab",
        category: "Tabs",
        default_chord: "Ctrl+PageDown",
    },
    ActionEntry {
        id: "tab.prev",
        label: "Previous tab",
        category: "Tabs",
        default_chord: "Ctrl+PageUp",
    },
    ActionEntry {
        id: "tab.next-alt",
        label: "Next tab (alternate)",
        category: "Tabs",
        default_chord: "Ctrl+Tab",
    },
    ActionEntry {
        id: "tab.prev-alt",
        label: "Previous tab (alternate)",
        category: "Tabs",
        default_chord: "Ctrl+Shift+Tab",
    },
    ActionEntry {
        id: "tab.goto-1",
        label: "Jump to tab 1",
        category: "Tabs",
        default_chord: "Alt+1",
    },
    ActionEntry {
        id: "tab.goto-2",
        label: "Jump to tab 2",
        category: "Tabs",
        default_chord: "Alt+2",
    },
    ActionEntry {
        id: "tab.goto-3",
        label: "Jump to tab 3",
        category: "Tabs",
        default_chord: "Alt+3",
    },
    ActionEntry {
        id: "tab.goto-4",
        label: "Jump to tab 4",
        category: "Tabs",
        default_chord: "Alt+4",
    },
    ActionEntry {
        id: "tab.goto-5",
        label: "Jump to tab 5",
        category: "Tabs",
        default_chord: "Alt+5",
    },
    ActionEntry {
        id: "tab.goto-6",
        label: "Jump to tab 6",
        category: "Tabs",
        default_chord: "Alt+6",
    },
    ActionEntry {
        id: "tab.goto-7",
        label: "Jump to tab 7",
        category: "Tabs",
        default_chord: "Alt+7",
    },
    ActionEntry {
        id: "tab.goto-8",
        label: "Jump to tab 8",
        category: "Tabs",
        default_chord: "Alt+8",
    },
    ActionEntry {
        id: "tab.goto-9",
        label: "Jump to tab 9",
        category: "Tabs",
        default_chord: "Alt+9",
    },
    ActionEntry {
        id: "tab.rename",
        label: "Rename tab",
        category: "Tabs",
        default_chord: "F2",
    },
    // ---- Panes ------------------------------------------------------
    ActionEntry {
        id: "pane.split-horizontal",
        label: "Split pane right",
        category: "Panes",
        default_chord: "Ctrl+Shift+E",
    },
    ActionEntry {
        id: "pane.split-vertical",
        label: "Split pane down",
        category: "Panes",
        default_chord: "Ctrl+Shift+O",
    },
    ActionEntry {
        id: "pane.close",
        label: "Close focused pane",
        category: "Panes",
        default_chord: "Ctrl+Shift+D",
    },
    ActionEntry {
        id: "pane.focus-left",
        label: "Focus pane left",
        category: "Panes",
        default_chord: "Ctrl+Alt+Left",
    },
    ActionEntry {
        id: "pane.focus-right",
        label: "Focus pane right",
        category: "Panes",
        default_chord: "Ctrl+Alt+Right",
    },
    ActionEntry {
        id: "pane.focus-up",
        label: "Focus pane up",
        category: "Panes",
        default_chord: "Ctrl+Alt+Up",
    },
    ActionEntry {
        id: "pane.focus-down",
        label: "Focus pane down",
        category: "Panes",
        default_chord: "Ctrl+Alt+Down",
    },
    // ---- Clipboard --------------------------------------------------
    ActionEntry {
        id: "clipboard.copy",
        label: "Copy",
        category: "Clipboard",
        default_chord: "Ctrl+Shift+C",
    },
    ActionEntry {
        id: "clipboard.paste",
        label: "Paste",
        category: "Clipboard",
        default_chord: "Ctrl+Shift+V",
    },
    // ---- Overlays / UI ---------------------------------------------
    // `overlay.palette` (Ctrl+K) DELETED. It advertised a "Command
    // palette" in the Shortcuts dialog, but no reducer ever opened one
    // (`action_message` returned `None`), so pressing Ctrl+K only did what the
    // shell already does — kill-to-end-of-line — after telling the user it
    // would do something else. A binding whose sole effect is to corrupt the
    // command line while lying about it should not ship, so the action is
    // removed from the catalog entirely until a palette exists. With it gone,
    // Ctrl+K resolves to no app action and falls through to the PTY, which is
    // the readline behaviour the user actually expects.
    // Terminal chrome uses Ctrl+Shift chords. Bare Ctrl+letter
    // belongs to the PTY: Ctrl+F is readline forward-char and vi page-down;
    // Ctrl+H is readline backward-delete-char / the traditional BS byte.
    ActionEntry {
        id: "overlay.find",
        label: "Find in terminal",
        category: "Overlays",
        default_chord: "Ctrl+Shift+F",
    },
    ActionEntry {
        id: "overlay.history",
        label: "Command history",
        category: "Overlays",
        default_chord: "Ctrl+Shift+H",
    },
    ActionEntry {
        id: "overlay.settings",
        label: "Open settings",
        category: "Overlays",
        default_chord: "Ctrl+,",
    },
    ActionEntry {
        id: "overlay.ssh",
        label: "Manage SSH hosts",
        category: "Overlays",
        default_chord: "Ctrl+Shift+S",
    },
    // Ctrl+B is readline/emacs backward-char, vi page-up, and tmux's
    // default prefix. Claiming it in a terminal emulator makes all three
    // unreachable, so the app accelerator keeps the key but adds Shift.
    ActionEntry {
        id: "ui.toggle-sidebar",
        label: "Toggle sidebar",
        category: "Overlays",
        default_chord: "Ctrl+Shift+B",
    },
];

/// Look up a catalog entry by id. `None` if the id isn't known
/// (defensive against a stale `settings.json` entry left over from a
/// removed feature).
#[cfg(test)]
pub(crate) fn find_action(id: &str) -> Option<&'static ActionEntry> {
    ALL_ACTIONS.iter().find(|a| a.id == id)
}

/// Default action_id -> chord table, freshly derived from `ALL_ACTIONS`.
/// Every action gets a chord; the app ships with zero conflicts.
#[cfg(test)]
pub(crate) fn default_bindings() -> BTreeMap<String, String> {
    ALL_ACTIONS
        .iter()
        .map(|a| (a.id.to_owned(), a.default_chord.to_owned()))
        .collect()
}

/// Merge user overrides on top of defaults. Overrides with an unknown
/// action id are dropped (defence against a stale on-disk entry).
/// Overrides whose chord fails to parse are ALSO dropped so the runtime
/// never has to defend against a malformed chord.
#[cfg(test)]
pub(crate) fn resolved_bindings(overrides: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = default_bindings();
    for (id, chord) in overrides {
        if find_action(id).is_none() {
            continue;
        }
        if let Some(parsed) = Chord::parse(chord) {
            out.insert(id.clone(), parsed.to_string());
        }
    }
    out
}

/// Reverse lookup: chord -> Vec<action_id>. Used by the conflict detector.
///
/// TEST-ONLY. The doc used to claim "used by the runtime dispatcher
/// (lookup a pressed chord)"; the runtime dispatcher is
/// [`action_for_chord`], which walks `ALL_ACTIONS` directly so it can apply
/// the catalog-order tie-break. Only [`conflicts_in`] calls this, and only
/// the tests call that (see there).
#[cfg(test)]
pub(crate) fn chord_to_actions(
    bindings: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Iterate ALL_ACTIONS to keep catalog order deterministic — a raw
    // BTreeMap iter would sort by id which mangles the "first
    // registered wins" tie-breaker.
    for a in ALL_ACTIONS {
        if let Some(chord) = bindings.get(a.id) {
            out.entry(chord.clone()).or_default().push(a.id.to_owned());
        }
    }
    out
}

/// Every chord assigned to more than one action.
///
/// TEST-ONLY. The catalog tests use this to prove the SHIPPED textual
/// defaults have no conflict. Runtime editing now goes through the framework's
/// live catalog, which refuses conflicts before applying them; this helper
/// remains the legacy catalog's default-conflict gate.
#[cfg(test)]
pub(crate) fn conflicts_in(bindings: &BTreeMap<String, String>) -> BTreeMap<String, Vec<String>> {
    chord_to_actions(bindings)
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .collect()
}

/// Convert the app's legacy textual default into the framework shortcut type
/// used by capture, conflict detection, persistence, display, and dispatch.
pub(crate) fn framework_shortcut(text: &str) -> Option<enzui_core::Shortcut> {
    use enzui_core::{Modifier, NamedKey, Shortcut, ShortcutKey};

    let chord = Chord::parse(text)?;
    let mut modifiers = Modifier::NONE;
    if chord.ctrl {
        modifiers |= Modifier::CTRL;
    }
    if chord.shift {
        modifiers |= Modifier::SHIFT;
    }
    if chord.alt {
        modifiers |= Modifier::ALT;
    }
    let named = match chord.key.as_str() {
        "Tab" => Some(NamedKey::Tab),
        "Enter" => Some(NamedKey::Enter),
        "Escape" => Some(NamedKey::Escape),
        "Space" => Some(NamedKey::Space),
        "Backspace" => Some(NamedKey::Backspace),
        "Delete" => Some(NamedKey::Delete),
        "Insert" => Some(NamedKey::Insert),
        "Home" => Some(NamedKey::Home),
        "End" => Some(NamedKey::End),
        "PageUp" => Some(NamedKey::PageUp),
        "PageDown" => Some(NamedKey::PageDown),
        "Left" => Some(NamedKey::ArrowLeft),
        "Right" => Some(NamedKey::ArrowRight),
        "Up" => Some(NamedKey::ArrowUp),
        "Down" => Some(NamedKey::ArrowDown),
        "F1" => Some(NamedKey::F1),
        "F2" => Some(NamedKey::F2),
        "F3" => Some(NamedKey::F3),
        "F4" => Some(NamedKey::F4),
        "F5" => Some(NamedKey::F5),
        "F6" => Some(NamedKey::F6),
        "F7" => Some(NamedKey::F7),
        "F8" => Some(NamedKey::F8),
        "F9" => Some(NamedKey::F9),
        "F10" => Some(NamedKey::F10),
        "F11" => Some(NamedKey::F11),
        "F12" => Some(NamedKey::F12),
        _ => None,
    };
    let key = named.map_or_else(|| ShortcutKey::from(chord.key), ShortcutKey::from);
    Some(Shortcut::new(modifiers, key))
}

/// Normalise a raw key spelling to the canonical catalog form.
/// Returns `None` for the empty string or a modifier-only spelling
/// (which shouldn't reach here, but defence is cheap).
#[path = "keybindings/chord.rs"]
mod chord;
pub(crate) use chord::Chord;

pub(crate) fn normalise_key(key: &str) -> Option<String> {
    let k = key.trim();
    if k.is_empty() {
        return None;
    }
    // Single ASCII letter → uppercase. Digits keep their spelling.
    if k.len() == 1 {
        let ch = k.chars().next().unwrap();
        if ch.is_ascii_alphabetic() {
            return Some(ch.to_ascii_uppercase().to_string());
        }
        // Map common control chars to named keys.
        match ch {
            '\n' | '\r' => return Some("Enter".to_owned()),
            '\t' => return Some("Tab".to_owned()),
            '\x1B' => return Some("Escape".to_owned()),
            '\x08' | '\x7F' => return Some("Backspace".to_owned()),
            _ => {}
        }
        // Ctrl+letter arrives as ASCII 0x01..0x1A. This path is hit when the
        // framework's `EnzKeybindingRow` builds "Ctrl+<control-char>"
        // from the raw `event.text`.
        let b = ch as u32;
        if (0x01..=0x1A).contains(&b) {
            let letter = (b'A' + (b as u8) - 1) as char;
            return Some(letter.to_string());
        }
        // Private-use named-key codepoints (U+F700..U+F7FF, the AppKit
        // function-key range). Map common ones.
        if (0xF700..=0xF7FF).contains(&b) {
            let named = match b {
                0xF72C => "PageUp",
                0xF72D => "PageDown",
                0xF729 => "Home",
                0xF72B => "End",
                0xF702 => "Left",
                0xF703 => "Right",
                0xF700 => "Up",
                0xF701 => "Down",
                0xF728 => "Delete",
                0xF727 => "Insert",
                _ if (0xF704..=0xF71B).contains(&b) => {
                    let n = b - 0xF704 + 1;
                    return Some(format!("F{n}"));
                }
                _ => "",
            };
            if !named.is_empty() {
                return Some(named.to_owned());
            }
        }
        // Symbols / digits pass through verbatim.
        return Some(ch.to_string());
    }
    // Named keys — case-insensitive with a small alias table.
    let low = k.to_ascii_lowercase();
    let canonical = match low.as_str() {
        "pageup" | "pgup" | "prior" => "PageUp",
        "pagedown" | "pgdn" | "next" => "PageDown",
        "enter" | "return" | "ret" => "Enter",
        "escape" | "esc" => "Escape",
        "space" | "spc" | "spacebar" => "Space",
        "tab" => "Tab",
        "left" | "leftarrow" | "arrowleft" => "Left",
        "right" | "rightarrow" | "arrowright" => "Right",
        "up" | "uparrow" | "arrowup" => "Up",
        "down" | "downarrow" | "arrowdown" => "Down",
        "home" => "Home",
        "end" => "End",
        "delete" | "del" => "Delete",
        "backspace" | "bs" => "Backspace",
        "insert" | "ins" => "Insert",
        _ => "",
    };
    if !canonical.is_empty() {
        return Some(canonical.to_owned());
    }
    // F1..F24.
    if (low.starts_with('f') || low.starts_with('F'))
        && low[1..].chars().all(|c| c.is_ascii_digit())
        && !low[1..].is_empty()
        && let Ok(n) = low[1..].parse::<u32>()
        && (1..=24).contains(&n)
    {
        return Some(format!("F{n}"));
    }
    // Fallback: keep whatever the caller passed (preserves the user's
    // spelling for exotic keys). Refuse purely-modifier tokens.
    match low.as_str() {
        "ctrl" | "control" | "shift" | "alt" | "meta" | "opt" | "option" => None,
        _ => Some(k.to_owned()),
    }
}

/// Decode the body of the `"keybindings"` object out of the settings
/// JSON payload. Same shape as the existing `pull_str` helper in
/// `services::decode_settings`: this function receives the FULL
/// settings text and pulls the `"keybindings": { ... }` object.
///
/// Kept in this module so the parser + serializer live next to the
/// catalog they encode. Malformed entries (unknown ids, unparseable
/// chords) are silently dropped so an on-disk error never leaves the
/// user with an unreachable overlay.
#[cfg(test)] // See `encode_overrides_body` - no settings persistence yet.
pub(crate) fn decode_overrides(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let key = "\"keybindings\":";
    let Some(start) = text.find(key) else {
        return out;
    };
    let after = &text[start + key.len()..];
    let after = after.trim_start();
    if !after.starts_with('{') {
        return out;
    }
    // Scan until the matching '}' at nesting depth 0.
    let bytes = after.as_bytes();
    let mut depth = 0i32;
    let mut end = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    if end == 0 {
        return out;
    }
    let body = &after[1..end];
    // Walk `body` looking for `"id": "chord"` pairs. Same shape as
    // the ssh_hosts.json parser in services.rs — deliberately small,
    // no external dep.
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Find opening quote of key.
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        i += 1;
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let id = &body[key_start..i];
        i += 1;
        // Skip ':' and whitespace.
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        i += 1;
        let val_start = i;
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let val = &body[val_start..i];
        i += 1;
        let id = unesc(id);
        let chord = unesc(val);
        // Only keep entries with a known action id AND a parseable
        // chord. This turns a stale/broken on-disk pair into a silent
        // no-op instead of a runtime crash.
        if find_action(&id).is_some()
            && let Some(parsed) = Chord::parse(&chord)
        {
            out.insert(id, parsed.to_string());
        }
    }
    out
}

#[cfg(test)] // Helper of the `#[cfg(test)]` overrides codec above.
fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for ch in s.chars() {
        if esc {
            match ch {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                other => out.push(other),
            }
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else {
            out.push(ch);
        }
    }
    out
}
