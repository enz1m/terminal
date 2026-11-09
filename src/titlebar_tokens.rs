//! Terminal titlebar token constructors.

use crate::grid_view;
use enzui_theme::Palette;
use enzui_widgets::accessibility_metrics::luminance;
use enzui_widgets::toolbar_icon::ToolbarIconTokens;

/// Toolbar-icon tokens for the icons that sit on the **transparent** titlebar
/// band.
///
/// # The measurement
///
/// The band is `SurfaceTone::Transparent` and `shell_tokens` makes the content
/// pane `PaneStyle::blended()`, so what is painted behind these icons is not a
/// theme surface at all: it is the terminal canvas, whose colour comes from the
/// user's colour scheme and is dark under every shipped scheme but Solarized
/// Light. A tint that followed the UI theme would be measured against the wrong
/// backdrop. Measured on a 375x812 capture against the canvas the icons sit on
/// (`(36,36,48)`, identical in both themes):
///
/// | icon | dark theme | light theme |
/// |---|---|---|
/// | `plus` | 6.61:1 | **2.72:1** |
/// | kebab | 7.32:1 | **2.92:1** |
///
/// WCAG 2.1 SC 1.4.11 asks 3:1 of a user-interface component, so the light
/// theme was not "a bit off": it failed, and it failed because the ink was
/// chosen against a surface that is not the one behind it.
///
/// # Why this is not an app-local colour
///
/// The ink is the framework's own
/// [`ToolbarIconTokens::from_palette`], resolved from the shipped palette
/// whose polarity matches the painted surface — the framework's existing
/// "icons over a dark surface" token set, not a literal this app invented
/// (the hand-roll guard would refuse one). The polarity is read from the
/// canvas with the framework's own WCAG [`luminance`], so a user who selects
/// **Solarized Light** flips the icons to dark ink instead of being handed
/// white-on-cream: the fix is a function of the surface, which is exactly why
/// a constant could not express it.
pub(crate) fn titlebar_icon_tokens() -> ToolbarIconTokens {
    titlebar_icon_tokens_for(grid_view::terminal_background())
}

/// The palette whose ink is legible on the painted terminal canvas, also used
/// for the titlebar find field.
///
/// The band is transparent, so "the surface behind this control" is the terminal
/// background, not the theme's surface. Resolved from the canvas rather than
/// from the theme for the same reason [`titlebar_icon_tokens_for`] is: a user on
/// Solarized Light with a dark app theme must still be able to read it.
pub(crate) fn terminal_canvas_palette() -> Palette {
    let behind = grid_view::terminal_background();
    if luminance(behind) < 0.179 {
        Palette::dark()
    } else {
        Palette::light()
    }
}

/// [`titlebar_icon_tokens`] as a function of the painted surface alone.
///
/// Split out so the *polarity* can be tested without touching the
/// process-global active colour scheme (which a parallel test would race):
/// the pixel test proves the tokens reach the rendered icons, this proves the
/// choice follows the surface rather than being "always white".
pub(crate) fn titlebar_icon_tokens_for(behind: iced::Color) -> ToolbarIconTokens {
    // 0.179 is the WCAG luminance at which black and white ink tie (both
    // exactly 4.5:1) — the standard "is this surface dark?" pivot, not a
    // tuned constant.
    let palette = if luminance(behind) < 0.179 {
        Palette::dark()
    } else {
        Palette::light()
    };
    ToolbarIconTokens::from_palette(&palette)
}
