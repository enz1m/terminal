// NO module-level `#![allow(dead_code)]`. A blanket allow hides a
// feature that is built, tested and unreachable; anything deliberately unused
// carries a TARGETED allow or `#[cfg(test)]` on the item, with a reason.
//! Named 16-colour ANSI palettes (+ default fg / bg / cursor) that the
//! user can pick from the Settings dialog.
//!
//! Real terminal emulators (iTerm, wezterm, alacritty, kitty, gnome-
//! terminal) ship a curated set of colour schemes: the base 16 ANSI
//! slots (black, red, green, yellow, blue, magenta, cyan, white +
//! their 8 bright variants) plus explicit default foreground /
//! background / cursor tones. Selecting a scheme "Solarized Dark" or
//! "Dracula" is one of the most-requested features of any terminal
//! app because those palettes have been tuned for readability, hue
//! separation on log output, and gentle-on-the-eyes contrast during
//! long sessions.
//!
//! Design:
//!   * A [`ColorScheme`] struct owns 16 `(u8, u8, u8)` ANSI slots +
//!     `default_fg` / `default_bg` / `cursor` triples.
//!   * The full catalogue lives in [`SCHEMES`], indexed by a stable
//!     string id persisted to `settings.json`.
//!   * The active scheme is exposed via [`active`] (reads a global
//!     `AtomicUsize` set by [`set_active_by_id`]). Kept a global
//!     because the projection layer (`services.rs`,
//!     `emulator.rs`) is called from 20+ call sites and
//!     threading a `&ColorScheme` through every one would be a huge
//!     diff for a single ambient value that changes rarely.
//!
//! The projection layer calls [`resolve_ansi_index_for`] (this module)
//! which respects the active scheme for indices 0..=15 and falls
//! back to the standard xterm-256 table (`palette::resolve_ansi_index`)
//! for indices 16..=255. That way schemes only override the base 16
//! slots — the 6×6×6 colour cube and the greyscale ramp stay universal,
//! which matches how every real terminal emulator implements colour
//! schemes.

use std::sync::atomic::{AtomicUsize, Ordering};

/// A single named colour scheme: 16 ANSI slots + default fg/bg/cursor.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ColorScheme {
    /// Stable id used in `settings.json`. Lowercase, hyphenated.
    pub id: &'static str,
    /// Human-readable name shown in the Settings dialog picker.
    pub name: &'static str,
    /// The 16 ANSI palette entries. Same slot order as
    /// [`crate::palette::SYSTEM_16`]:
    ///   0 = black, 1 = red, 2 = green, 3 = yellow, 4 = blue,
    ///   5 = magenta, 6 = cyan, 7 = white,
    ///   8..=15 = bright variants of the same.
    pub ansi: [(u8, u8, u8); 16],
    /// Default text (foreground) colour when the shell hasn't set an
    /// explicit SGR fg. This IS the "normal text" colour of the
    /// scheme.
    pub default_fg: (u8, u8, u8),
    /// Default background colour of the terminal grid: the canvas this scheme
    /// is painted on, and the only source of the terminal's background colour.
    /// There is no second, manual background system to defer to.
    pub default_bg: (u8, u8, u8),
    /// Cursor colour when the pane is focused. Real emulators pick a
    /// scheme-specific cursor tone (Solarized uses base01 grey, Nord
    /// uses frost cyan, Dracula uses foreground light).
    pub cursor: (u8, u8, u8),
}

// ---------------------------------------------------------------------------
// Catalogue
// ---------------------------------------------------------------------------
//
// All hex values below are the canonical published palettes:
//   * Solarized Dark / Light — Ethan Schoonover, solarized.io
//   * Dracula — draculatheme.com
//   * Gruvbox Dark — morhetz/gruvbox on GitHub (dark medium contrast)
//   * Nord — nordtheme.com
//   * Tango — GNOME Tango palette (gnome-terminal default)
//
// The 16 ANSI slots follow the standard iTerm2 export order: normal
// black/red/green/yellow/blue/magenta/cyan/white, then the bright
// variants of the same. Some schemes (Solarized, Nord) treat the
// bright slots as accent hues rather than "brighter of the base"; we
// keep the canonical scheme mapping so `SGR 1;31` (bright red) shows
// what the scheme author intended.

pub(crate) const SCHEMES: &[ColorScheme] = &[
    // -----------------------------------------------------------------
    // Dracula — draculatheme.com/terminal.
    //
    // THE SHIPPED DEFAULT, and first so the pre-settings global
    // (`ACTIVE_INDEX` starts at 0) is already a high-contrast scheme.
    //
    // It replaced the old `Default` scheme, which was the classic xterm
    // `SYSTEM_16` table on a `#242430` canvas: ANSI navy `#000080` at
    // **1.04:1** for every directory name `ls --color` printed. Dracula was
    // chosen over Nord on measurement, not taste — its own `default_fg`
    // measures 13.36:1 on its canvas (Nord: 9.25:1) and its weakest CHROMATIC
    // ANSI slot measures 4.53:1 (Nord: 3.05:1), so it is the one shipped
    // palette that clears the contrast floor on every slot except the
    // canonically-near-canvas black. Values are draculatheme.com's published
    // terminal palette, unedited — see `scheme_ink.rs` for the measured
    // table of every scheme.
    // -----------------------------------------------------------------
    ColorScheme {
        id: "dracula",
        name: "Dracula",
        ansi: [
            (0x21, 0x22, 0x2c), // 0 background
            (0xff, 0x55, 0x55), // 1 red
            (0x50, 0xfa, 0x7b), // 2 green
            (0xf1, 0xfa, 0x8c), // 3 yellow
            (0xbd, 0x93, 0xf9), // 4 blue (purple)
            (0xff, 0x79, 0xc6), // 5 magenta (pink)
            (0x8b, 0xe9, 0xfd), // 6 cyan
            (0xf8, 0xf8, 0xf2), // 7 white
            (0x62, 0x72, 0xa4), // 8  bright black (comment)
            (0xff, 0x6e, 0x6e), // 9  bright red
            (0x69, 0xff, 0x94), // 10 bright green
            (0xff, 0xff, 0xa5), // 11 bright yellow
            (0xd6, 0xac, 0xff), // 12 bright blue
            (0xff, 0x92, 0xdf), // 13 bright magenta
            (0xa4, 0xff, 0xff), // 14 bright cyan
            (0xff, 0xff, 0xff), // 15 bright white
        ],
        default_fg: (0xf8, 0xf8, 0xf2),
        default_bg: (0x28, 0x2a, 0x36),
        cursor: (0xf8, 0xf8, 0xf2),
    },
    // -----------------------------------------------------------------
    // Solarized Dark — Ethan Schoonover.
    // Canonical published hex values (see
    // https://ethanschoonover.com/solarized/).
    // -----------------------------------------------------------------
    ColorScheme {
        id: "solarized-dark",
        name: "Solarized Dark",
        ansi: [
            (0x07, 0x36, 0x42), // 0 base02  (background-highlight)
            (0xdc, 0x32, 0x2f), // 1 red
            (0x85, 0x99, 0x00), // 2 green
            (0xb5, 0x89, 0x00), // 3 yellow
            (0x26, 0x8b, 0xd2), // 4 blue
            (0xd3, 0x36, 0x82), // 5 magenta
            (0x2a, 0xa1, 0x98), // 6 cyan
            (0xee, 0xe8, 0xd5), // 7 base2 (content-highlight)
            (0x00, 0x2b, 0x36), // 8  base03 bright black = darkest bg
            (0xcb, 0x4b, 0x16), // 9  orange (bright red slot)
            (0x58, 0x6e, 0x75), // 10 base01 (bright green slot)
            (0x65, 0x7b, 0x83), // 11 base00 (bright yellow slot)
            (0x83, 0x94, 0x96), // 12 base0  (bright blue slot)
            (0x6c, 0x71, 0xc4), // 13 violet (bright magenta slot)
            (0x93, 0xa1, 0xa1), // 14 base1  (bright cyan slot)
            (0xfd, 0xf6, 0xe3), // 15 base3  (bright white)
        ],
        default_fg: (0x83, 0x94, 0x96), // base0
        default_bg: (0x00, 0x2b, 0x36), // base03
        cursor: (0x93, 0xa1, 0xa1),     // base1
    },
    // -----------------------------------------------------------------
    // Solarized Light — same palette, light canvas.
    // -----------------------------------------------------------------
    ColorScheme {
        id: "solarized-light",
        name: "Solarized Light",
        ansi: [
            (0xee, 0xe8, 0xd5), // 0 base2
            (0xdc, 0x32, 0x2f), // 1 red
            (0x85, 0x99, 0x00), // 2 green
            (0xb5, 0x89, 0x00), // 3 yellow
            (0x26, 0x8b, 0xd2), // 4 blue
            (0xd3, 0x36, 0x82), // 5 magenta
            (0x2a, 0xa1, 0x98), // 6 cyan
            (0x07, 0x36, 0x42), // 7 base02
            (0xfd, 0xf6, 0xe3), // 8  base3
            (0xcb, 0x4b, 0x16), // 9  orange
            (0x93, 0xa1, 0xa1), // 10 base1
            (0x83, 0x94, 0x96), // 11 base0
            (0x65, 0x7b, 0x83), // 12 base00
            (0x6c, 0x71, 0xc4), // 13 violet
            (0x58, 0x6e, 0x75), // 14 base01
            (0x00, 0x2b, 0x36), // 15 base03
        ],
        default_fg: (0x65, 0x7b, 0x83), // base00
        default_bg: (0xfd, 0xf6, 0xe3), // base3
        cursor: (0x58, 0x6e, 0x75),     // base01
    },
    // -----------------------------------------------------------------
    // Gruvbox Dark (medium contrast) — morhetz/gruvbox.
    // -----------------------------------------------------------------
    ColorScheme {
        id: "gruvbox-dark",
        name: "Gruvbox Dark",
        ansi: [
            (0x28, 0x28, 0x28), // 0 bg0
            (0xcc, 0x24, 0x1d), // 1 red
            (0x98, 0x97, 0x1a), // 2 green
            (0xd7, 0x99, 0x21), // 3 yellow
            (0x45, 0x85, 0x88), // 4 blue
            (0xb1, 0x62, 0x86), // 5 magenta (purple)
            (0x68, 0x9d, 0x6a), // 6 cyan (aqua)
            (0xa8, 0x99, 0x84), // 7 fg4 (light4)
            (0x92, 0x83, 0x74), // 8  bright black (gray)
            (0xfb, 0x49, 0x34), // 9  bright red
            (0xb8, 0xbb, 0x26), // 10 bright green
            (0xfa, 0xbd, 0x2f), // 11 bright yellow
            (0x83, 0xa5, 0x98), // 12 bright blue
            (0xd3, 0x86, 0x9b), // 13 bright magenta
            (0x8e, 0xc0, 0x7c), // 14 bright cyan (aqua)
            (0xeb, 0xdb, 0xb2), // 15 fg1 (light1)
        ],
        default_fg: (0xeb, 0xdb, 0xb2),
        default_bg: (0x28, 0x28, 0x28),
        cursor: (0xeb, 0xdb, 0xb2),
    },
    // -----------------------------------------------------------------
    // Nord — nordtheme.com.
    // -----------------------------------------------------------------
    ColorScheme {
        id: "nord",
        name: "Nord",
        ansi: [
            (0x3b, 0x42, 0x52), // 0 nord1 (polar-night)
            (0xbf, 0x61, 0x6a), // 1 nord11 (aurora red)
            (0xa3, 0xbe, 0x8c), // 2 nord14 (aurora green)
            (0xeb, 0xcb, 0x8b), // 3 nord13 (aurora yellow)
            (0x81, 0xa1, 0xc1), // 4 nord9  (frost blue)
            (0xb4, 0x8e, 0xad), // 5 nord15 (aurora purple)
            (0x88, 0xc0, 0xd0), // 6 nord8  (frost cyan)
            (0xe5, 0xe9, 0xf0), // 7 nord5  (snow-storm)
            (0x4c, 0x56, 0x6a), // 8  nord3  (bright polar-night)
            (0xbf, 0x61, 0x6a), // 9  same red
            (0xa3, 0xbe, 0x8c), // 10 same green
            (0xeb, 0xcb, 0x8b), // 11 same yellow
            (0x81, 0xa1, 0xc1), // 12 same blue
            (0xb4, 0x8e, 0xad), // 13 same purple
            (0x8f, 0xbc, 0xbb), // 14 nord7  (frost teal)
            (0xec, 0xef, 0xf4), // 15 nord6  (snow-storm brightest)
        ],
        default_fg: (0xd8, 0xde, 0xe9), // nord4
        default_bg: (0x2e, 0x34, 0x40), // nord0
        cursor: (0xd8, 0xde, 0xe9),
    },
    // -----------------------------------------------------------------
    // Tango — GNOME Tango palette (gnome-terminal default).
    // -----------------------------------------------------------------
    ColorScheme {
        id: "tango",
        name: "Tango",
        ansi: [
            (0x2e, 0x34, 0x36), // 0 aluminium 6 (nearly black)
            (0xcc, 0x00, 0x00), // 1 scarlet red 2
            (0x4e, 0x9a, 0x06), // 2 chameleon 3
            (0xc4, 0xa0, 0x00), // 3 butter 3
            (0x34, 0x65, 0xa4), // 4 sky blue 2
            (0x75, 0x50, 0x7b), // 5 plum 2
            (0x06, 0x98, 0x9a), // 6 (Tango's cyan approx)
            (0xd3, 0xd7, 0xcf), // 7 aluminium 2
            (0x55, 0x57, 0x53), // 8  aluminium 5 (dark grey)
            (0xef, 0x29, 0x29), // 9  scarlet red 1
            (0x8a, 0xe2, 0x34), // 10 chameleon 1
            (0xfc, 0xe9, 0x4f), // 11 butter 1
            (0x72, 0x9f, 0xcf), // 12 sky blue 1
            (0xad, 0x7f, 0xa8), // 13 plum 1
            (0x34, 0xe2, 0xe2), // 14 cyan
            (0xee, 0xee, 0xec), // 15 aluminium 1 (near white)
        ],
        default_fg: (0xd3, 0xd7, 0xcf),
        default_bg: (0x2e, 0x34, 0x36),
        cursor: (0xef, 0xef, 0xef),
    },
];

/// The default scheme id, used when `Settings::color_scheme` is empty or names
/// something unknown.
///
/// Was `"default"` — the classic xterm palette on a `#242430` canvas,
/// whose ANSI navy blue measured 1.04:1 and made `ls --color` directory names
/// invisible. That mapping is deleted, not re-tuned; the default is now a
/// published high-contrast palette. It is deliberately `SCHEMES[0]` so the
/// process-global `ACTIVE_INDEX` (which starts at 0, before any settings are
/// applied) already names it — `default_id_is_the_first_scheme` gates that.
pub(crate) const DEFAULT_ID: &str = "dracula";

/// Look up a scheme by its stable id. Returns the default scheme on
/// unknown ids so a stale `settings.json` value can never leave the
/// palette resolver in an undefined state.
pub(crate) fn find_by_id(id: &str) -> &'static ColorScheme {
    for s in SCHEMES {
        if s.id == id {
            return s;
        }
    }
    &SCHEMES[0]
}

/// Return the numeric index of a scheme by id, or 0 (default) on unknown.
pub(crate) fn index_by_id(id: &str) -> usize {
    SCHEMES.iter().position(|s| s.id == id).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Global active-scheme cell
// ---------------------------------------------------------------------------

static ACTIVE_INDEX: AtomicUsize = AtomicUsize::new(0);

/// Set the active scheme by id. Silently no-ops on unknown ids —
/// callers can pass a stale persisted value without corrupting the
/// global.
pub(crate) fn set_active_by_id(id: &str) {
    ACTIVE_INDEX.store(index_by_id(id), Ordering::Relaxed);
}

/// Read the currently-active scheme. Cheap — one atomic load + array
/// index.
pub(crate) fn active() -> &'static ColorScheme {
    scheme_at(ACTIVE_INDEX.load(Ordering::Relaxed))
}

/// Return the scheme at `idx`, clamping to the last entry on out-of-range.
///
/// Factored out of [`active`] so unit tests can exercise the clamping
/// logic without mutating the process-global `ACTIVE_INDEX`.  That
/// removes the `Mutex` that previously serialised the two global-
/// touching tests — and with it, the poison-cascade where one
/// assertion failure inside the lock produced a second, misleading
/// `PoisonError` in the other test.
fn scheme_at(idx: usize) -> &'static ColorScheme {
    &SCHEMES[idx.min(SCHEMES.len() - 1)]
}

// ---------------------------------------------------------------------------
// Palette resolution through the active scheme
// ---------------------------------------------------------------------------

// Palette resolution is not performed by the projection layer: a cell stores
// `CellInk::Indexed(i)` and `grid_view::resolve_fg_ink` reads the active scheme
// once, at draw time, then calls the explicit-scheme function below. There is
// deliberately no resolver that reads the process-global active scheme.

/// Resolve an xterm-256 palette index to sRGB through an EXPLICIT scheme.
///
/// * For indices 0..=15, returns `scheme.ansi[i]`.
/// * For indices 16..=255 (the 6×6×6 cube + greyscale ramp), delegates
///   to the standard xterm-256 table because schemes only override the
///   base 16 slots.
pub(crate) fn resolve_ansi_index_for(scheme: &ColorScheme, i: u8) -> (u8, u8, u8) {
    if (i as usize) < 16 {
        scheme.ansi[i as usize]
    } else {
        crate::palette::resolve_ansi_index(i)
    }
}

/// Tests that switch the process-global active scheme hold this exclusively;
/// tests that compare rendered frames hold it shared, so a scheme switch in a
/// parallel test cannot recolour a frame between two shots.
#[cfg(test)]
pub(crate) static SCHEME_SWITCH: std::sync::RwLock<()> = std::sync::RwLock::new(());

#[cfg(test)]
pub(crate) fn steady_scheme() -> std::sync::RwLockReadGuard<'static, ()> {
    SCHEME_SWITCH
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
