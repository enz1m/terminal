//! Terminal chrome geometry methods.

use super::*;

impl App {
    pub(crate) fn titlebar_height(&self) -> f32 {
        shell_tokens(self.enz_theme()).titlebar_height
    }

    /// The top strip of the content pane that the **floating** titlebar bar
    /// (its icon cluster + close-X) draws over — the height the terminal grid
    /// must be inset by so no row is painted underneath the icons.
    ///
    /// It is not a hardcoded 40/52: it is the framework's own reported
    /// `content_inset[0]` for the exact layout this app builds
    /// ([`ShellLayout::FloatingTitlebarSidebar`]) resolved from the same
    /// [`shell_tokens()`] the shell is composed with. The framework computes it
    /// as `titlebar_height + gap * 2` (`floating_content_inset`), so if the band
    /// height or the shell gap is retuned this inset moves with it and the grid
    /// stays clear of the bar. Reading it here, rather than re-deriving it
    /// locally, keeps the grid out from under the floating bar's icons.
    ///
    /// Resolved against the live content width so the sidebar's Inline/Overlay
    /// decision (which does not affect the *titlebar* inset for a floating bar,
    /// but is the geometry function's contract) is taken from the same width
    /// the shell uses.
    ///
    /// The grid starts at the bar's own bottom edge, not below the shell's
    /// gaps under it: those left a band of empty canvas between the bar and
    /// the first row (and, tiled, between the bar and the line the seams
    /// meet). One terminal's older output continues under the bar instead
    /// (`under_titlebar`).
    pub(crate) fn grid_top_inset(&self) -> f32 {
        let inset = self.shell_geometry().content_inset[0].max(0.0);
        self.titlebar_height().min(inset)
    }

    // PTY rows and paint use the exact detached shell's floating clearance.
    pub(crate) fn detached_top_strip(&self) -> f32 {
        enzui_widgets::app_shell::resolve_shell_geometry(
            ShellLayout::FloatingTitlebar,
            Default::default(),
            crate::shell_tokens::detached_shell_tokens(),
            self.enz_theme().breakpoints,
            0.0,
        )
        .content_inset[0]
            .max(0.0)
    }

    /// The width, in logical px, the **content pane** actually receives — the
    /// window minus the shell's two edge insets and, while the sidebar is laid
    /// out **inline** beside the grid, minus the sidebar pane and the one
    /// gutter between them.
    ///
    /// The horizontal twin of [`Self::grid_top_inset`], and read the same way:
    /// from the framework's own `ShellGeometry::content_width` for the exact
    /// layout this app builds, resolved from the SAME [`shell_tokens()`] the
    /// shell is composed with. Re-deriving `width - sidebar - gap*3` locally
    /// would be a second copy of the shell's arithmetic that a retuned `gap`,
    /// `edge_inset` or `sidebar_width` would silently desynchronise — the
    /// hand-rolled layout the shell widget exists to delete.
    ///
    /// # Why the PTY needs this and not `content_size.width`
    ///
    /// The shell is told `cols` from this number. `content_inset` reports the
    /// floating bar's **overlap** of the content's top strip, which is the
    /// wrong axis for the sidebar: an inline sidebar does not float over the
    /// grid, it takes real horizontal space from it, so its width never appears
    /// in `content_inset`. Sizing `cols` off the window width therefore
    /// overstated the visible columns by ~260 px and cut off `btop`'s right
    /// side.
    ///
    /// It tracks the **resolved** presentation, so collapsing the sidebar to an
    /// overlay drawer (or hiding it) reports the reclaimed full width — which
    /// is why the three sidebar mutations now fire [`Self::resize_active_pty`].
    pub(crate) fn content_pane_width(&self) -> f32 {
        self.shell_geometry().content_width.max(0.0)
    }

    /// The terminal surface's inner inset, in logical px.
    ///
    /// Ordinary shells leave space between their text and the window edges.
    ///
    /// A **TUI owns the whole area**: `btop` draws its own frame and must reach
    /// the sides. The
    /// inset collapses to zero while the active pane is on the alternate
    /// screen, and that is not a paint-only switch: the same number is
    /// subtracted from the rect every geometry consumer reads
    /// ([`Self::terminal_text_width`] / [`Self::terminal_text_height`]), so the
    /// PTY's rows/cols, the scroll window, the pointer-to-cell mapping and the
    /// find/selection coordinates all agree with the pixels. Insetting the
    /// paint alone is how a terminal starts clipping its last row.
    ///
    /// It is applied inside [`grid_view::terminal_grid`]'s text container, not on
    /// the content pane: the pane also carries the overlay scrollbar lane
    /// (`enzui_widgets::enz_offset_scrollbar`), and insetting the pane would
    /// move the thumb 8 px inward, out of the grab band `scrollback_app_tests`
    /// measures. The lane stays on the pane edge; only the text moves.
    ///
    /// Grouped leaves use the same inset, on all four sides, and nothing else: a
    /// split leaf carries no per-leaf frame, radius or header band, so its text
    /// rect is its own allocation minus this inset, exactly as the unsplit one
    /// is.
    pub(crate) fn terminal_inset(&self) -> f32 {
        self.pane_inset(self.active_pane_is_alt_screen())
    }

    /// The inner inset of one leaf, which collapses to zero on the alternate
    /// screen so a TUI reaches every edge. Derived from that pane's own screen
    /// mode, not the active pane's, so a split with one shell and one TUI insets
    /// each leaf correctly.
    pub(crate) fn pane_inset(&self, alt_screen: bool) -> f32 {
        if alt_screen {
            0.0
        } else {
            self.enz_theme().spacing.sm()
        }
    }
}
