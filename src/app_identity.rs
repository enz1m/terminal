//! Terminal application identity and window settings.

use enzui_widgets::app_shell::{DesktopApplicationIdentity, ShellLayout, enz_window_settings_for};

pub(crate) const APP_ID: &str = "io.github.enzui.Terminal";

pub(crate) fn app_identity() -> &'static DesktopApplicationIdentity {
    static IDENTITY: std::sync::OnceLock<DesktopApplicationIdentity> = std::sync::OnceLock::new();
    IDENTITY.get_or_init(|| {
        DesktopApplicationIdentity::new(APP_ID)
            .expect("BUG: APP_ID must be a portable desktop-entry basename")
    })
}

pub(crate) fn main_window_settings() -> iced::window::Settings {
    let mut settings = app_identity().apply_to_window_settings(enz_window_settings_for(&[
        ShellLayout::FloatingTitlebarSidebar,
        ShellLayout::TitlebarSidebarContent,
    ]));
    // Close requests are reviewed by this app: nothing is destroyed until the
    // durable state the process owes has settled.
    settings.exit_on_close_request = enzui_core::close_guard::EXIT_ON_CLOSE_REQUEST;
    #[cfg(feature = "native-invalidation-acceptance")]
    if std::env::var_os(enzui_core::app_runtime::native_invalidation::CONTROL_PATH_ENV).is_some() {
        settings.size = iced::Size::new(580.0, 660.0);
    }
    settings
}
