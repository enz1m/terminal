//! Terminal dialog and host-row id constants.

/// The settings dialog's group title — also the accessible name of the
/// published `Role::Group`.
pub(crate) const SETTINGS_TITLE: &str = "Terminal settings";

/// The settings dialog's id namespace, shared by every derived control id
/// (the circular X, and the two pinned footer commands).
///
/// Named rather than repeated as a literal so `settings_dialog::close_button_id`
/// and `settings_dialog::footer_action_id` in the tests below cannot drift from
/// what the view actually builds — a literal repeated in a test is a
/// tautology (it passes with the view deleted).
pub(crate) const SETTINGS_ID_PREFIX: &str = "term-settings";

/// The stable id of the "Reset appearance" pinned footer command.
pub(crate) const SETTINGS_RESET_APPEARANCE_ID: &str = "reset-appearance";

/// The SSH connection manager dialog title.
pub(crate) const SSH_MANAGER_TITLE: &str = "Manage SSH hosts\u{2026}";
/// Stable scope shared by the manager host and its nested form presenter.
pub(crate) const SSH_MANAGER_SCOPE_KEY: &str = "terminal-ssh-manager";

// The routable identity namespaces for the SSH manager's host rows.
//
// Every id below is derived from a key that is stable for the LIFETIME of the
// thing it names: a saved host's `id` (`ssh-host-N`, minted once by
// `SaveSshConnection` and preserved across edits and renames) and an OpenSSH
// row's disambiguated alias. The list-row surface's default derivation would
// use the row's accessible *label*, which for a saved host is a renameable,
// non-unique display string — two hosts named `prod` would mint one id, and a
// routed AT focus request would reach whichever one iced visited first.
pub(crate) const SAVED_HOST_ROW_ID_PREFIX: &str = "ssh-host";
pub(crate) const SAVED_HOST_EDIT_ID_PREFIX: &str = "ssh-host-edit";
pub(crate) const SAVED_HOST_DELETE_ID_PREFIX: &str = "ssh-host-delete";
pub(crate) const OPENSSH_HOST_ROW_ID_PREFIX: &str = "openssh-host";
