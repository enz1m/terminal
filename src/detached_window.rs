//! Terminal detached-window types.

use std::collections::HashMap;

use crate::{pty, scrollback};

/// One detached session's window.
///
/// The session id rather than an index: the sidebar can reorder while the
/// window is up, and an index would then point at somebody else's terminal.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DetachedViewport {
    /// A detached surface owns its viewport independently of `Pane::scroll`.
    /// This prevents a late callback from the second window moving the docked
    /// pane after that window has begun closing.
    pub(crate) scroll: scrollback::ScrollPosition,
    /// Fractional trackpad rows are retained per window AND pane. Sharing the
    /// docked residue made two windows complete each other's wheel gesture.
    pub(crate) wheel_row_residue: f32,
}

impl DetachedViewport {
    pub(crate) fn new(scroll: scrollback::ScrollPosition) -> Self {
        Self {
            scroll,
            wheel_row_residue: 0.0,
        }
    }
}

/// One retired session's unfinished teardown.
///
/// Keyed by the exact teardown token minted before that session's child was
/// spawned, so it follows the session between windows and a report for one
/// token can never settle another. Custody normally lives in the process
/// executor; `owner` is set only while the executor was too busy to admit it,
/// and even then the reducer never sweeps it.
#[derive(Debug)]
pub(crate) struct TeardownDebt {
    pub(crate) pane: String,
    pub(crate) token: pty::TeardownToken,
    pub(crate) outcome: pty::PtyTeardownOutcome,
    pub(crate) settled: bool,
    pub(crate) owner: Option<pty::PtyTeardownDebtOwner>,
}

/// Why this app asked for a window to be destroyed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseIntent {
    /// One dependent window; the per-window session policy still applies.
    Dependent,
    /// The committed process teardown, which has already published what it
    /// owed and emitted its own exit.
    Teardown,
}

#[derive(Debug, Clone)]
pub(crate) struct DetachedWindow {
    pub(crate) session: String,
    /// This window's own size, from the id-filtered resize subscription. Its
    /// PTY geometry comes from here, never from the main window's
    /// `content_size`.
    pub(crate) size: iced::Size,
    /// Set as soon as Re-dock requests closure. The compositor's close event
    /// remains the owner of the actual re-dock transition, but queued input is
    /// stale from this point and must not reach scroll state or the PTY.
    pub(crate) closing: bool,
    /// Scroll state is nested under the runtime window id (the key in
    /// `App::detached_windows`) and then the stable pane id. A detached window
    /// can therefore neither consume the docked pane's offset/residue nor a
    /// sibling detached window's.
    pub(crate) viewports: HashMap<String, DetachedViewport>,
    /// Whether this window's command-history overlay is open. Per window, so
    /// two detached terminals (and the docked shell) show history independently.
    pub(crate) history_open: bool,
    /// Visibility of this window's framework command menu.
    pub(crate) titlebar_overflow_open: bool,
}

impl DetachedWindow {
    pub(crate) fn new(session: String, panes: &[(String, scrollback::ScrollPosition)]) -> Self {
        Self {
            session,
            // Honest zero: nothing has been reported yet, and
            // `resize_detached_pty` is only ever called from a real resize
            // event. Inventing a monitor size here would size the PTY to a
            // screen it is not on.
            size: iced::Size::ZERO,
            closing: false,
            viewports: panes
                .iter()
                .map(|(id, scroll)| (id.clone(), DetachedViewport::new(*scroll)))
                .collect(),
            history_open: false,
            titlebar_overflow_open: false,
        }
    }
}
