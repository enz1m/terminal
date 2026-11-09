//! Terminal inner reducer shell.

use super::*;

impl App {
    #[allow(
        clippy::too_many_lines,
        reason = "The reducer dispatches each message once while preserving the shared post-update bookkeeping."
    )]
    pub(crate) fn update_inner(&mut self, message: Message) -> Task<Message> {
        if matches!(&message, Message::WindowClosed(id) if self.main_window == Some(*id))
            && let Some(startup) = self.startup.as_mut()
        {
            startup.invalidate();
        }
        if self.startup.is_some() && !persistence_runtime::Startup::accepts(&message) {
            return Task::none();
        }
        if !self.accepts_operation_completion(&message) {
            return Task::none();
        }
        if (self.pending_ssh_host_save.is_some() || self.ssh_host_save_uncertain)
            && matches!(
                &message,
                Message::OpenSshAddForm
                    | Message::OpenSshEditForm(_)
                    | Message::DeleteSshHost(_)
                    | Message::ConfirmDeleteSshHost(_)
                    | Message::ConfirmDeleteSshHostEvent(_)
                    | Message::ConnectSshHost(_)
                    | Message::InstallCaptureKeyring(_)
            )
        {
            self.ssh_save_error = Some(if self.ssh_host_save_uncertain {
                ssh_host_save::UNCERTAIN
            } else {
                "A connection is being saved. Wait for publication to finish before changing saved connections."
            }.into());
            return Task::none();
        }
        if matches!(
            &message,
            Message::NewLocal
                | Message::SelectSession(_)
                | Message::SelectDockedSession(_)
                | Message::SelectNextSession
                | Message::SelectPrevSession
                | Message::FocusPane(_)
                | Message::FocusPaneDir(_)
                | Message::ClosePane
                | Message::NewPane
                | Message::NewPaneDown
                | Message::DetachedSplitPane { .. }
                | Message::CloseActive
                | Message::ReconnectRestoredPane
                | Message::ResetSavedSessions
                | Message::RedockSession(_)
                | Message::PaneGridEvent(_)
                | Message::WindowClosed(_)
                | Message::DeleteSshHost(_)
                | Message::ConnectSshHost(_)
                | Message::OpenSshAddForm
                | Message::OpenSshEditForm(_)
                | Message::CloseSshManager
                | Message::CancelSshCredentialOps
        ) {
            self.cancel_ssh_host_save();
            self.retire_openssh_choice();
        }
        match message {
            Message::ActivityWindowTagged(..) | Message::ActivityPrune(..) => return Task::none(),
            Message::WindowChromeKey(window, _, key, mods, captured) => {
                return self.reduce_window_chrome(window, key, mods, captured);
            }
            Message::PaneInput(origin, message) => return self.reduce_pane_input(origin, *message),
            Message::PanePointer(origin, _, start, message) => {
                return self.reduce1_pane_pointer_0(origin, start, message);
            }
            Message::InputIme(..)
            | Message::InputKey(..)
            | Message::InputFocus(..)
            | Message::InputFocusEdge(..)
            | Message::StampedInput(..)
            | Message::InputMenuOpen => return self.reduce1_input_sink_1(),
            Message::InputMenuEvent(origin, event) => return self.reduce_input_menu(origin, event),
            Message::InputCursor(window, point) => {
                return self.reduce1_input_cursor_2(window, point);
            }
            Message::InputGroupInto(target) => {
                return self.reduce1_input_group_into_3(target);
            }
            Message::OwnedUriActivation(origin, uri, response) => {
                return self.reduce1_owned_uri_activation_4(origin, uri, response);
            }
            Message::OwnedLinkFinished(origin, result) => {
                return self.reduce1_owned_link_finished_5(origin, result);
            }
            Message::Theme(event) => {
                return self.reduce1_theme_6(event);
            }
            Message::ModifiersChanged(m) => {
                return self.reduce1_modifiers_changed_8(m);
            }
            #[cfg(test)]
            Message::WindowKeyPressed {
                window,
                key,
                mods,
                captured,
            } => {
                return self.reduce2_window_key_pressed_0(window, key, mods, captured);
            }
            Message::DetachedKeyPressed {
                window,
                pane,
                key,
                mods,
                captured,
            } => {
                return self.reduce2_detached_key_pressed_1(window, pane, key, mods, captured);
            }
            Message::KeyPressed {
                key,
                mods,
                captured,
            } => {
                return self.reduce3_key_pressed_0(key, mods, captured);
            }
            Message::ContentResized(size) => {
                return self.reduce4_content_resized_0(size);
            }
            Message::Poll => {
                return self.reduce4_poll_1();
            }
            #[cfg(feature = "native-invalidation-acceptance")]
            Message::NativeAcceptancePoll => unreachable!("handled before the production reducer"),
            // ---- Scrollback
            // ------------------------------------------------------
            Message::TranscriptWheel(rows) => {
                return self.reduce4_transcript_wheel_2(rows);
            }
            Message::ScrollTranscriptRows(delta) => {
                return self.reduce4_scroll_transcript_rows_3(delta);
            }
            Message::ScrollTranscriptToRow(row) => {
                return self.reduce4_scroll_transcript_to_row_4(row);
            }
            // ---- The grid's pointer reports ------------------------------
            Message::GridCellPressed(target) => return self.reduce5_grid_cell_pressed_0(target),
            Message::GridCellDragged(target) => return self.reduce5_grid_cell_dragged_1(target),
            Message::GridCellReleased(target) => return self.reduce5_grid_cell_released_2(target),
            Message::TerminalFit { pane, cols, rows } => {
                self.terminal_fit_reported(pane, cols, rows);
            }
            Message::TerminalViewIgnored => {}
            #[cfg(test)]
            Message::LinkOpenFinished(result) => self.finish_link_open(result),
            Message::UriPortalWake(wake) => return self.reduce6_uri_portal_wake_0(wake),
            Message::JumpToLatest => return self.reduce6_jump_to_latest_1(),
            Message::DetachedTranscriptWheel { window, pane, rows } => {
                return self.reduce7_detached_transcript_wheel_0(window, pane, rows);
            }
            Message::DetachedScrollTranscriptRows {
                window,
                pane,
                delta,
            } => {
                return self.reduce7_detached_scroll_transcript_rows_1(window, pane, delta);
            }
            Message::DetachedScrollTranscriptToRow { window, pane, row } => {
                return self.reduce7_detached_scroll_transcript_to_row_2(window, pane, row);
            }
            Message::DetachedJumpToLatest { window, pane } => {
                return self.reduce7_detached_jump_to_latest_3(window, pane);
            }
            Message::DetachedToggleHistory(window) => {
                return self.reduce8_detached_toggle_history_0(window);
            }
            Message::DetachedCloseHistory(window) => {
                return self.reduce8_detached_close_history_1(window);
            }
            Message::DetachedPasteHistoryCommand { window, command } => {
                return self.reduce8_detached_paste_history_command_2(window, command);
            }
            Message::SelectSession(i) => return self.reduce9_select_session_0(i),
            Message::SelectNextSession => return self.reduce9_select_next_session_1(),
            Message::SelectDockedSession(n) => {
                let index = (0..self.sessions.len())
                    .filter(|&index| !self.in_own_window(index))
                    .nth(n);
                if let Some(index) = index {
                    return self.update(Message::SelectSession(index));
                }
            }
            Message::SelectPrevSession => return self.reduce9_select_prev_session_2(),
            Message::NewLocal => return self.reduce9_new_local_3(),
            Message::PaneSpawned(ticket, pane, outcome) => {
                if let Some(outcome) = outcome.take() {
                    self.finish_pane_spawn(ticket, &pane, outcome);
                }
            }
            Message::RestoredGridResized(_) => {}
            Message::ReconnectRestoredPane => return self.reduce10_reconnect_restored_pane_0(),
            Message::ResetSavedSessions => return self.reduce10_reset_saved_sessions_1(),
            Message::CloseActive => return self.reduce10_close_active_2(),
            Message::NewPane => return self.reduce11_new_pane_0(),
            Message::NewPaneDown => return self.reduce11_new_pane_down_1(),
            Message::DetachedSplitPane {
                window,
                pane,
                direction,
            } => {
                return self.reduce11_detached_split_pane_2(window, pane, direction);
            }
            Message::ClosePane => return self.reduce11_close_pane_3(),
            Message::FocusPane(id) => return self.reduce12_focus_pane_0(id),
            Message::PaneGridEvent(event) => return self.reduce12_pane_grid_event_1(event),
            Message::BeginPaneRename => return self.reduce12_begin_pane_rename_2(),
            Message::PaneRenameEdited(v) => return self.reduce12_pane_rename_edited_3(v),
            Message::CommitPaneRename => return self.reduce13_commit_pane_rename_0(),
            Message::CancelPaneRename => return self.reduce13_cancel_pane_rename_1(),
            #[cfg(test)]
            Message::ToggleDetach => return self.reduce13_toggle_detach_2(),

            // ---- Session-row context menu ---------------------------
            Message::OpenContextMenu(id, at) => {
                return self.reduce14_open_context_menu_0(id, at);
            }
            // The ONE funnel every framework host event arrives through.
            //
            // The host is controlled: it proposes, this applies. `apply_event`
            // is the framework's single application — it opens, it closes
            // atomically, it moves the roving highlight through the
            // `MenuNavigation` that SKIPS DISABLED ROWS (this menu has three
            // genuinely disabled rows), and it re-resolves an activation
            // against the live surface.
            Message::ContextMenuEvent(event) => {
                return self.reduce14_context_menu_event_1(event);
            }
            Message::ContextRowActivated(id, row) => {
                return self.reduce14_context_row_activated_2(id, row);
            }
            Message::BeginSessionRename => {
                return self.reduce15_begin_session_rename_0();
            }
            Message::SessionRenameEdited(text) => {
                return self.reduce15_session_rename_edited_1(text);
            }
            Message::CommitSessionRename => {
                return self.reduce15_commit_session_rename_2();
            }
            Message::CancelSessionRename => {
                return self.reduce15_cancel_session_rename_3();
            }

            // All three of these move the sidebar, and the sidebar's
            // presentation decides how much width the content pane gets
            // (`content_pane_width`). Collapsing it hands the grid ~260 px more
            // real estate, so the shell must be told the new column count or it
            // keeps wrapping to the old, narrower one until some unrelated
            // event (a window resize, a font change) happens to fire a resize.
            Message::DismissSidebar => return self.reduce16_dismiss_sidebar_0(),
            Message::SidebarState(next) => return self.reduce16_sidebar_state_1(next),
            Message::ToggleSidebar => return self.reduce16_toggle_sidebar_2(),
            Message::FocusPaneDir(dir) => return self.reduce16_focus_pane_dir_3(dir),
            Message::ToggleTitlebarOverflow => {
                return self.reduce17_toggle_titlebar_overflow_0();
            }
            Message::CloseTitlebarOverflow => {
                return self.reduce17_close_titlebar_overflow_1();
            }
            Message::OpenSettings => {
                return self.reduce17_open_settings_3();
            }
            Message::CloseSettings => {
                return self.reduce17_close_settings_4();
            }
            Message::SettingsExitSettled(key) => {
                return self.reduce17_settings_exit_settled_5(key);
            }
            Message::DetachedTitlebarOverflow(window, open) => {
                if let Some(detached) = self.detached_windows.get_mut(&window)
                    && !detached.closing
                {
                    detached.titlebar_overflow_open = open;
                }
                return Task::none();
            }
            Message::SidebarTitlebarOverflow(open) => {
                self.sidebar_titlebar_overflow_open = open;
                return Task::none();
            }
            Message::Window(command) => return self.reduce18_window_0(command),
            Message::WindowFor(id, command) => return self.reduce18_window_for_1(id, command),
            Message::WindowState(event) => {
                return self
                    .window_chrome
                    .get_mut(&event.window())
                    .map_or_else(Task::none, |chrome| {
                        chrome.update(event).map(Message::WindowState)
                    });
            }
            Message::MainWindowOpened(id) => {
                let task = self.reduce19_main_window_opened_0(id);
                let chrome = if self.main_window == Some(id) {
                    self.observe_window_chrome(id)
                } else {
                    Task::none()
                };
                return Task::batch([task, chrome]);
            }
            Message::DetachedWindowOpened { session, window } => {
                let task = self.reduce19_detached_window_opened_1(session, window);
                let chrome = if self.detached_windows.contains_key(&window) {
                    self.observe_window_chrome(window)
                } else {
                    Task::none()
                };
                return Task::batch([task, chrome]);
            }
            Message::WindowResized(id, size) => {
                return self.reduce19_window_resized_2(id, size);
            }
            Message::WindowClosed(id) => {
                self.window_chrome.remove(&id);
                return self.reduce19_window_closed_3(id);
            }
            Message::WindowFocusChanged(id, focused) => {
                return self.reduce19_window_focus_changed_4(id, focused);
            }
            Message::RedockSession(session) => {
                return self.reduce35_redock_session_2(session);
            }
            #[cfg(feature = "center")]
            Message::Center(event) => return self.center_event(event),
            Message::SettingsModalEvent(event) => {
                return self.reduce35_settings_modal_event_3(event);
            }
            Message::FocusNext => return self.reduce20_focus_next_0(),
            Message::FocusPrevious => return self.reduce20_focus_previous_1(),
            Message::SetColorScheme(id) => return self.reduce20_set_color_scheme_2(id),
            Message::SetFontSizePx(px) => return self.reduce20_set_font_size_px_3(px),
            Message::SetCursorStyle(style) => return self.reduce20_set_cursor_style_4(style),
            Message::SetScrollback(n) => return self.reduce20_set_scrollback_5(n),

            // ---- Appearance and shortcut settings -------------------------
            Message::ResetAppearance => return self.reduce21_reset_appearance_0(),
            Message::SelectSettingsPage(page) => return self.reduce21_select_settings_page_1(page),
            Message::SettingsBack => self.settings_drilled_in = false,
            Message::EditShortcut(event) => return self.reduce22_edit_shortcut_0(event),
            Message::StartupPreferences(value) => return self.startup_preferences(value),
            Message::StartupSessions(value) => return self.startup_sessions(value),
            Message::SshHostSaveFinished(generation, outcome) => {
                return self.reduce23_ssh_host_save_finished_0(generation, outcome);
            }
            Message::PreferencesWorkerFinished(generation, result) => {
                return self.reduce23_preferences_worker_finished_1(generation, result);
            }
            Message::PreferencesSaved(result) => {
                return self.reduce23_preferences_saved_2(result);
            }
            Message::CloseRequested(window) => return self.request_window_close(window),
            Message::CloseSettle => return self.reduce24_close_settle_0(),
            Message::RetryClose => return self.retry_close(),
            Message::RetryTeardown => return self.retry_teardown(),
            Message::TeardownPoll => return self.reduce24_teardown_poll_1(),
            Message::CloseWithoutSaving => return self.close_without_saving(),
            Message::PinnedStorageFinished(result) => self.finish_pinned_storage(*result),
            Message::RetryPinnedStorage => self.secure_pins.retry(),
            Message::DismissNotice(id) => self.dismiss_notice(id),
            Message::GitBranch(pane, branch) => self.set_git_branch(pane, branch),
            #[cfg(test)]
            Message::CancelPinnedStorage => self.secure_pins.cancel(),
            Message::ClearPinnedOutput => return self.reduce25_clear_pinned_output_0(),

            // ---- Find bar (cluster B) ------------------------------------
            Message::OpenFind => return self.reduce25_open_find_1(),
            Message::CloseFind => return self.reduce25_close_find_2(),
            Message::FindQueryChanged(query) => return self.reduce25_find_query_changed_3(query),
            Message::FindNext => return self.reduce25_find_next_4(),
            Message::FindPrev => return self.reduce25_find_prev_5(),

            // ---- Clipboard paste ------------------------------------------
            Message::RequestCopy => {
                return self.reduce26_request_copy_0();
            }
            Message::RequestPaste => return self.request_paste(),
            Message::DataDropped(origin, data) => self.drop_text_into_pane(origin, data),
            Message::FilesDropped(origin, files) => {
                return self.drop_files_into_pane(origin, files);
            }
            Message::FilesCopied(origin, files, result) => {
                self.finish_file_copy(origin, files, result);
            }
            Message::ClipboardPasteReady(origin, text) => self.finish_paste(origin, text),
            Message::Toast(event) => {
                return self.reduce26_toast_1(event);
            }

            // ---- History overlay (cluster C) ------------------------------
            Message::OpenHistory => {
                return self.reduce26_open_history_2();
            }
            Message::ToggleHistory => {
                return self.reduce26_toggle_history_3();
            }
            Message::CloseHistory => {
                return self.reduce26_close_history_4();
            }
            Message::CopyHistoryCommand(command) => {
                return self.reduce26_copy_history_command_5(command);
            }
            Message::UngroupSession(group_id, member) => {
                return self.reduce27_ungroup_session_0(group_id, member);
            }
            Message::SessionDropped(request) => {
                return self.reduce27_session_dropped_1(request);
            }
            Message::PasteHistoryCommand(command) => {
                return self.reduce27_paste_history_command_2(command);
            }

            // ---- SSH connection manager -----------------------------------
            Message::OpenSshManager => {
                return self.reduce27_open_ssh_manager_3();
            }
            Message::CloseSshManager => {
                return self.reduce27_close_ssh_manager_4();
            }
            Message::CancelSshCredentialOps => {
                return self.reduce27_cancel_ssh_credential_ops_5();
            }
            Message::CredentialBackendReady(backend) => {
                return self.reduce28_credential_backend_ready_0(backend);
            }
            Message::InstallCaptureKeyring(fixture) => {
                return self.reduce28_install_capture_keyring_1(fixture);
            }
            Message::SshManagerModalEvent(event) => return self.reduce_ssh_manager_host(event),
            Message::OpenSshAddForm => {
                return self.reduce28_open_ssh_add_form_2();
            }
            Message::OpenSshEditForm(idx) => {
                return self.reduce28_open_ssh_edit_form_3(idx);
            }
            Message::SshFormEvent(event) => return self.update_ssh_form(event),
            #[cfg(test)]
            Message::CloseSshForm => {
                return self.reduce28_close_ssh_form_4();
            }
            #[cfg(test)]
            Message::SaveSshConnection => {
                return self.reduce29_save_ssh_connection_0();
            }
            Message::DeleteSshHost(idx) => {
                return self.reduce29_delete_ssh_host_1(idx);
            }
            Message::ConfirmDeleteSshHost(accepted) => {
                return self.reduce29_confirm_delete_ssh_host_2(accepted);
            }
            Message::ConfirmDeleteSshHostEvent(event) => {
                return self.reduce29_confirm_delete_ssh_host_event_3(event);
            }
            Message::ConfirmHostKey(origin) => {
                return self.reduce35_confirm_host_key_4(origin);
            }
            Message::ConnectSshHost(idx) => {
                return self.reduce30_connect_ssh_host_0(idx);
            }
            Message::ConnectOpenSshHost(idx, generation) => {
                return self.reduce30_connect_openssh_host_1(idx, generation);
            }
            Message::ReloadOpenSshConfig => {
                return self.reduce30_reload_openssh_config_2();
            }
            Message::OpenSshFirstFrame(window) => return self.openssh_first_frame(window),
            Message::OpenSshDiscoveryFinished(completion) => {
                return self.reduce31_openssh_discovery_finished_0(completion);
            }
            #[cfg(test)]
            Message::OpenSshConfigLoaded(result) => {
                return self.reduce31_openssh_config_loaded_1(result);
            }
            Message::SshCredentialStored(origin, owner, slot, result) => {
                return self.reduce31_ssh_credential_stored_2(origin, owner, slot, result);
            }
            Message::SshCredentialLoaded(origin, owner, slot, token, result) => {
                return self.reduce32_ssh_credential_loaded_0(origin, owner, slot, token, result);
            }
            Message::SshCredentialDeleted(origin, owner, slot, result) => {
                return self.reduce32_ssh_credential_deleted_1(origin, owner, slot, result);
            }
            Message::SshCredentialReconciled(origin, owner, slot, retry_mismatch, result) => {
                return self.reduce32_ssh_credential_reconciled_2(
                    origin,
                    owner,
                    slot,
                    retry_mismatch,
                    result,
                );
            }
            Message::CredentialMutationWake => return self.retry_credential_mutations(false),
            Message::RetryCredentialMutations => return self.retry_credential_mutations(true),
            // Test compatibility commands all enter the shared presenter's
            // event path. Production controls emit `SshFormEvent` directly.
            #[cfg(test)]
            Message::SshFormNameEdited(v) => {
                return self.reduce33_ssh_form_name_edited_0(v);
            }
            #[cfg(test)]
            Message::SshFormDestEdited(v) => {
                return self.reduce33_ssh_form_dest_edited_1(v);
            }
            #[cfg(test)]
            Message::SshFormPortEdited(v) => {
                return self.reduce33_ssh_form_port_edited_2(v);
            }
            #[cfg(test)]
            Message::SshFormUsernameEdited(v) => {
                return self.reduce33_ssh_form_username_edited_3(v);
            }
            #[cfg(test)]
            Message::SshFormAuthModeSelected(idx) => {
                return self.reduce33_ssh_form_auth_mode_selected_4(idx);
            }
            #[cfg(test)]
            Message::SshFormPasswordEdited(v) => {
                return self.reduce34_ssh_form_password_edited_0(v);
            }
            #[cfg(test)]
            Message::SshFormTogglePasswordRevealed => {
                return self.reduce34_ssh_form_toggle_password_revealed_1();
            }
            #[cfg(test)]
            Message::SshFormKeyPathEdited(v) => {
                return self.reduce34_ssh_form_key_path_edited_2(v);
            }
            #[cfg(test)]
            Message::SshFormPassphraseEdited(v) => {
                return self.reduce34_ssh_form_passphrase_edited_3(v);
            }
            #[cfg(test)]
            Message::SshFormTogglePassphraseRevealed => {
                return self.reduce34_ssh_form_toggle_passphrase_revealed_4();
            }
            #[cfg(test)]
            Message::SshFormDefaultPathEdited(v) => {
                return self.reduce35_ssh_form_default_path_edited_0(v);
            }
            #[cfg(test)]
            Message::SshFormNotesEdited(v) => {
                return self.reduce35_ssh_form_notes_edited_1(v);
            }
        }
        Task::none()
    }
}
