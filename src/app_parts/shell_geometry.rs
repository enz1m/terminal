//! Terminal shell-geometry method.

use super::*;

impl App {
    /// The framework's resolved geometry for the exact shell this app composes.
    ///
    /// One call site for the layout/tokens/breakpoints/width tuple, so
    /// [`Self::grid_top_inset`] and [`Self::content_pane_width`] cannot drift
    /// apart: a test that pins one axis against the laid-out tree is then
    /// pinning the same resolution the other axis reads.
    pub(crate) fn shell_geometry(&self) -> enzui_widgets::app_shell::ShellGeometry {
        use enzui_widgets::app_shell::resolve_shell_geometry;
        let enz = self.enz_theme();
        resolve_shell_geometry(
            ShellLayout::FloatingTitlebarSidebar,
            self.nav_split,
            shell_tokens(enz),
            enz.breakpoints,
            self.content_size.width,
        )
    }
}
