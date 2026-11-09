//! Keyboard chord parsing and canonicalisation.

use super::*;

// ---------------------------------------------------------------------------
// Chord parsing / canonicalisation.
// ---------------------------------------------------------------------------

/// A parsed keyboard chord — modifiers + one non-modifier key.
///
/// `key` is stored in a NORMALISED form:
///   * Single ASCII letters -> uppercase (`t` -> `T`).
///   * Digits kept as-is (`1` .. `9` / `0`).
///   * Named keys use their canonical PascalCase name
///     (`PageUp`, `PageDown`, `Enter`, `Escape`, `Tab`, `Space`,
///     `Left`, `Right`, `Up`, `Down`, `Home`, `End`, `F1`..`F12`,
///     `Delete`, `Backspace`, `Insert`, `?`, `,`, `.`, `;`, `/`, `\\`,
///     `'`, `` ` ``, `-`, `=`, `[`, `]`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

impl Chord {
    /// Build a chord from raw event data. `key` is passed through
    /// `normalise_key` so downstream code can rely on canonical
    /// spelling regardless of platform.
    pub(crate) fn new(ctrl: bool, shift: bool, alt: bool, key: &str) -> Option<Self> {
        let key = normalise_key(key)?;
        Some(Self {
            ctrl,
            shift,
            alt,
            key,
        })
    }

    /// Parse a human-typed chord like `"Ctrl+Shift+T"`, `"C-S-Tab"`,
    /// `"alt 3"`, `"F2"`, `"?"`. Returns `None` if the input has no
    /// non-modifier key or contains an unrecognised token.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        // Accept `+`, `-`, and whitespace as separators. We can't
        // split on `+` naively because the key ITSELF might be `+`;
        // handle that by pulling modifier tokens greedily off the
        // left until only the terminal key remains.
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut remaining = s.to_owned();
        loop {
            let trimmed = remaining.trim();
            // Pick the next token: everything up to the first
            // separator (+/-/whitespace) that's followed by MORE
            // input. If no separator remains, the whole string is
            // the terminal key.
            let sep_idx = trimmed.find(|c: char| c == '+' || c == '-' || c.is_whitespace());
            let (tok, rest) = match sep_idx {
                Some(0) => return None, // leading separator
                Some(i) => {
                    let tok = &trimmed[..i];
                    // Consume one separator.
                    let rest = &trimmed[i + 1..];
                    (tok, rest.trim())
                }
                None => {
                    // Whole thing is the terminal key.
                    (trimmed, "")
                }
            };
            let low = tok.to_ascii_lowercase();
            let is_modifier = matches!(
                low.as_str(),
                "ctrl" | "control" | "c" | "shift" | "s" | "alt" | "meta" | "opt" | "option"
            );
            if is_modifier && !rest.is_empty() {
                match low.as_str() {
                    "ctrl" | "control" | "c" => ctrl = true,
                    "shift" | "s" => shift = true,
                    "alt" | "meta" | "opt" | "option" => alt = true,
                    _ => unreachable!(),
                }
                remaining = rest.to_owned();
                continue;
            }
            // Terminal token — must consume the REMAINING text
            // entirely with no interior whitespace. "not a real chord"
            // has "not" as the first candidate but leaves the rest
            // dangling: reject rather than trying to smash it into
            // one key spelling. The one exception is a stray leading
            // token of "+" (i.e. the user typed `Ctrl++` to bind the
            // literal `+` key) which we detect by the token being
            // empty at this point — handled above.
            if !rest.is_empty() {
                // Only allow rejoining when the separator we
                // consumed was `+` or `-` AND the rest has no
                // further whitespace-delimited words. Concretely:
                // reject anything with an interior space, which
                // catches the "not a real chord" family. Chords are
                // always compact tokens.
                if rest.contains(char::is_whitespace) {
                    return None;
                }
                let key = format!("{tok}+{rest}");
                return Chord::new(ctrl, shift, alt, &key);
            }
            return Chord::new(ctrl, shift, alt, tok);
        }
    }
}

// Canonical string form is provided via `Display` (and the `ToString` blanket
// impl), so `chord.to_string()` == `format_chord(&chord)`. No inherent
// `to_string` — that would shadow `Display` (clippy inherent_to_string_shadow_display).
impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format_chord(self))
    }
}

/// Format a chord in canonical `"Ctrl+Shift+Alt+KEY"` order. Kept as
/// a free function so tests can call it on ad-hoc `Chord` literals
/// without `use`.
pub(crate) fn format_chord(c: &Chord) -> String {
    let mut out = String::new();
    if c.ctrl {
        out.push_str("Ctrl+");
    }
    if c.shift {
        out.push_str("Shift+");
    }
    if c.alt {
        out.push_str("Alt+");
    }
    out.push_str(&c.key);
    out
}
