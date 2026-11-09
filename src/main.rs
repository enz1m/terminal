//! terminal-emulator — enzUI-ICED application.
//!
//! A cmux-style tabbed terminal: a left sidebar of named sessions beside a live
//! PTY-backed terminal content area. Built on the iced-based enzUI framework.
//!
// A split reducer with self-delegating stubs recurses until the stack
// overflows, and rustc only warns about that by default.
#![deny(unconditional_recursion)]
#![cfg_attr(
    test,
    allow(
        dead_code,
        clippy::missing_assert_message,
        clippy::unwrap_in_result,
        clippy::unwrap_used,
        clippy::panic,
        reason = "Test builds retain runtime endpoints and report failed IO and invariants through assertions."
    )
)]

mod i18n;

mod app_identity;

mod launch_directory;
pub(crate) use app_identity::{APP_ID, app_identity, main_window_settings};
mod blocking_dispatch;

mod capture_keyring;
mod capture_seed_data;
mod color_scheme;
mod context_menu;
mod dialog_ids;
pub(crate) use dialog_ids::{
    OPENSSH_HOST_ROW_ID_PREFIX, SAVED_HOST_DELETE_ID_PREFIX, SAVED_HOST_EDIT_ID_PREFIX,
    SAVED_HOST_ROW_ID_PREFIX, SETTINGS_ID_PREFIX, SETTINGS_RESET_APPEARANCE_ID, SETTINGS_TITLE,
    SSH_MANAGER_SCOPE_KEY, SSH_MANAGER_TITLE,
};
mod detached_window;
pub(crate) use detached_window::{CloseIntent, DetachedViewport, DetachedWindow, TeardownDebt};
mod endpoint_connections;
mod escape_policy;
mod failure_translation;
mod focus_messages;
pub(crate) use focus_messages::{
    SessionRowFocusTarget, chrome_focus_message, restore_session_row_focus_task,
    window_focus_message,
};
mod focus_ids;
pub(crate) use focus_ids::{
    TITLEBAR_OVERFLOW_MENU_ID_PREFIX, find_field_id, focus_find_task, release_chrome_focus_task,
    settings_button_id, titlebar_overflow_button_id,
};

mod git_branch;
mod grid_view;
mod host_rows;
mod terminal_view;
pub(crate) use host_rows::{openssh_host_at_name, saved_host_at_name};
mod host_trust;
mod input_surface;
mod keybindings;
mod keymap;
mod labels;
mod native_drop;
pub(crate) use labels::{
    FIND_EMPTY_STATUS, FIND_PLACEHOLDER, HISTORY_COPY_TOAST_KEY, HISTORY_LIST_ID,
    HISTORY_ROW_PITCH, SIDEBAR_SSH_LABEL, history_viewport_height,
};
mod link_open;
mod modal_focus;
mod model;
mod mouse;
#[cfg(feature = "native-invalidation-acceptance")]
mod native_invalidation;
mod operation_origin;
mod palette;
mod pane_endpoint;
mod pointer_probe;
pub(crate) use pointer_probe::GridPointerProbe;
mod pane_fixtures;
pub(crate) use pane_fixtures::{failed_pane, scripted_pane};
mod pane_layout;

mod persistence_runtime;
mod pinned_output;
mod pinned_secret;
mod pinned_sessions;
mod pinned_storage;
mod pty;
mod scrollback;
mod selection;
mod session_row;
mod session_store;

mod settings_texts;
pub(crate) use settings_texts::{
    APPEARANCE_TITLE, CONFIRM_DELETE_SSH_BODY, CONFIRM_DELETE_SSH_TITLE, SETTINGS_ROWS,
    settings_row,
};
mod settings_store;
mod settings_surface;
mod shell_activity;
mod shell_history;
mod shell_status;
pub(crate) use shell_status::{
    SHELL_EXITED_PREFIX, SHELL_SPAWN_FAILED_PREFIX, find_enter_is_handled_globally, paste_bytes,
};
mod shell_tokens;
pub(crate) use shell_tokens::shell_tokens;
mod emulator;
#[cfg(all(test, target_os = "linux"))]
mod shell_startup_tests;
mod sidebar_session_tree;
mod ssh_config;
mod ssh_credentials;
mod ssh_discovery;
mod ssh_discovery_app;
mod ssh_host_ids;
mod ssh_host_save;

mod titlebar_tokens;
pub(crate) use titlebar_tokens::{terminal_canvas_palette, titlebar_icon_tokens};
mod window_input;
mod window_input_view;
mod window_notices;

#[path = "app_parts/active_pane.rs"]
mod active_pane;
#[path = "app_parts/active_pane_split.rs"]
mod active_pane_split;
#[cfg(feature = "center")]
mod center;
#[path = "app_parts/chrome_geometry.rs"]
mod chrome_geometry;
#[path = "app_parts/close_methods.rs"]
mod close_methods;
#[path = "app_parts/context_row.rs"]
mod context_row;
#[path = "app_parts/credential_mutation.rs"]
mod credential_mutation;
#[path = "app_parts/cwd_menu.rs"]
mod cwd_menu;
#[path = "app_parts/detached_endpoint.rs"]
mod detached_endpoint;
#[path = "app_parts/detached_pane.rs"]
mod detached_pane;
#[path = "app_parts/detached_pane_view.rs"]
mod detached_pane_view;
#[path = "app_parts/detached_view.rs"]
mod detached_view;
#[path = "app_parts/exit_install.rs"]
mod exit_install;
#[path = "app_parts/find_matches.rs"]
mod find_matches;
#[path = "app_parts/focus_pointer.rs"]
mod focus_pointer;
#[path = "app_parts/form_dialog_width.rs"]
mod form_dialog_width;
#[path = "app_parts/geometry_detach.rs"]
mod geometry_detach;
#[path = "app_parts/jump_transcript.rs"]
mod jump_transcript;
#[path = "app_parts/link_methods.rs"]
mod link_methods;
#[path = "app_parts/message.rs"]
mod message;
#[path = "app_parts/message_impl.rs"]
mod message_impl;
#[path = "app_parts/mouse_cells.rs"]
mod mouse_cells;
#[path = "app_parts/orphan_query.rs"]
mod orphan_query;
#[path = "app_parts/pty_bytes.rs"]
mod pty_bytes;
#[path = "app_parts/pty_drain.rs"]
mod pty_drain;
#[path = "app_parts/pty_resize.rs"]
mod pty_resize;
#[path = "app_parts/pty_retire.rs"]
mod pty_retire;
#[path = "app_parts/pty_write.rs"]
mod pty_write;
#[path = "app_parts/reduce_batch1.rs"]
mod reduce_batch1;
#[path = "app_parts/reduce_batch10.rs"]
mod reduce_batch10;
#[path = "app_parts/reduce_batch11.rs"]
mod reduce_batch11;
#[path = "app_parts/reduce_batch12.rs"]
mod reduce_batch12;
#[path = "app_parts/reduce_batch13.rs"]
mod reduce_batch13;
#[path = "app_parts/reduce_batch14.rs"]
mod reduce_batch14;
#[path = "app_parts/reduce_batch15.rs"]
mod reduce_batch15;
#[path = "app_parts/reduce_batch16.rs"]
mod reduce_batch16;
#[path = "app_parts/reduce_batch17.rs"]
mod reduce_batch17;
#[path = "app_parts/reduce_batch18.rs"]
mod reduce_batch18;
#[path = "app_parts/reduce_batch19.rs"]
mod reduce_batch19;
#[path = "app_parts/reduce_batch2.rs"]
mod reduce_batch2;
#[path = "app_parts/reduce_batch20.rs"]
mod reduce_batch20;
#[path = "app_parts/reduce_batch21.rs"]
mod reduce_batch21;
#[path = "app_parts/reduce_batch22.rs"]
mod reduce_batch22;
#[path = "app_parts/reduce_batch23.rs"]
mod reduce_batch23;
#[path = "app_parts/reduce_batch24.rs"]
mod reduce_batch24;
#[path = "app_parts/reduce_batch25.rs"]
mod reduce_batch25;
#[path = "app_parts/reduce_batch26.rs"]
mod reduce_batch26;
#[path = "app_parts/reduce_batch27.rs"]
mod reduce_batch27;
#[path = "app_parts/reduce_batch28.rs"]
mod reduce_batch28;
#[path = "app_parts/reduce_batch29.rs"]
mod reduce_batch29;
#[path = "app_parts/reduce_batch3.rs"]
mod reduce_batch3;
#[path = "app_parts/reduce_batch30.rs"]
mod reduce_batch30;
#[path = "app_parts/reduce_batch31.rs"]
mod reduce_batch31;
#[path = "app_parts/reduce_batch32.rs"]
mod reduce_batch32;
#[path = "app_parts/reduce_batch33.rs"]
mod reduce_batch33;
#[path = "app_parts/reduce_batch34.rs"]
mod reduce_batch34;
#[path = "app_parts/reduce_batch35.rs"]
mod reduce_batch35;
#[path = "app_parts/reduce_batch4.rs"]
mod reduce_batch4;
#[path = "app_parts/reduce_batch5.rs"]
mod reduce_batch5;
#[path = "app_parts/reduce_batch6.rs"]
mod reduce_batch6;
#[path = "app_parts/reduce_batch7.rs"]
mod reduce_batch7;
#[path = "app_parts/reduce_batch8.rs"]
mod reduce_batch8;
#[path = "app_parts/reduce_batch9.rs"]
mod reduce_batch9;
#[path = "app_parts/reflow_subscription.rs"]
mod reflow_subscription;
#[path = "app_parts/save_tasks.rs"]
mod save_tasks;
#[path = "app_parts/scroll_active.rs"]
mod scroll_active;
#[path = "app_parts/scroll_detached.rs"]
mod scroll_detached;
#[path = "app_parts/scroll_geometry.rs"]
mod scroll_geometry;
#[path = "app_parts/secret_dispatch.rs"]
mod secret_dispatch;
#[path = "app_parts/secret_finish.rs"]
mod secret_finish;
#[path = "app_parts/secret_load.rs"]
mod secret_load;
#[path = "app_parts/secret_ops.rs"]
mod secret_ops;
#[path = "app_parts/secret_tasks.rs"]
mod secret_tasks;
#[path = "app_parts/selection_methods.rs"]
mod selection_methods;
#[path = "app_parts/session_helpers.rs"]
mod session_helpers;
#[path = "app_parts/session_remove.rs"]
mod session_remove;
#[path = "app_parts/session_resize.rs"]
mod session_resize;
#[path = "app_parts/session_restore.rs"]
mod session_restore;
#[path = "app_parts/session_spawn.rs"]
mod session_spawn;
#[path = "app_parts/session_swap.rs"]
mod session_swap;
#[path = "app_parts/shell_composition.rs"]
mod shell_composition;
#[path = "app_parts/shell_events.rs"]
mod shell_events;
#[path = "app_parts/shell_geometry.rs"]
mod shell_geometry;
#[path = "app_parts/shell_query.rs"]
mod shell_query;
#[path = "app_parts/shell_view.rs"]
mod shell_view;
#[path = "app_parts/ssh_connect.rs"]
mod ssh_connect;
#[path = "app_parts/ssh_dialogs.rs"]
mod ssh_dialogs;
#[path = "app_parts/ssh_host_list.rs"]
mod ssh_host_list;
#[path = "app_parts/ssh_methods.rs"]
mod ssh_methods;
#[path = "app_parts/surface_geometry.rs"]
mod surface_geometry;
#[path = "app_parts/teardown_debts.rs"]
mod teardown_debts;
#[path = "app_parts/terminal_effects.rs"]
mod terminal_effects;
#[path = "app_parts/terminal_pane_view.rs"]
mod terminal_pane_view;
#[path = "app_parts/test_accessors.rs"]
mod test_accessors;
#[path = "app_parts/theme_shortcuts.rs"]
mod theme_shortcuts;
#[path = "app_parts/titlebar_find.rs"]
mod titlebar_find;
#[path = "app_parts/titlebar_inner.rs"]
mod titlebar_inner;
#[path = "app_parts/titlebar_view.rs"]
mod titlebar_view;
#[path = "app_parts/transcript_snapshot.rs"]
mod transcript_snapshot;
#[path = "app_parts/transcript_window.rs"]
mod transcript_window;
#[path = "app_parts/update_inner.rs"]
mod update_inner;
#[path = "app_parts/update_root.rs"]
mod update_root;
#[path = "app_parts/update_settlement.rs"]
mod update_settlement;
#[path = "app_parts/window_closed.rs"]
mod window_closed;
#[path = "app_parts/window_orphan.rs"]
mod window_orphan;
#[path = "app_parts/window_view.rs"]
mod window_view;

pub(crate) use message::Message;
#[path = "app_parts/credential_queue.rs"]
mod credential_queue;
#[path = "app_parts/credential_types.rs"]
mod credential_types;

#[cfg(test)]
pub(crate) use credential_types::SshSpawnOverride;
pub(crate) use credential_types::{
    CredentialFuture, CredentialMutationAttempt, CredentialMutationIntent, CredentialMutationPhase,
    InFlightCredentialOp, PendingCredentialMutation, PendingSecretDeletion, PendingSshConnection,
};
#[path = "app_parts/app_construct.rs"]
mod app_construct;
#[path = "app_parts/app_struct.rs"]
mod app_struct;

pub(crate) use app_struct::App;
#[path = "app_parts/ssh_form.rs"]
mod ssh_form;

pub(crate) use ssh_form::SshFormPresentation;

#[cfg(test)]
mod ssh_credential_persistence_tests;

#[cfg(all(test, target_os = "linux"))]
mod ssh_host_id_tests;

/// Grouping a running session into the content area's split
/// layout, and taking it back out.
mod pane_group;

/// Consequence gates for find, history, grouping and grid tokens.
#[cfg(test)]
mod review;

/// Production sidebar titlebar composition.
mod sidebar_header;
/// Unsplit terminal surface composition.
mod terminal_surface;
mod under_titlebar;

mod history_view;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use enzui_core::action_overflow::Descriptor;
use enzui_core::activation::{ActivationBroker, ActivationResponse};
use enzui_core::click::{ClickCount, ClickDetector, SingleClickPolicy};
use enzui_core::editable_shortcuts::{EditableShortcutCatalog, RebindRefusal, ShortcutValidity};
use enzui_core::form::{FieldValue, FormSubmission, SecretText};
use enzui_core::identity::derive_id;
use enzui_core::persist::{PersistError, Store};
use enzui_core::theme_file::integration::{ThemeEvent, ThemeIntegration};
use enzui_core::{
    Action, Icon,
    focus::{Traversal, traversal_operation as traverse},
};
use enzui_core::{Modifier, NamedKey, Shortcut, ShortcutBinding, ShortcutKey};
use enzui_services::credential_store::{
    CancelHandle, CredentialStore, Secret, UnavailableCredentialStore,
    mutation_owner::{
        CREDENTIAL_MUTATION_CAPACITY, CredentialMutationOwner, CredentialMutationSnapshot,
        CredentialMutationStore,
    },
};
use enzui_theme::EnzTheme;
use enzui_widgets::action_bar::ActionItem as TitlebarActionItem;
use enzui_widgets::app_shell::{ShellLayout, enz_app_shell};
use enzui_widgets::banner::{BannerTone, enz_banner};
use enzui_widgets::button::{ButtonIconPosition, ButtonKind, enz_button};
use enzui_widgets::cached_subtree::{SubtreeHandle, SubtreeId, enz_cached_subtree};
use enzui_widgets::confirm_dialog::{ConfirmDialogEvent, enz_confirm_dialog};
use enzui_widgets::context_menu_host::enz_optional_point_context_menu_host;
use enzui_widgets::editable_shortcuts::EditableShortcutsEvent;
use enzui_widgets::form_dialog::{FormDialogEvent, FormOutcome, FormPresenter, enz_form_dialog};
use enzui_widgets::list_row_surface::enz_list_row_surface;
use enzui_widgets::modal_host::{
    ModalHostEvent, ModalPresentation, ModalPresentationKey, enz_modal_host,
};
use enzui_widgets::modal_surface::enz_modal_surface;
use enzui_widgets::nav_split::NavSplitState;
use enzui_widgets::offset_scrollbar::enz_offset_scrollbar;
use enzui_widgets::scroll_view::enz_scroll_view;
use enzui_widgets::search_field::enz_titlebar_search;
use enzui_widgets::section_header::enz_section_header;
use enzui_widgets::sidebar_content::enz_sidebar_content;
use enzui_widgets::status_page::{StatusPageVariant, enz_status_page};
use enzui_widgets::text_field::enz_text_field;
use enzui_widgets::title_bar::enz_title_bar;
use enzui_widgets::title_bar_gesture::WindowCommand;
use enzui_widgets::toast::ToastTone;
use enzui_widgets::toast_host::{
    ToastEvent, ToastExpirySchedule, ToastHost, ToastSpec, WaitingBudget,
};
use enzui_widgets::toolbar_icon::enz_toolbar_icon;
use iced::keyboard::{self, Modifiers};
use ssh_credentials::{
    CredentialBackend, CredentialIntent, CredentialNotice, CredentialSlot, SshCredentialResult,
};
// Raw iced primitives that have NO framework replacement in this file:
//   * `container`/`column`/`row`/`Space` — layout primitives, not styled
//     widgets; files-app (the zero-literal reference) keeps these too.
//
// The titlebar reads the shell's own resolved `ShellGeometry::sidebar` through
// `.titlebar_with_geometry(..)` rather than wrapping itself in `responsive` to
// re-derive collapse from its own width, which would be a second collapse
// authority that disagrees where the band is under 640 px while the sidebar is
// still inline.
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Subscription, Task, advanced::widget::operate};

use capture_seed_data::{
    demo_transcript as capture_demo_transcript, enabled as capture_seed_enabled,
};
use grid_view::CellGeometry;
use model::{
    FindState, Pane, SSH_FORM_AUTH_MODE, Session, SessionKind, SessionStatus, Settings,
    SshAuthMode, SshAuthModeKind, SshHostForm, SshHostRecord, TranscriptLine, TranscriptRow,
    display_reported_cwd,
};
#[cfg(test)]
use model::{
    SSH_FORM_DEFAULT_PATH, SSH_FORM_DESTINATION, SSH_FORM_KEY_PATH, SSH_FORM_NAME, SSH_FORM_NOTES,
    SSH_FORM_PASSPHRASE, SSH_FORM_PASSWORD, SSH_FORM_PORT, SSH_FORM_USERNAME,
};
use pane_layout::PaneLayout;

fn main() -> iced::Result {
    i18n::localizer().scope(localized_main)
}

fn localized_main() -> iced::Result {
    let launch_directory = match launch_directory::parse(std::env::args_os().skip(1)) {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("terminal-emulator: {error}");
            std::process::exit(2);
        }
    };
    enzui_core::crash_report::CrashReporter::for_app(
        "terminal-emulator",
        env!("CARGO_PKG_VERSION"),
    );
    // Process-global environment is startup input, not application state. Read
    // the capture flag once, before iced starts worker threads, then pass the
    // resolved setting into every state construction.
    let capture_seed = capture_seed_enabled();
    #[cfg(feature = "native-invalidation-acceptance")]
    let capture_seed = capture_seed
        || std::env::var_os(enzui_core::app_runtime::native_invalidation::CONTROL_PATH_ENV)
            .is_some();
    // The three keyring states a review shot must show are backend
    // states, so the capture harness names one. Absent ⇒ nothing installed.
    let capture_keyring = capture_keyring::from_env_value(
        std::env::var_os("TERMINAL_EMULATOR_CAPTURE_KEYRING").as_deref(),
    );
    let capture_seed = capture_seed || capture_keyring.is_some();
    // A source-fresh toast-placement shot needs one live transient on
    // the first frame. `"1"` posts a copy acknowledgement through the real
    // `CopyHistoryCommand` reducer arm — the same route the History context
    // menu uses — so the captured overlay is the production path, not a
    // capture-only widget. Absent (or empty, as the nested compositor pre-
    // exports) ⇒ no toast, exactly like a normal boot.
    let capture_toast = capture_seed_data::enabled_from(
        std::env::var_os("TERMINAL_EMULATOR_CAPTURE_TOAST").as_deref(),
    );
    // `iced::daemon`, not `iced::application`.
    //
    // "Right click on sidebar -> Detach tab doesn't do anything" was true, and
    // a bool could not fix it: detaching a tab means a SECOND WINDOW, and
    // `iced::application` owns exactly one — its `view` is handed no
    // `window::Id`, so there is no signature in which a second window's
    // content could be returned (`app_shell.rs`, "A second window", item 1).
    //
    // The four traps that section lists are all answered here rather than
    // rediscovered:
    //
    //  1. `.window(..)` is GONE — `Daemon` has no such builder, so the boot
    //     task opens the main window itself and remembers its id.
    //  2. a daemon does not stop when its last window closes, so
    //     `Message::WindowClosed` runs the shared `WindowLedger` and issues a
    //     real `iced::exit()` (see `App::window_closed`).
    //  3. no `window::latest()` anywhere on a gesture path: every gesture
    //     carries the id of the window that raised it.
    //  4. `resize_events()` is app-wide, so it is filtered by id — a detached
    //     terminal being dragged wider must not drive the main window's
    //     responsive rungs.
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(
            move || {
                let (app, boot) = match launch_directory.as_ref() {
                    Some(directory) => App::new_in_directory(capture_seed, directory.clone()),
                    None => App::new(capture_seed),
                };
                // Capture fixtures never expose themselves to an agent.
                #[cfg(feature = "center")]
                let mut app = app;
                #[cfg(feature = "center")]
                if !capture_seed {
                    app.request_center();
                }
                #[cfg(feature = "native-invalidation-acceptance")]
                let mut app = app;
                #[cfg(feature = "native-invalidation-acceptance")]
                let native_boot = app.prepare_native_acceptance();
                #[cfg(not(feature = "native-invalidation-acceptance"))]
                let native_boot = Task::none();
                let keyring_fixture = match capture_keyring {
                    Some(fixture) => Task::done(Message::InstallCaptureKeyring(fixture)),
                    None => Task::none(),
                };
                // Decorations are fixed at creation; account for both runtime
                // layouts.
                let (id, open) = iced::window::open(main_window_settings());
                let mut app = app;
                app.shell_activity = shell_activity::ShellActivity::production();
                app.expect_main_window(id);
                let toast_fixture = if capture_toast {
                    Task::done(Message::CopyHistoryCommand(
                        "printf 'command copied'".into(),
                    ))
                } else {
                    Task::none()
                };
                (
                    app,
                    Task::batch([
                        boot,
                        keyring_fixture,
                        toast_fixture,
                        native_boot,
                        open.map(Message::MainWindowOpened),
                    ]),
                )
            },
            App::update,
            App::view_window,
        )
        .title(App::window_title)
        .subscription(App::subscription)
        .theme(App::window_theme),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(i18n::localizer().clone())
            .traversal(enzui_widgets::app_setup::TraversalBinding::Disabled),
    )
}

// `find_button_id` is GONE with the icon it named. Find is not
// an invoker in the band any more, it IS the field there
// (`App::titlebar_find`), and its focus target is `find_field_id()`.

macro_rules! screen_routes {
    ($($route:ident),+ $(,)?) => {
        /// Top-level routes. Terminal currently has one shell; `ALL` is
        /// generated by the same declaration so route-wide gates fail closed.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum Screen {
            $($route),+
        }

        impl Screen {
            #[cfg(test)]
            const ALL: &'static [Self] = &[$(Self::$route),+];
        }
    };
}

screen_routes!(Shell);

#[cfg(test)]
mod session_spawn_tests;
