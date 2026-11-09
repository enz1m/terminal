//! Terminal shell query methods.

use super::*;

impl App {
    /// The link authority of the pane's live screen.
    ///
    /// Zero when the pane has no running session, which is what retires the
    /// links of a closed or replaced pane's retained transcript.
    pub(crate) fn live_link_generation(&self, pane_id: &str) -> u64 {
        #[cfg(test)]
        if let Some(generation) = self.link_authority_override {
            return generation;
        }
        self.ptys
            .get(pane_id)
            .map_or(0, pty::PtySession::link_generation)
    }

    // ----- session restore / seeding ---------------------------------------

    /// The presentation the shell actually renders, resolved the SAME way the
    /// view does: the `< 640` force wins (matching `sidebar_force`), otherwise
    /// the framework `resolve_for_width` decides from the tracked width and the
    /// shell geometry. Routing the toggle through this is what stops a narrow
    /// (Overlay) toggle wrongly flipping the *preference* instead of opening
    /// the drawer — the nav-split `toggle_for_presentation` contract.
    pub(crate) fn effective_presentation(&self) -> enzui_widgets::nav_split::SidebarPresentation {
        use enzui_widgets::nav_split::{NavSplitGeometry, SidebarWidth, resolve_for_width};
        if let Some(forced) = self.sidebar_force() {
            return forced;
        }
        let geom = NavSplitGeometry::new(self.content_size.width)
            .with_sidebar(SidebarWidth::new(260.0))
            .with_content_minimum(360.0);
        resolve_for_width(
            self.nav_split.preference,
            self.enz_theme().breakpoints,
            geom,
        )
    }

    /// The `Message` a catalog action id maps to, or `None` when the action
    /// has no reducer in this build.
    ///
    /// The missing half of the keybinding dispatcher. Returning `None`
    /// (rather than a no-op) is deliberate: an unmapped chord must fall
    /// through to the PTY exactly as it did before, so binding
    /// `pane.focus-left` to `Ctrl+Alt+Left` cannot silently eat that key from
    /// the shell.
    pub(crate) fn action_message(action_id: &str) -> Option<Message> {
        Some(match action_id {
            "tab.new" => Message::NewLocal,
            "tab.close" => Message::CloseActive,
            // Session switching by keyboard — ordinary window
            // management the app already has sessions for. next/prev wrap;
            // the `-alt` chords are the same action under a second chord.
            "tab.next" | "tab.next-alt" => Message::SelectNextSession,
            "tab.prev" | "tab.prev-alt" => Message::SelectPrevSession,
            // goto-N is 1-indexed over the tabs the main window shows; past the
            // last of them, Alt+9 with three tabs open changes nothing.
            "tab.goto-1" => Message::SelectDockedSession(0),
            "tab.goto-2" => Message::SelectDockedSession(1),
            "tab.goto-3" => Message::SelectDockedSession(2),
            "tab.goto-4" => Message::SelectDockedSession(3),
            "tab.goto-5" => Message::SelectDockedSession(4),
            "tab.goto-6" => Message::SelectDockedSession(5),
            "tab.goto-7" => Message::SelectDockedSession(6),
            "tab.goto-8" => Message::SelectDockedSession(7),
            "tab.goto-9" => Message::SelectDockedSession(8),
            // Keep the persisted action id/chord, but honor its advertised
            // target: F2 renames the tab/session, never the active pane.
            "tab.rename" => Message::BeginSessionRename,
            "pane.split-horizontal" => Message::NewPane,
            "pane.split-vertical" => Message::NewPaneDown,
            "pane.close" => Message::ClosePane,
            "pane.focus-left" => Message::FocusPaneDir(pane_layout::FocusDir::Left),
            "pane.focus-right" => Message::FocusPaneDir(pane_layout::FocusDir::Right),
            "pane.focus-up" => Message::FocusPaneDir(pane_layout::FocusDir::Up),
            "pane.focus-down" => Message::FocusPaneDir(pane_layout::FocusDir::Down),
            "overlay.find" => Message::OpenFind,
            "overlay.history" => Message::ToggleHistory,
            "overlay.settings" => Message::OpenSettings,
            "overlay.ssh" => Message::OpenSshManager,
            "ui.toggle-sidebar" => Message::ToggleSidebar,
            // clipboard.paste is handled by the `RequestPaste` interception in
            // the key handler (an async clipboard read Task), not here, so it
            // has no `action_message`. clipboard.copy is handled identically:
            // it dispatches to an async clipboard write through the same
            // interception path.
            _ => return None,
        })
    }

    pub(crate) fn toggle_sidebar(&mut self) {
        // Presentation-aware: overlay flips the drawer, inline/hidden flip the
        // preference. Without this the mobile toggle only ever flipped the
        // preference and could never OPEN the overlay drawer (the sidebar was
        // unreachable on mobile).
        let presentation = self.effective_presentation();
        self.nav_split.toggle_for_presentation(presentation);
    }

    /// Reveal the sidebar-hosted tab-name editor without toggling an already
    /// visible surface closed. Wide layouts need the preference set to Shown;
    /// compact layouts additionally need their overlay drawer opened.
    pub(crate) fn ensure_sidebar_visible_for_rename(&mut self) {
        use enzui_widgets::nav_split::{SidebarPreference, SidebarPresentation};

        self.nav_split.preference = SidebarPreference::Shown;
        if self.effective_presentation() == SidebarPresentation::Overlay {
            self.nav_split.overlay_open = true;
        }
    }

    /// The forced sidebar presentation, driven from the tracked window width.
    ///
    /// Forcing is used ONLY for the pre-configure frame, where
    /// `content_size.width` is still 0 (the compositor has not yet reported a
    /// real size and winit's default is stale). There the shell is forced to
    /// `SidebarPresentation::Hidden` so the first painted frame never flashes
    /// a sidebar at the wrong (stale) width.
    ///
    /// For every real width the return is `None`, letting the nav-split widget
    /// resolve from its **live** layout constraints — Inline when the sidebar
    /// fits, Overlay (a togglable drawer) when it does not. An earlier version
    /// force-Hid the sidebar below 640px, which made the mobile sidebar
    /// *unreachable*: forced-Hidden removes the drawer entirely, so the toggle
    /// had nothing to open. Letting the widget resolve gives Overlay at narrow
    /// widths, which is what the toggle opens.
    ///
    /// Extracted from `view` so the decision is testable through the consumer
    /// without rendering.
    pub(crate) fn sidebar_force(&self) -> Option<enzui_widgets::nav_split::SidebarPresentation> {
        use enzui_widgets::nav_split::SidebarPresentation;
        if self.content_size.width <= 0.5 {
            Some(SidebarPresentation::Hidden)
        } else {
            None
        }
    }
}
