// No module-level allow — every item here is live.
//! Mouse-tracking **reporting**: the terminal's side of the wire when an
//! application turns on DECSET 1000/1002/1003 and picks an encoding.
//!
//! This is the pointer analogue of [`crate::keymap`]: a pure translator from a
//! pointer gesture plus the pane's live tracking state into the bytes the child
//! expects. Like [`crate::keymap::cursor_key_bytes`] and the bracketed-paste
//! flag, the mode is *tracked, not applied to the grid* — it changes the BYTES
//! we must send, nothing on the surface.
//!
//! # The state is a PAIR, not a boolean
//!
//! termwiz's own doc comments say `SGRMouse` (1006) and `SGRPixelsMouse` (1016)
//! are encoding *selectors* that do not themselves enable reporting, while
//! `MouseTracking` (1000) / `ButtonEventMouse` (1002) / `AnyEventMouse` (1003)
//! enable reporting without choosing an encoding. So the tracked state is a
//! `(tracking, encoding)` pair; a single boolean cannot model "1003 is on but
//! the app never asked for SGR" versus "SGR is selected but nothing tracks".
//!
//! # Encodings, and why 1006 is not optional
//!
//! The legacy X10 encoding packs each coordinate into one byte offset by 32, so
//! it cannot express a column past **223** (255 − 32). On a wide window a click
//! past column 223 silently reports the wrong cell — the failure a user cannot
//! diagnose. SGR (1006) writes decimal coordinates with no ceiling, so it is the
//! encoding that must exist for a modern-width terminal to be correct. We
//! implement X10 only as the fallback an application gets if it enables tracking
//! *without* selecting SGR, and it clamps loudly (see [`X10_MAX_COORD`]).
//!
//! # Declined by name (rather than silently)
//!
//! * `HighlightMouseTracking` (1001) — requires the terminal to block waiting
//!   for the application to reply before highlighting, so a misbehaving program
//!   can hang the UI. Declined; xterm's own docs warn about it.
//! * `Utf8Mouse` (1005) — legacy UTF-8 extension, superseded by SGR.
//! * `SGRPixelsMouse` (1016) — pixel rather than cell coordinates; needs a
//!   cell→pixel mapping we would otherwise not carry. Declined.
//!
//! All three stay in `apply_mode`'s catch-all, but now *named* there.

/// The largest coordinate the legacy X10 encoding can carry (255 − 32).
pub(crate) const X10_MAX_COORD: usize = 223;

/// Which pointer events a tracking application asked to be told about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum MouseTracking {
    /// No tracking: the pointer belongs to local selection / scrollback.
    #[default]
    Off,
    /// DECSET 1000: button press and release only.
    Click,
    /// DECSET 1002: press, release, and motion **while a button is held**.
    ButtonEvent,
    /// DECSET 1003: every pointer motion, button held or not.
    AnyEvent,
}

/// How coordinates and the button are encoded on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum MouseEncoding {
    /// Legacy X10: `ESC [ M` then three bytes each offset by 32. Cannot express
    /// a coordinate past [`X10_MAX_COORD`].
    #[default]
    X10,
    /// DECSET 1006 SGR: `ESC [ < b ; col ; row (M|m)`, decimal and unbounded.
    Sgr,
}

/// The `(tracking, encoding)` pair a pane's program currently has engaged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct MouseProtocol {
    pub tracking: MouseTracking,
    pub encoding: MouseEncoding,
}

impl MouseProtocol {
    /// Whether any tracking mode is engaged (the pointer belongs to the app).
    pub(crate) fn tracks(&self) -> bool {
        self.tracking != MouseTracking::Off
    }
}

/// The mouse button a gesture carries.
///
/// Only `Left` exists: the terminal grid's pointer hooks report a left
/// press/drag/release and nothing else, so a middle- or right-button report has
/// no source to build from. Middle-click paste needs the primary selection, a
/// framework question; wiring right-button reporting would start by having the
/// grid report which button pressed. The `base()` seam keeps the encoding ready
/// for that button code (`Middle = 1`, `Right = 2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Button {
    Left,
}

impl Button {
    fn base(self) -> u8 {
        match self {
            Button::Left => 0,
        }
    }
}

/// One pointer gesture to (maybe) report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerEvent {
    /// Button pressed.
    Press(Button),
    /// Button released.
    Release(Button),
    /// Motion with `Button` held (a drag).
    Drag(Button),
    /// Wheel notch away from the user (scroll up).
    WheelUp,
    /// Wheel notch toward the user (scroll down).
    WheelDown,
}

impl PointerEvent {
    /// Whether the current `tracking` mode reports this event at all.
    fn reported_by(self, tracking: MouseTracking) -> bool {
        match tracking {
            MouseTracking::Off => false,
            // 1000: presses, releases and wheel notches — no motion.
            MouseTracking::Click => !matches!(self, PointerEvent::Drag(_)),
            // 1002 and 1003 both report drags here. The difference between
            // them, 1003's bare pointer motion with no button held, needs a
            // pointer-move hook the grid does not expose (it reports only
            // press/drag/release), so 1003 behaves as 1002.
            MouseTracking::ButtonEvent | MouseTracking::AnyEvent => true,
        }
    }

    /// The `cb` button code (before encoding), including the drag/wheel flags.
    fn code(self) -> u8 {
        match self {
            PointerEvent::Press(b) | PointerEvent::Release(b) => b.base(),
            // Motion adds the 32 flag.
            PointerEvent::Drag(b) => b.base() + 32,
            PointerEvent::WheelUp => 64,
            PointerEvent::WheelDown => 65,
        }
    }

    /// Whether this event is a *release* (SGR final byte `m` rather than `M`).
    fn is_release(self) -> bool {
        matches!(self, PointerEvent::Release(_))
    }
}

impl MouseProtocol {
    /// The bytes to send for `event` at 1-based cell (`col`, `row`), or `None`
    /// when the current tracking mode does not report this event (including
    /// [`MouseTracking::Off`]).
    ///
    /// `col`/`row` are 1-based screen cells, as every mouse protocol expects.
    pub(crate) fn encode(&self, event: PointerEvent, col: usize, row: usize) -> Option<Vec<u8>> {
        if !event.reported_by(self.tracking) {
            return None;
        }
        let col = col.max(1);
        let row = row.max(1);
        Some(match self.encoding {
            MouseEncoding::Sgr => {
                let final_byte = if event.is_release() { b'm' } else { b'M' };
                let mut out = format!("\x1b[<{};{};{}", event.code(), col, row).into_bytes();
                out.push(final_byte);
                out
            }
            MouseEncoding::X10 => {
                // Each field is value + 32 in a single byte, so a coordinate
                // past X10_MAX_COORD cannot be expressed and is CLAMPED — the
                // documented failure that makes 1006 mandatory on a wide window.
                let cb = 32u8.wrapping_add(event.code());
                let cx = 32u8.saturating_add(col.min(X10_MAX_COORD) as u8);
                let cy = 32u8.saturating_add(row.min(X10_MAX_COORD) as u8);
                vec![0x1b, b'[', b'M', cb, cx, cy]
            }
        })
    }

    /// Wheel bytes for `notches` (positive = scroll down / toward newest,
    /// negative = scroll up) at 1-based cell (`col`, `row`), one report per
    /// notch, or `None` when tracking is off.
    pub(crate) fn wheel_bytes(&self, notches: i64, col: usize, row: usize) -> Option<Vec<u8>> {
        if !self.tracks() || notches == 0 {
            return None;
        }
        let event = if notches > 0 {
            PointerEvent::WheelDown
        } else {
            PointerEvent::WheelUp
        };
        let one = self.encode(event, col, row)?;
        let mut out = Vec::with_capacity(one.len() * notches.unsigned_abs() as usize);
        for _ in 0..notches.unsigned_abs() {
            out.extend_from_slice(&one);
        }
        Some(out)
    }
}
