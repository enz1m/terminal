//! Terminal surface geometry methods.

use super::*;

impl App {
    /// The size the terminal surface is laid out into: the content pane's
    /// width and the pane grid's real allocated height. With a split this is
    /// the extent the framework pane grid divides.
    ///
    /// The height is the pane grid's measured allocation when `view` has laid it
    /// out ([`Self::grid_area`]), so a rename or error banner that consumes rows
    /// above the grid shortens the terminal instead of the shell over-reporting
    /// the full window height. It falls back to the analytic height before the
    /// first layout.
    pub(crate) fn terminal_surface_size(&self) -> iced::Size {
        let height = self
            .grid_area
            .get()
            .map(|area| area.height)
            .filter(|height| *height > 0.0)
            .unwrap_or_else(|| self.terminal_available_height());
        iced::Size::new(self.content_pane_width(), height)
    }

    /// The rectangle one pane is laid out into, from the framework pane grid's
    /// own region arithmetic.
    ///
    /// `EnzPaneGridState::pane_rects` reports the seam already removed and the
    /// first child's extent rounded as the widget lays it out. A split ratio of
    /// the content width instead over-reports each leaf by its share of the
    /// seam, enough at a 9 px cell to hand the PTY a column the pane cannot
    /// paint.
    ///
    /// While a pointer drag is live the rects come from the
    /// persisted `layout` (the committed ratios) rather than the pane-grid
    /// widget, whose seam moves per pixel until release. The two are identical
    /// at rest, so at-rest geometry is unchanged; during a drag the committed
    /// rect keeps each leaf's columns, rows and scroll window stable — the text
    /// stays wrapped and windowed for the geometry the PTY still holds — while
    /// the leaf's own clip hides the frozen content the moving seam sweeps
    /// over.
    pub(crate) fn pane_surface_size(&self, pane_id: &str) -> iced::Size {
        let surface = self.terminal_surface_size();
        let Some(session) = self.sessions.get(self.active) else {
            return surface;
        };
        // One pane, or the narrow rung that shows one leaf at a time: the
        // surface is the content area, with no grid in between.
        if session.panes.len() == 1
            || pane_layout::is_narrow_layout_with(
                self.content_pane_width(),
                session.panes.len(),
                &self.enz_theme().sizing,
            )
        {
            return surface;
        }
        if session.pane_grid.pointer_resize_active() {
            return session
                .layout
                .pane_grid_state()
                .pane_rects(surface)
                .into_iter()
                .find(|(id, _)| id.as_str() == pane_id)
                .map_or(surface, |(_, rect)| {
                    iced::Size::new(rect.width, rect.height)
                });
        }
        session
            .pane_grid
            .pane_rects(surface)
            .into_iter()
            .find(|(id, _)| id.as_str() == pane_id)
            .map_or(surface, |(_, rect)| {
                iced::Size::new(rect.width, rect.height)
            })
    }

    /// The rect the terminal text of one pane lives in: that pane's allocation
    /// minus the inner inset on all four sides.
    ///
    /// Shared by paint, PTY columns/rows, the scroll window and the
    /// pointer-to-cell mapping, so none of them can disagree with the pixels.
    pub(crate) fn pane_text_size(&self, pane_id: &str) -> iced::Size {
        let alt_screen = self
            .sessions
            .get(self.active)
            .and_then(|session| session.panes.iter().find(|pane| pane.id == pane_id))
            .is_some_and(|pane| pane.alt_screen);
        let inset = self.pane_inset(alt_screen);
        let rect = self
            .restored_surface_size(pane_id)
            .unwrap_or_else(|| self.pane_surface_size(pane_id));
        iced::Size::new(
            (rect.width - 2.0 * inset).max(0.0),
            (rect.height - 2.0 * inset).max(0.0),
        )
    }

    /// [`Self::pane_text_size`] for the pane the user is typing into.
    pub(crate) fn active_pane_text_size(&self) -> iced::Size {
        match self.active_pane_id() {
            Some(id) => self.pane_text_size(&id),
            None => {
                let inset = self.terminal_inset();
                let surface = self.terminal_surface_size();
                iced::Size::new(
                    (surface.width - 2.0 * inset).max(0.0),
                    (surface.height - 2.0 * inset).max(0.0),
                )
            }
        }
    }

    /// The width of the content rect the active pane's terminal text lives in.
    ///
    /// `cols_for` callers read this rather than [`Self::content_pane_width`],
    /// which remains the layout width the shell breakpoints resolve against, so
    /// a rung does not flip when the text gains breathing room. With a split it
    /// is the focused leaf's rect, so the reducer's scroll page and the painted
    /// leaf agree.
    pub(crate) fn terminal_text_width(&self) -> f32 {
        self.active_pane_text_size().width
    }

    /// The height of the same rect: the pane's own allocation minus the inner
    /// inset top and bottom.
    pub(crate) fn terminal_text_height(&self) -> f32 {
        self.active_pane_text_size().height
    }

    /// Content-column height. The shell's floating-titlebar inset is handled
    /// separately.
    ///
    /// Find no longer subtracts anything here. The inline find
    /// row is gone — the field is in the titlebar band, which the shell already
    /// reserves — so searching neither shortens the grid nor resizes the PTY.
    pub(crate) fn terminal_layout_height(&self) -> f32 {
        self.content_size.height.max(0.0)
    }

    /// Unobstructed height available to the terminal grid and therefore PTY.
    pub(crate) fn terminal_available_height(&self) -> f32 {
        (self.terminal_layout_height() - self.grid_top_inset()).max(0.0)
    }
}
