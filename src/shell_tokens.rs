//! Terminal shell-token constructor.

use enzui_theme::EnzTheme;
use enzui_widgets::app_shell::{PaneStyle, ShellTokens, SidebarHeader};

/// Terminal-specific surfaces and sidebar sizing. Band geometry is framework-owned.
pub(crate) fn shell_tokens(enz: &EnzTheme) -> ShellTokens {
    ShellTokens {
        sidebar_width: enz.sizing.sidebar_width,
        content_minimum: 360.0,
        sidebar_header: SidebarHeader::Inside,
        // The terminal canvas continues behind its transparent floating bar.
        content: PaneStyle::blended(),
        // The frame around the sidebar and the terminal is frosted glass
        // over the desktop where the compositor paints it.
        frame_material: Some(enzui_theme::MaterialStyle::FlatFrosted),
        ..ShellTokens::inset_panes()
    }
}

pub(crate) fn detached_shell_tokens() -> ShellTokens {
    ShellTokens::default()
        .edge_inset(0.0)
        .content_style(PaneStyle::blended())
}
