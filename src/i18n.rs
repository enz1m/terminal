//! Bundled Fluent messages and the application's explicit language boundary.
use enzui_core::localization::{
    Arguments, Catalog, CatalogBuilder, Locale, Localizer, preferred_locales,
};
use std::sync::OnceLock;

fn catalog() -> Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let mut builder = CatalogBuilder::new();
            builder
                .add_resource("en-US", include_str!("../assets/locales/en-US.ftl"))
                .expect("valid bundled translations");
            builder
                .add_resource("es", include_str!("../assets/locales/es.ftl"))
                .expect("valid bundled translations");
            builder.build()
        })
        .clone()
}

#[cfg(test)]
pub(crate) fn for_locale(locale: &str) -> Localizer {
    let preferred = Locale::from_posix(locale).ok().into_iter().collect();
    Localizer::new(catalog(), preferred, Locale::default())
}

pub(crate) fn localizer() -> &'static Localizer {
    static LOCALIZER: OnceLock<Localizer> = OnceLock::new();
    LOCALIZER.get_or_init(|| Localizer::new(catalog(), preferred_locales(), Locale::default()))
}

pub(crate) fn text(source: &str) -> String {
    translate(localizer(), source)
}

pub(crate) fn translate(localizer: &Localizer, source: &str) -> String {
    match source_key(source) {
        Some(key) => localizer.text(key),
        None => source.to_owned(),
    }
}

pub(crate) fn format(
    localizer: &Localizer,
    source: &str,
    values: &[(&str, &dyn enzui_core::localization::MessageArgument)],
) -> String {
    let Some(key) = source_key(source) else {
        return source.to_owned();
    };
    let mut args = Arguments::new();
    for (name, value) in values {
        args.set(*name, value.message_value());
    }
    localizer.text_with(key, &args)
}

pub(crate) fn source_key(source: &str) -> Option<&'static str> {
    MESSAGES
        .iter()
        .find_map(|(en, key)| (*en == source).then_some(*key))
}

pub(crate) fn labels(localizer: &Localizer) -> std::collections::HashMap<&'static str, String> {
    MESSAGES
        .iter()
        .filter(|(source, _)| !source.contains('{'))
        .map(|(source, key)| (*source, localizer.text(key)))
        .collect()
}

// Static presentation getters share one bounded set of startup translations.
pub(crate) fn label(source: &'static str) -> &'static str {
    static LABELS: OnceLock<std::collections::HashMap<&'static str, String>> = OnceLock::new();
    LABELS
        .get_or_init(|| labels(localizer()))
        .get(source)
        .map_or(source, String::as_str)
}

#[rustfmt::skip]
const MESSAGES: &[(&str, &str)] = &[
    ("Running command", "terminal-shell-running-command"),
    (" · Agent is ready", "app-agent-is-ready-8175bb"),
    (" · Agent unavailable", "app-agent-unavailable-b1015c"),
    ("A connection is being saved. Wait for publication to finish before changing saved connections.", "app-a-connection-is-being-saved-wait-for-publication-to-finish-befor-050f3f"),
    ("A credential operation for this connection is still pending. Wait for it to finish before retrying.", "app-a-credential-operation-for-this-connection-is-still-pending-wait-f7d290"),
    ("A routed terminal action needs a shortcut; use Reset to restore its default.", "app-a-routed-terminal-action-needs-a-shortcut-use-reset-to-restore-i-56ab95"),
    ("A safe workspace was opened. Saved session data is protected and will not be overwritten.", "app-a-safe-workspace-was-opened-saved-session-data-is-protected-and--f42d19"),
    ("A saved connection ID is invalid. No connection was changed. Repair the saved profiles and restart before retrying.", "app-a-saved-connection-id-is-invalid-no-connection-was-changed-repai-8e02a4"),
    ("A saved connection operation is still pending. Wait for it to finish before retrying.", "app-a-saved-connection-operation-is-still-pending-wait-for-it-to-fin-7905e6"),
    ("Add a connection to save an SSH host.", "app-add-a-connection-to-save-an-ssh-host-1ec27b"),
    ("Add connection", "app-add-connection-685f88"),
    ("Agent", "app-agent-11b39c"),
    ("Agent is ready", "app-agent-is-ready-6e1953"),
    ("Appearance", "app-appearance-3907fa"),
    ("Auth mode", "app-auth-mode-0fdea2"),
    ("Background saving has not finished.", "app-background-saving-has-not-finished-c98e2c"),
    ("Backspace", "app-backspace-292fff"),
    ("Bar", "app-bar-95d64c"),
    ("Block", "app-block-211d0b"),
    ("Cancel", "app-cancel-19766e"),
    ("Cancel keyring request", "app-cancel-keyring-request-32912c"),
    ("Caps Lock", "app-caps-lock-71c8e9"),
    ("Choose how this connection authenticates.", "app-choose-how-this-connection-authenticates-8db3c9"),
    ("Clear saved output for this session", "app-clear-saved-output-for-this-session-d1881f"),
    ("Clipboard", "app-clipboard-285d69"),
    ("Close", "app-close-7d9eb7"),
    ("Close command history", "app-close-command-history-3bbc78"),
    ("Close find", "app-close-find-eb903c"),
    ("Close focused pane", "app-close-focused-pane-b4106d"),
    ("Close pane", "app-close-pane-7fa0f9"),
    ("Close selected pane", "app-close-selected-pane-14ca42"),
    ("Close session", "app-close-session-e50336"),
    ("Close tab", "app-close-tab-50a3f9"),
    ("Close without saving", "app-close-without-saving-bb5e6e"),
    ("Close {}", "app-close-78b32e"),
    ("Color scheme", "app-color-scheme-10ee94"),
    ("Command copied", "app-command-copied-e5c024"),
    ("Command history", "app-command-history-f5f0f8"),
    ("Command history unavailable: shell integration startup failed.", "app-command-history-unavailable-shell-integration-startup-failed-ca0b2f"),
    ("Command history unavailable: unsupported shell or invocation.", "app-command-history-unavailable-unsupported-shell-or-invocation-081844"),
    ("Confirm host key", "app-confirm-host-key-a469eb"),
    ("Connect", "app-connect-1a2303"),
    ("Connect to OpenSSH host {at_alias}", "app-connect-to-openssh-host-at-alias-e90c04"),
    ("Connect to SSH host {at_name}", "app-connect-to-ssh-host-at-name-71086c"),
    ("Connecting to the same SSH endpoint…", "app-connecting-to-the-same-ssh-endpoint-795b90"),
    ("Connecting to {dest}...", "app-connecting-to-dest-e49e4a"),
    ("Copy", "app-copy-e21f93"),
    ("Copy command", "app-copy-command-9a01fe"),
    ("Copy command: {}", "app-copy-command-3e7590"),
    ("Could not save settings: {error}", "app-could-not-save-settings-error-58d11d"),
    ("Cursor style", "app-cursor-style-4781e1"),
    ("Default connection profiles are in use. Saved settings are protected and will not be overwritten.", "app-default-connection-profiles-are-in-use-saved-settings-are-protec-8cea9c"),
    ("Default path (opens in this directory on the remote)", "app-default-path-opens-in-this-directory-on-the-remote-e8958d"),
    ("Default settings are in use. Saved settings are protected and will not be overwritten.", "app-default-settings-are-in-use-saved-settings-are-protected-and-wil-f8d0c3"),
    ("Delete", "app-delete-e2d0a5"),
    ("Delete SSH host {at_name}", "app-delete-ssh-host-at-name-7e3489"),
    ("Delete SSH host?", "app-delete-ssh-host-41a73f"),
    ("Detach tab", "app-detach-tab-fa98c3"),
    ("Down", "app-down-b86d11"),
    ("Edit", "app-edit-464c4f"),
    ("Edit SSH host", "app-edit-ssh-host-f66b11"),
    ("Edit SSH host {at_name}", "app-edit-ssh-host-at-name-0e1ed4"),
    ("End", "app-end-f4db1e"),
    ("Enter", "app-enter-dc8659"),
    ("Enter a port between 1 and 65535.", "app-enter-a-port-between-1-and-65535-8d698c"),
    ("Fill in the form below to add your first connection.", "app-fill-in-the-form-below-to-add-your-first-connection-cf4ad1"),
    ("Find in terminal", "app-find-in-terminal-382564"),
    ("Find in terminal…", "app-find-in-terminal-aa1607"),
    ("Find in this pane", "app-find-in-this-pane-cf795c"),
    ("Focus pane down", "app-focus-pane-down-634e72"),
    ("Focus pane left", "app-focus-pane-left-270312"),
    ("Focus pane right", "app-focus-pane-right-bf3836"),
    ("Focus pane up", "app-focus-pane-up-f0ad95"),
    ("Font size", "app-font-size-6d784d"),
    ("Group above", "app-group-above-8f5e74"),
    ("Group below", "app-group-below-d19f69"),
    ("Group into {}", "app-group-into-1c27f6"),
    ("Group to the left", "app-group-to-the-left-532c83"),
    ("Group to the right", "app-group-to-the-right-fabcb5"),
    ("History", "app-history-0e7696"),
    ("Home", "app-home-3a7869"),
    ("Host", "app-host-4a8231"),
    ("Host is required", "app-host-is-required-100ee5"),
    ("Host key not confirmed", "app-host-key-not-confirmed-dcb13c"),
    ("Insert", "app-insert-8aa318"),
    ("Jump to latest", "app-jump-to-latest-867524"),
    ("Jump to latest output", "app-jump-to-latest-output-416085"),
    ("Jump to tab 1", "app-jump-to-tab-1-92b8e8"),
    ("Jump to tab 2", "app-jump-to-tab-2-45a1c3"),
    ("Jump to tab 3", "app-jump-to-tab-3-a13617"),
    ("Jump to tab 4", "app-jump-to-tab-4-595679"),
    ("Jump to tab 5", "app-jump-to-tab-5-febd38"),
    ("Jump to tab 6", "app-jump-to-tab-6-6936ac"),
    ("Jump to tab 7", "app-jump-to-tab-7-3f64b7"),
    ("Jump to tab 8", "app-jump-to-tab-8-4d5dde"),
    ("Jump to tab 9", "app-jump-to-tab-9-e42bda"),
    ("Key file", "app-key-file-33c799"),
    ("Key passphrase", "app-key-passphrase-251003"),
    ("Left", "app-left-58eb90"),
    ("Loading OpenSSH hosts…", "app-loading-openssh-hosts-7b813a"),
    ("Loading saved workspace…", "app-loading-saved-workspace-2d9338"),
    ("Local", "app-local-8c31e6"),
    ("Local shell", "app-local-shell-37968e"),
    ("Local shell and SSH sessions", "app-local-shell-and-ssh-sessions-decfc3"),
    ("Manage SSH hosts", "app-manage-ssh-hosts-35c736"),
    ("Manage SSH hosts…", "app-manage-ssh-hosts-240e1d"),
    ("Move down", "app-move-down-40bb50"),
    ("Move up", "app-move-up-c66feb"),
    ("New local terminal", "app-new-local-terminal-28fbdc"),
    ("New shell", "app-new-shell-8a926e"),
    ("New tab", "app-new-tab-1e08fd"),
    ("Next", "app-next-1ff57a"),
    ("Next (Enter)", "app-next-enter-bd3738"),
    ("Next match", "app-next-match-825e5a"),
    ("Next tab", "app-next-tab-1ef952"),
    ("Next tab (alternate)", "app-next-tab-alternate-ed5c1c"),
    ("No OpenSSH config was found.", "app-no-openssh-config-was-found-d5dc74"),
    ("No SSH agent detected (SSH_AUTH_SOCK not set or socket missing)", "app-no-ssh-agent-detected-ssh-auth-sock-not-set-or-socket-missing-150c55"),
    ("No commands yet", "app-no-commands-yet-3ad0ab"),
    ("No desktop link opener is available.", "app-no-desktop-link-opener-is-available-35fd92"),
    ("No keyring service is available. Remembered SSH credentials are unavailable; nothing will be written to disk.", "app-no-keyring-service-is-available-remembered-ssh-credentials-are-u-5856cc"),
    ("No live terminal", "app-no-live-terminal-7d9af9"),
    ("No named OpenSSH hosts found.", "app-no-named-openssh-hosts-found-1c06ac"),
    ("No results", "app-no-results-a43619"),
    ("No saved hosts yet.", "app-no-saved-hosts-yet-c1415f"),
    ("No space left on device", "app-no-space-left-on-device-2ec76a"),
    ("Notes", "app-notes-8a7525"),
    ("Notes: {}", "app-notes-1b5889"),
    ("Only eligible settled output was retained; no process or credentials were resumed.", "app-only-eligible-settled-output-was-retained-no-process-or-credenti-9bac93"),
    ("Open a local shell", "app-open-a-local-shell-f52211"),
    ("Open settings", "app-open-settings-ca381c"),
    ("OpenSSH config", "app-openssh-config-77352c"),
    ("OpenSSH config could not be loaded.", "app-openssh-config-could-not-be-loaded-f1c976"),
    ("OpenSSH config could not be read because permission was denied.", "app-openssh-config-could-not-be-read-because-permission-was-denied-2c09d8"),
    ("OpenSSH config is invalid.", "app-openssh-config-is-invalid-13fa08"),
    ("OpenSSH discovery is unavailable. Reload Remote before reconnecting.", "app-openssh-discovery-is-unavailable-reload-remote-before-reconnecti-b8a190"),
    ("OpenSSH discovery is unavailable. Reload to retry.", "app-openssh-discovery-is-unavailable-reload-to-retry-401fdc"),
    ("OpenSSH discovery is unavailable: configuration limits were exceeded.", "app-openssh-discovery-is-unavailable-configuration-limits-were-excee-a6b1e2"),
    ("OpenSSH discovery is unavailable: the previous reader is still busy. Reload to retry.", "app-openssh-discovery-is-unavailable-the-previous-reader-is-still-bu-25d47f"),
    ("Opening links is not enabled for this build.", "app-opening-links-is-not-enabled-for-this-build-fc939b"),
    ("Opening the link was cancelled.", "app-opening-the-link-was-cancelled-2b896c"),
    ("Optional notes", "app-optional-notes-5a3bb5"),
    ("Overlays", "app-overlays-975c56"),
    ("Pane limit reached: at most 8 terminal surfaces can be rendered.", "app-pane-limit-reached-at-most-8-terminal-surfaces-can-be-rendered-858c37"),
    ("Pane limit reached: at most {} terminal surfaces can be rendered.", "app-pane-limit-reached-at-most-terminal-surfaces-can-be-rendered-ccb011"),
    ("Pane name", "app-pane-name-8fe253"),
    ("Panes", "app-panes-f86aa4"),
    ("Password", "app-password-e7cf3e"),
    ("Paste", "app-paste-f3380f"),
    ("Paste command into terminal: {}", "app-paste-command-into-terminal-985281"),
    ("Paste into terminal", "app-paste-into-terminal-e19e0f"),
    ("Per-tab command history", "app-per-tab-command-history-48a7c8"),
    ("Permission denied", "app-permission-denied-59b77d"),
    ("Permission to open the link was denied.", "app-permission-to-open-the-link-was-denied-0253a1"),
    ("Pin tab", "app-pin-tab-bf5083"),
    ("Pinned, output not saved securely", "app-pinned-output-not-saved-securely-7bf0dd"),
    ("Port", "app-port-72e9a5"),
    ("Preferences are unavailable. Saving is disabled until restart.", "app-preferences-are-unavailable-saving-is-disabled-until-restart-0dec46"),
    ("Preferences have not been loaded; saving is unavailable.", "app-preferences-have-not-been-loaded-saving-is-unavailable-c9daf0"),
    ("Preferences publication could not be confirmed. Credentials were not changed. Restore verified preferences and restart before retrying.", "app-preferences-publication-could-not-be-confirmed-credentials-were--d61c96"),
    ("Previous", "app-previous-a57b08"),
    ("Previous (Shift+Enter)", "app-previous-shift-enter-8ebef4"),
    ("Previous match", "app-previous-match-daa2f8"),
    ("Previous tab", "app-previous-tab-c88d5e"),
    ("Previous tab (alternate)", "app-previous-tab-alternate-f24250"),
    ("Re-dock", "app-re-dock-05861d"),
    ("Re-dock tab", "app-re-dock-tab-6e688d"),
    ("Re-dock the detached tab before grouping it into another session.", "app-re-dock-the-detached-tab-before-grouping-it-into-another-session-766b6c"),
    ("Re-dock {} into the main window", "app-re-dock-into-the-main-window-58a011"),
    ("Reconnect", "app-reconnect-bf8a9e"),
    ("Reload OpenSSH config", "app-reload-openssh-config-68eb6c"),
    ("Remembered in the system keyring. Leave empty to keep the saved one.", "app-remembered-in-the-system-keyring-leave-empty-to-keep-the-saved-o-2ea8be"),
    ("Remote sessions", "app-remote-sessions-9a3ac7"),
    ("Rename", "app-rename-3064d7"),
    ("Rename selected pane", "app-rename-selected-pane-273059"),
    ("Rename tab", "app-rename-tab-29365e"),
    ("Requested working directory is no longer available.", "app-requested-working-directory-is-no-longer-available-389948"),
    ("Reset appearance", "app-reset-appearance-54ba48"),
    ("Reset saved sessions", "app-reset-saved-sessions-aa01c9"),
    ("Restored", "app-restored-5d561a"),
    ("Restored output — process not live", "app-restored-output-process-not-live-b77dbf"),
    ("Retry cleanup", "app-retry-cleanup-8b02bf"),
    ("Retry keyring", "app-retry-keyring-2ee46b"),
    ("Retry saving", "app-retry-saving-3adee6"),
    ("Retry secure storage", "app-retry-secure-storage-e4aa43"),
    ("Right", "app-right-883361"),
    ("Run a command in the terminal and it appears here.", "app-run-a-command-in-the-terminal-and-it-appears-here-a142d3"),
    ("SSH connection cancelled or superseded. Open Remote to reconnect explicitly.", "app-ssh-connection-cancelled-or-superseded-open-remote-to-reconnect--0b71ce"),
    ("SSH connection cancelled. Open Remote to reconnect explicitly.", "app-ssh-connection-cancelled-open-remote-to-reconnect-explicitly-25f203"),
    ("SSH connection failed: {}", "app-ssh-connection-failed-f26d72"),
    ("SSH credential state is uncertain. Keep Terminal open and retry reconciliation.", "app-ssh-credential-state-is-uncertain-keep-terminal-open-and-retry-r-37fb5f"),
    ("SSH key path", "app-ssh-key-path-48e94a"),
    ("SSH restoration cancelled. Open Remote to reconnect explicitly.", "app-ssh-restoration-cancelled-open-remote-to-reconnect-explicitly-030f6e"),
    ("Save connection", "app-save-connection-df24ef"),
    ("Save failed", "app-save-failed-53ad6f"),
    ("Saved connection IDs are duplicated. No connection was changed. Repair the saved profiles and restart before retrying.", "app-saved-connection-ids-are-duplicated-no-connection-was-changed-re-43e6be"),
    ("Saved connection IDs are exhausted. No connection was changed. Cancel and restore valid saved profiles before retrying.", "app-saved-connection-ids-are-exhausted-no-connection-was-changed-can-13017a"),
    ("Saved connections", "app-saved-connections-0c6678"),
    ("Saved preferences are unavailable. No connection was changed. Restore readable preferences and restart before retrying.", "app-saved-preferences-are-unavailable-no-connection-was-changed-rest-8e285a"),
    ("Saved session data is still being written.", "app-saved-session-data-is-still-being-written-ac2007"),
    ("Saved sessions are protected; no process was started.", "app-saved-sessions-are-protected-no-process-was-started-fd0d14"),
    ("Saved state could not be opened", "app-saved-state-could-not-be-opened-e6ee25"),
    ("Saved state could not be understood", "app-saved-state-could-not-be-understood-a16a20"),
    ("Saved state requires explicit replacement authorization.", "app-saved-state-requires-explicit-replacement-authorization-14a329"),
    ("Saved working directory is invalid; started in the default directory.", "app-saved-working-directory-is-invalid-started-in-the-default-direct-47c733"),
    ("Saved working directory {} is unavailable; started in {}", "app-saved-working-directory-is-unavailable-started-in-b919c6"),
    ("Saving connection…", "app-saving-connection-af4349"),
    ("Saving connection… Other preference saves will wait. Cancelling cannot undo a publication already in progress.", "app-saving-connection-other-preference-saves-will-wait-cancelling-ca-005eff"),
    ("Saving is blocked to protect existing saved data", "app-saving-is-blocked-to-protect-existing-saved-data-b47fe4"),
    ("Saving is busy; this change has not been written yet.", "app-saving-is-busy-this-change-has-not-been-written-yet-1fbe92"),
    ("Saving stopped before this change was written.", "app-saving-stopped-before-this-change-was-written-423127"),
    ("Scrollback lines ({} – {})", "app-scrollback-lines-84bdec"),
    ("Secret Service denied access.", "app-secret-service-denied-access-c6b256"),
    ("Secret Service did not complete the operation.", "app-secret-service-did-not-complete-the-operation-a1edd7"),
    ("Secret Service is unavailable.", "app-secret-service-is-unavailable-f1868a"),
    ("Select a session", "app-select-a-session-c938f6"),
    ("Selected shell executable authority is unavailable.", "app-selected-shell-executable-authority-is-unavailable-14e65e"),
    ("Session actions", "app-session-actions-742ab4"),
    ("Session data could not be saved.", "app-session-data-could-not-be-saved-c28a5a"),
    ("Session name", "app-session-name-136a71"),
    ("Session name is required", "app-session-name-is-required-23fcf0"),
    ("Session state could not be saved", "app-session-state-could-not-be-saved-bd9583"),
    ("Sessions", "app-sessions-6fa3cb"),
    ("Settings", "app-settings-74a883"),
    ("Shift", "app-shift-2e544a"),
    ("Space", "app-space-20eac5"),
    ("Split down on the same endpoint", "app-split-down-on-the-same-endpoint-c9522f"),
    ("Split pane down", "app-split-pane-down-cbaec7"),
    ("Split pane down on the same endpoint", "app-split-pane-down-on-the-same-endpoint-b53978"),
    ("Split pane right", "app-split-pane-right-fee3f9"),
    ("Split pane right on the same endpoint", "app-split-pane-right-on-the-same-endpoint-222dad"),
    ("Split right on the same endpoint", "app-split-right-on-the-same-endpoint-b809b5"),
    ("Starting shell…", "app-starting-shell-12e49f"),
    ("Stored output removal is pending. {reason}", "app-stored-output-removal-is-pending-reason-c848eb"),
    ("Tab name", "app-tab-name-f21a01"),
    ("Tabs", "app-tabs-8e5ea5"),
    ("Terminal", "app-terminal-e0926f"),
    ("Terminal is settling credential changes for close. Keep this window open and retry afterward.", "app-terminal-is-settling-credential-changes-for-close-keep-this-wind-f03c34"),
    ("Terminal settings", "app-terminal-settings-138a36"),
    ("That input is too large to send to the terminal in one go.", "app-that-input-is-too-large-to-send-to-the-terminal-in-one-go-cb1c98"),
    ("That session is no longer open.", "app-that-session-is-no-longer-open-59266d"),
    ("The OpenSSH alias is missing or its config could not be read. This pane remains disconnected.", "app-the-openssh-alias-is-missing-or-its-config-could-not-be-read-thi-dc237c"),
    ("The OpenSSH config changed. Open Remote to connect explicitly; this pane was not retargeted.", "app-the-openssh-config-changed-open-remote-to-connect-explicitly-thi-773cda"),
    ("The background saver failed before this change was written.", "app-the-background-saver-failed-before-this-change-was-written-3e9f5a"),
    ("The connection being edited is no longer current. No connection was changed. Reopen the connection and retry.", "app-the-connection-being-edited-is-no-longer-current-no-connection-w-f02edc"),
    ("The connection form is unavailable. Reopen it and retry.", "app-the-connection-form-is-unavailable-reopen-it-and-retry-f00746"),
    ("The connection is invalid. Correct the form and retry.", "app-the-connection-is-invalid-correct-the-form-and-retry-8e447b"),
    ("The connection was not published. Saved profiles and credentials are unchanged. Retry Save connection.", "app-the-connection-was-not-published-saved-profiles-and-credentials--f495b5"),
    ("The keyring remained locked.", "app-the-keyring-remained-locked-2e3a8f"),
    ("The link could not be opened.", "app-the-link-could-not-be-opened-13ba7c"),
    ("The link opener lost its parent window.", "app-the-link-opener-lost-its-parent-window-eabf86"),
    ("The metadata checkpoint could not be published.", "app-the-metadata-checkpoint-could-not-be-published-c55295"),
    ("The new connection ID is already in use. No connection was changed. Retry saving the connection.", "app-the-new-connection-id-is-already-in-use-no-connection-was-change-46dc06"),
    ("The saved SSH endpoint or authentication changed. Open Remote to connect explicitly; this pane was not retargeted.", "app-the-saved-ssh-endpoint-or-authentication-changed-open-remote-to--bc24bc"),
    ("The saved SSH profile was deleted. Open Remote to choose a new connection.", "app-the-saved-ssh-profile-was-deleted-open-remote-to-choose-a-new-co-9ba634"),
    ("The saved connection limit has been reached. Remove a connection before retrying.", "app-the-saved-connection-limit-has-been-reached-remove-a-connection--413a90"),
    ("The saved secret for {} was never sent: {endpoint} is not a confirmed host in known_hosts. Confirm this host's fingerprint before connecting this profile again; nothing was accepted on your behalf.", "app-the-saved-secret-for-was-never-sent-endpoint-is-not-a-confirmed--5f7f0d"),
    ("The secure operation was cancelled.", "app-the-secure-operation-was-cancelled-479803"),
    ("The secure snapshot is missing, stale, corrupt or oversized.", "app-the-secure-snapshot-is-missing-stale-corrupt-or-oversized-b16fa5"),
    ("The selected file could not be found.", "app-the-selected-file-could-not-be-found-5afdaa"),
    ("The selected file could not be opened because permission was denied.", "app-the-selected-file-could-not-be-opened-because-permission-was-den-37db66"),
    ("The selected file could not be opened.", "app-the-selected-file-could-not-be-opened-b54498"),
    ("The system refused to start the terminal process.", "app-the-system-refused-to-start-the-terminal-process-a63ed4"),
    ("The terminal cannot start another session until the application is restarted.", "app-the-terminal-cannot-start-another-session-until-the-application--c9f0fe"),
    ("The terminal is still sending earlier input. Try again in a moment.", "app-the-terminal-is-still-sending-earlier-input-try-again-in-a-momen-3d8145"),
    ("The terminal output reader could not start.", "app-the-terminal-output-reader-could-not-start-6b09dc"),
    ("The terminal session encountered an error.", "app-the-terminal-session-encountered-an-error-268400"),
    ("The terminal session is closed.", "app-the-terminal-session-is-closed-3bd3a8"),
    ("The terminal start-up worker stopped.", "app-the-terminal-start-up-worker-stopped-56e280"),
    ("Theme", "app-theme-efb52e"),
    ("There is no live credential owner. Open a terminal and retry saving the connection.", "app-there-is-no-live-credential-owner-open-a-terminal-and-retry-savi-1bd7e5"),
    ("This desktop does not support opening links.", "app-this-desktop-does-not-support-opening-links-3f5d70"),
    ("This link is not a valid address.", "app-this-link-is-not-a-valid-address-23f424"),
    ("This pane is a local shell, not an SSH endpoint.", "app-this-pane-is-a-local-shell-not-an-ssh-endpoint-bef159"),
    ("This pane requires a same-endpoint SSH split, not a local shell.", "app-this-pane-requires-a-same-endpoint-ssh-split-not-a-local-shell-a7cbed"),
    ("This removes the saved connection. This cannot be undone.", "app-this-removes-the-saved-connection-this-cannot-be-undone-e9c5b5"),
    ("This window cannot close yet", "app-this-window-cannot-close-yet-45bdcd"),
    ("This workspace has no per-pane SSH snapshot. Open Remote and connect explicitly; the old endpoint cannot be inferred safely.", "app-this-workspace-has-no-per-pane-ssh-snapshot-open-remote-and-conn-aa0c59"),
    ("Toast", "app-toast-34b860"),
    ("Toggle sidebar", "app-toggle-sidebar-041aef"),
    ("Too many terminal sessions are still shutting down. Close one and retry.", "app-too-many-terminal-sessions-are-still-shutting-down-close-one-and-a25a2b"),
    ("Type to search this session", "app-type-to-search-this-session-887f3b"),
    ("Unavailable", "app-unavailable-ca1844"),
    ("Underline", "app-underline-02f843"),
    ("Ungroup", "app-ungroup-b57ef4"),
    ("Ungroup selected pane", "app-ungroup-selected-pane-fb5e63"),
    ("Ungroup {} from {}", "app-ungroup-from-362247"),
    ("Unpin tab", "app-unpin-tab-20553d"),
    ("Up", "app-up-55490a"),
    ("Use Reconnect or New shell before splitting this restored pane.", "app-use-reconnect-or-new-shell-before-splitting-this-restored-pane-590bdb"),
    ("Username", "app-username-e3b89e"),
    ("Window", "app-window-19734a"),
    ("Working directory unavailable", "app-working-directory-unavailable-704f4d"),
    ("cancelled", "app-cancelled-8b4704"),
    ("copy", "app-copy-6f5a60"),
    ("delete", "app-delete-619759"),
    ("e.g. /var/www", "app-e-g-var-www-4472f3"),
    ("e.g. production-server", "app-e-g-production-server-eebf5b"),
    ("failed", "app-failed-5d28a9"),
    ("hostname or IP", "app-hostname-or-ip-2a8f86"),
    ("{} terminal pane, {}", "app-terminal-pane-2faf0f"),
    ("{}. No connection was changed. Retry Save connection.", "app-no-connection-was-changed-retry-save-connection-7ea2f0"),
    ("Waiting for the system keyring…", "app-waiting-for-the-system-keyring-c0cf76"),
    ("e.g. ubuntu", "app-e-g-ubuntu-609eda"),
    ("Secure output removal is pending.", "app-secure-output-removal-is-pending-73c10e"),
];

#[cfg(test)]
pub(crate) fn test_display_text(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

// Only visible-text comparisons ignore Fluent isolation; production keeps formatted text opaque.
#[cfg(test)]
pub(crate) fn test_nodes(
    mut nodes: Vec<enzui_core::collect::CollectedNode>,
) -> Vec<enzui_core::collect::CollectedNode> {
    for node in &mut nodes {
        node.semantics.label = node.semantics.label.as_deref().map(test_display_text);
        node.semantics.description = node.semantics.description.as_deref().map(test_display_text);
    }
    nodes
}
