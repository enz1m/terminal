// No module-level allow.
//! Standard xterm-256 palette + colour-resolution helpers shared
//! between the vt100 and termwiz terminal backends.
//!
//! Both backends emit ANSI SGR colour attributes as one of:
//!   * `Default`         — use the terminal's default fg/bg
//!   * `Palette(0..=255)` — index into the standard xterm table
//!   * `TrueColor(r,g,b)` — direct 24-bit sRGB
//!
//! Real terminal emulators (wezterm, alacritty, kitty, iTerm2)
//! resolve palette indices through a fixed 256-colour table:
//! - `0-7`: the "system" ANSI base colours (black, red, green, yellow, blue,
//!   magenta, cyan, white)
//! - `8-15`: the "bright" variants of the same
//! - `16-231`: a 6x6x6 RGB cube; index = 16 + r*36 + g*6 + b, where each
//!   channel is one of the 6 canonical levels `[0, 95, 135, 175, 215, 255]`
//! - `232-255`: 24 shades of grey from #080808 up to #eeeeee in 10-unit steps
//!
//! Previously this app bucketed every colour into six `LineTone`
//! semantic slots (Default/Muted/Accent/Success/Warning/Error) and
//! threw the real palette away — TUIs like btop that draw
//! multi-colour gradient meters came out monochrome. The backends now
//! record the colour **source** in [`crate::model::CellInk`] and the
//! draw path resolves it, so `LineTone` is retained only as a FALLBACK
//! for cells with `Default` foreground.
//!
//! # Resolution happens at draw time, and the safeguard measures contrast
//!
//! Two defects lived here. First, `TranscriptCell::fg` used to hold an
//! already-resolved `(u8, u8, u8)`, so switching colour scheme repainted the
//! canvas but left every visible glyph the colour the *old* scheme resolved it
//! to — "the background changes but the text colours do not". The cell now
//! stores the source ([`crate::model::CellInk`]) and `grid_view::map_cell`
//! resolves it against the active scheme on every frame, so a scheme change
//! recolours the transcript that is already on screen.
//!
//! Second, the old `readable_on_dark` safeguard fired only when a colour's
//! *largest* RGB channel was below `0x3a`. Classic ANSI navy `#000080` — the
//! colour `LS_COLORS`' default `di=01;34` paints every directory name — has a
//! blue channel of `0x80`, so it took the untouched branch while measuring
//! **1.04:1** against the old `#242430` canvas: invisible. "Darkness of the
//! brightest channel" is not contrast. [`readable_on`] measures the real WCAG
//! ratio (through the framework's `accessibility_metrics::contrast_ratio`) against the
//! background the glyph actually lands on, and it is applied ONLY to
//! default/indexed colours — a truecolor value is the application's own
//! request and is never rewritten (btop/htop dim meter tracks).

/// The six canonical channel levels used by the 6x6x6 colour cube
/// (indices 16-231). Exposed for tests and for callers that need to
/// reason about which cube slot an arbitrary RGB triple would land
/// in.
pub(crate) const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The 16 "system" ANSI colours (indices 0-15) as xterm defines them.
///
/// **This is the xterm REFERENCE table, not a shipped scheme.** It used
/// to be the `Default` colour scheme's palette, which is why `ls --color`
/// painted directories in `#000080` navy at 1.04:1 on the terminal canvas. No
/// entry of [`crate::color_scheme::SCHEMES`] uses it any more: every shipped
/// scheme is a published palette that owns its own canvas, and slots 0..=15 are
/// resolved through [`crate::color_scheme::resolve_ansi_index_for`]. The table stays
/// because it is the documented meaning of an SGR index (`31` *is* "maroon" in
/// xterm) and the tests below are the readable statement of that mapping.
pub(crate) const SYSTEM_16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // 0  black
    (0x80, 0x00, 0x00), // 1  maroon (dark red)
    (0x00, 0x80, 0x00), // 2  green
    (0x80, 0x80, 0x00), // 3  olive (dark yellow)
    (0x00, 0x00, 0x80), // 4  navy (dark blue)
    (0x80, 0x00, 0x80), // 5  purple (dark magenta)
    (0x00, 0x80, 0x80), // 6  teal (dark cyan)
    (0xc0, 0xc0, 0xc0), // 7  silver (light grey)
    (0x80, 0x80, 0x80), // 8  bright black (dark grey)
    (0xff, 0x00, 0x00), // 9  bright red
    (0x00, 0xff, 0x00), // 10 bright green
    (0xff, 0xff, 0x00), // 11 bright yellow
    (0x00, 0x00, 0xff), // 12 bright blue
    (0xff, 0x00, 0xff), // 13 bright magenta
    (0x00, 0xff, 0xff), // 14 bright cyan
    (0xff, 0xff, 0xff), // 15 bright white
];

/// Resolve an xterm-256 palette index to an sRGB triple.
///
/// This is the reference implementation used by every mainstream terminal
/// emulator. The app reaches it through
/// [`crate::color_scheme::resolve_ansi_index_for`], which overrides slots
/// 0..=15 with the active scheme's palette and delegates the universal cube and
/// greyscale ramp (16..=255) here.
pub(crate) fn resolve_ansi_index(i: u8) -> (u8, u8, u8) {
    if (i as usize) < 16 {
        SYSTEM_16[i as usize]
    } else if i < 232 {
        // 6x6x6 cube. `idx` is offset from 16; `r`, `g`, `b` each
        // range 0..=5, each mapped through CUBE_LEVELS to get the
        // canonical channel value.
        let idx = i - 16;
        let r = idx / 36;
        let g = (idx % 36) / 6;
        let b = idx % 6;
        (
            CUBE_LEVELS[r as usize],
            CUBE_LEVELS[g as usize],
            CUBE_LEVELS[b as usize],
        )
    } else {
        // Grayscale ramp: 24 shades from 0x08 up to 0xee in 10-unit
        // steps. Standard xterm formula: v = 8 + (i - 232) * 10.
        let v = 8u16 + (u16::from(i) - 232) * 10;
        let v = v.min(255) as u8;
        (v, v, v)
    }
}

/// Minimum WCAG contrast ratio the app guarantees between a *framework-chosen*
/// glyph colour (SGR default, or an ANSI/xterm palette index) and the canvas it
/// is painted on.
///
/// **Why 3.0 and not 4.5.** The nudge in [`readable_on`] preserves hue by moving
/// the colour along the straight line to white (or to black on a light canvas),
/// which is the only adjustment that leaves an ANSI colour recognisable as "the
/// red one". That bounds what is reachable: the brightest red-hued ink that
/// exists, `#ff0000`, measures **3.56:1** on Dracula's `#282a36` canvas and
/// **3.16:1** on Tango's `#2e3436` — see
/// `saturated_red_bounds_the_floor_on_every_dark_canvas`. A floor above ~3.1
/// could therefore only be met by desaturating red ANSI ink into pink, i.e. by
/// throwing away the palette's hue identity for a number. 3.0 is also WCAG
/// 1.4.3's large-text / 1.4.11's non-text threshold, and it is 2.9x the ratio
/// the reported navy-on-canvas defect measured.
pub(crate) const MIN_TERMINAL_INK_CONTRAST: f32 = 3.0;

/// WCAG contrast ratio between two opaque sRGB triples.
///
/// Framework capability, not hand-rolled: delegates to
/// `enzui_widgets::accessibility_metrics::contrast_ratio`, the same math the
/// framework's own a11y contrast probe scores every widget with.
pub(crate) fn contrast_ratio_rgb(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    enzui_widgets::accessibility_metrics::contrast_ratio(
        iced::Color::from_rgb8(a.0, a.1, a.2),
        iced::Color::from_rgb8(b.0, b.1, b.2),
    )
}

/// Raise a **framework-chosen** glyph colour until it clears
/// [`MIN_TERMINAL_INK_CONTRAST`] against the background it is painted on,
/// preserving its hue.
///
/// Applies to SGR `Default` and to xterm palette indices — colours the terminal
/// emulator picks on the application's behalf. It must NEVER be applied to a
/// truecolor value: a TUI that asked for `#101010` asked for it, and btop /
/// htop lean on dim intensities for their dotted meter tracks. See
/// `crate::model::CellInk`, which is what makes the distinction representable at
/// all.
///
/// The adjustment lerps toward whichever of white/black scores better against
/// `bg`. A lerp toward either endpoint of the RGB cube's neutral axis preserves
/// HSL hue exactly (channel ordering and the mid/max ratio are both invariant),
/// so a nudged navy is still blue — see
/// `nudged_navy_is_still_dominantly_blue`.
///
/// Returns `fg` unchanged when it already clears the floor (the overwhelmingly
/// common case: every chromatic slot of the default scheme clears it raw).
///
/// The grid resolves this for every indexed-colour cell on every frame, and a
/// full-screen TUI repaints thousands of such cells per frame from a handful of
/// distinct pairs, so results are memoised per thread. The function is pure, so
/// a cached answer is the computed answer.
pub(crate) fn readable_on(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> (u8, u8, u8) {
    type Entry = Option<((u8, u8, u8), (u8, u8, u8), (u8, u8, u8))>;
    const SLOTS: usize = 256;
    thread_local! {
        static MEMO: std::cell::RefCell<[Entry; SLOTS]> =
            const { std::cell::RefCell::new([None; SLOTS]) };
    }
    let key = u32::from_le_bytes([fg.0, fg.1, fg.2, 0]).wrapping_mul(0x9e37_79b1)
        ^ u32::from_le_bytes([bg.0, bg.1, bg.2, 0]).wrapping_mul(0x85eb_ca6b);
    let slot = (key >> 24) as usize % SLOTS;
    if let Some((f, b, out)) = MEMO.with(|memo| memo.borrow()[slot])
        && f == fg
        && b == bg
    {
        return out;
    }
    let out = readable_on_uncached(fg, bg);
    MEMO.with(|memo| memo.borrow_mut()[slot] = Some((fg, bg, out)));
    out
}

fn readable_on_uncached(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> (u8, u8, u8) {
    if contrast_ratio_rgb(fg, bg) >= MIN_TERMINAL_INK_CONTRAST {
        return fg;
    }
    const WHITE: (u8, u8, u8) = (0xff, 0xff, 0xff);
    const BLACK: (u8, u8, u8) = (0x00, 0x00, 0x00);
    let target = if contrast_ratio_rgb(WHITE, bg) >= contrast_ratio_rgb(BLACK, bg) {
        WHITE
    } else {
        BLACK
    };
    // Smallest t in (0, 1] that clears the floor, to 1/256 — keep the colour as
    // close to the palette author's value as the floor allows. Bisection is
    // valid because contrast toward the better endpoint is monotone in t.
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if contrast_ratio_rgb(lerp_rgb(fg, target, mid), bg) >= MIN_TERMINAL_INK_CONTRAST {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    lerp_rgb(fg, target, hi)
}

/// Linear per-channel interpolation between two sRGB triples.
fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let ch = |x: u8, y: u8| -> u8 {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2))
}
