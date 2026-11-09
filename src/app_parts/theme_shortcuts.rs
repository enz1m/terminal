//! Terminal theme and shortcut methods.

use super::*;

impl App {
    pub(crate) fn theme(&self) -> iced::Theme {
        self.themes.theme().iced_theme()
    }

    pub(crate) fn enz_theme(&self) -> &EnzTheme {
        self.themes.theme()
    }

    pub(crate) fn geometry(&self) -> CellGeometry {
        CellGeometry::from_font(
            self.settings.resolved_font_size_px(),
            self.terminal_family(),
        )
    }

    /// The monospace family the terminal measures and draws with, by name:
    /// resolved from this app's theme when the theme changes, so geometry and
    /// the view always agree and a process-wide font remap mid-frame cannot
    /// split them.
    pub(crate) fn terminal_family(&self) -> Option<String> {
        self.terminal_font_family.clone()
    }

    /// Resolves [`Self::terminal_family`] from the current theme.
    pub(crate) fn resolve_terminal_family(&mut self) {
        self.terminal_font_family = Some(crate::terminal_view::concrete_monospace(
            crate::terminal_view::theme_monospace(self.enz_theme()),
        ));
    }

    pub(crate) fn shortcut_defaults() -> Vec<ShortcutBinding<String>> {
        keybindings::ALL_ACTIONS
            .iter()
            // `clipboard.paste` and `clipboard.copy` have no `action_message`
            // (they dispatch to async clipboard tasks, not plain reducers) but
            // are honoured through interceptions in the key handler, so they
            // belong in the resolved catalog the dispatcher and the Shortcuts
            // dialog both read.
            .filter(|action| {
                Self::action_message(action.id).is_some()
                    || action.id == "clipboard.paste"
                    || action.id == "clipboard.copy"
            })
            .map(|action| {
                ShortcutBinding::new(
                    action.id,
                    action.id.to_owned(),
                    keybindings::framework_shortcut(action.default_chord).unwrap_or_else(|| {
                        panic!("invalid shipped shortcut {}", action.default_chord)
                    }),
                )
                .with_description(crate::i18n::label(action.label))
                .with_section(crate::i18n::label(action.category))
            })
            .collect()
    }

    pub(crate) fn shortcut_label(&self, action: &str) -> String {
        self.shortcuts
            .resolved()
            .iter()
            .find(|binding| binding.id() == action)
            .map(|binding| format!("{} ({})", binding.description(), binding.primary()))
            .unwrap_or_default()
    }

    /// The chord bound to `action`, for a menu's shortcut hint.
    pub(crate) fn shortcut_chord(&self, action: &str) -> Option<String> {
        self.shortcuts
            .resolved()
            .iter()
            .find(|binding| binding.id() == action)
            .map(|binding| binding.primary().to_string())
    }

    pub(crate) fn shortcut_action_for_key(
        &self,
        key: &keyboard::Key,
        mods: Modifiers,
    ) -> Option<String> {
        self.shortcuts
            .resolved()
            .iter()
            .find(|binding| {
                binding
                    .shortcuts()
                    .any(|shortcut| shortcut.matches(key, mods))
            })
            .map(|binding| binding.command().clone())
    }

    pub(crate) fn validate_shortcut(candidate: &Shortcut) -> ShortcutValidity {
        if candidate.modifiers() == Modifier::NONE
            && matches!(candidate.key(), ShortcutKey::Character(_))
        {
            return ShortcutValidity::Invalid(
                "unmodified printable keys belong to the terminal".into(),
            );
        }
        if candidate.modifiers().contains(Modifier::CTRL)
            && candidate.key().as_named() == Some(NamedKey::F6)
        {
            return ShortcutValidity::Reserved(
                "Ctrl+F6 and Ctrl+Shift+F6 move focus through window chrome".into(),
            );
        }
        ShortcutValidity::Valid
    }

    pub(crate) fn rebind_shortcut(
        &mut self,
        action: &str,
        candidate: Shortcut,
    ) -> Result<(), RebindRefusal> {
        self.shortcuts
            .try_rebind(action, candidate, Self::validate_shortcut)
    }
}
