use std::path::PathBuf;

use enzui_core::drop_files::CopiedFiles;
use iced::Task;

use enzui_widgets::file_drop::{DragAction, FileDrop};

use crate::{App, Message, operation_origin::OperationOrigin};

impl App {
    pub(crate) fn drop_files_into_pane(
        &mut self,
        origin: OperationOrigin,
        files: FileDrop,
    ) -> Task<Message> {
        if files
            .action()
            .is_some_and(|action| action != DragAction::Copy)
        {
            return Task::none();
        }
        if files.is_temporary() {
            if !self.accepts_pane_input(&origin, &Message::InputFocus(true)) {
                return Task::none();
            }
            let Some(storage) = attachment_storage() else {
                self.pane_feedback = Some("Cannot locate private attachment storage.".into());
                return Task::none();
            };
            let paths = files.paths().to_vec();
            let cancel = origin.generation.cancellation();
            let work = self.blocking.submit_fifo(move |context| {
                if context.is_cancelled() {
                    return Err("Attachment import was cancelled.".into());
                }
                enzui_core::drop_files::copy_files(
                    &paths,
                    Some(&storage),
                    128 * 1024 * 1024,
                    &cancel,
                )
                .map_err(|_| {
                    "Cannot retain this attachment; use regular files totaling at most 128 MiB."
                        .into()
                })
            });
            return Task::perform(
                async move {
                    work.await
                        .into_result()
                        .unwrap_or_else(|_| Err("Attachment worker is unavailable.".into()))
                },
                move |result| Message::FilesCopied(origin, files, result),
            );
        }
        if self.paste_dropped_paths(&origin, files.paths()) {
            files.finish();
        }
        Task::none()
    }

    pub(crate) fn finish_file_copy(
        &mut self,
        origin: OperationOrigin,
        files: FileDrop,
        result: Result<CopiedFiles, String>,
    ) {
        match result {
            Ok(copies) if self.paste_retained_paths(&origin, &copies) => {
                files.finish();
            }
            Ok(_) => {}
            Err(error) if origin.generation.is_live() => self.pane_feedback = Some(error),
            Err(_) => {}
        }
    }

    pub(crate) fn paste_retained_paths(
        &mut self,
        origin: &OperationOrigin,
        copies: &CopiedFiles,
    ) -> bool {
        if !self.paste_dropped_paths(origin, copies.paths()) {
            return false;
        }
        // The user may execute the pasted command after this app has closed.
        copies.preserve();
        true
    }

    pub(crate) fn drop_text_into_pane(
        &mut self,
        origin: OperationOrigin,
        data: enzui_widgets::file_drop::DataDrop,
    ) {
        if data
            .action()
            .is_some_and(|action| action != DragAction::Copy)
        {
            return;
        }
        if let Some(text) = data.data().text()
            && self.paste_native_text(&origin, text)
        {
            data.finish();
        }
    }

    pub(crate) fn paste_dropped_paths(
        &mut self,
        origin: &OperationOrigin,
        paths: &[PathBuf],
    ) -> bool {
        match quoted_paths(paths) {
            Ok(text) => self.paste_native_text(origin, &text),
            Err(reason) => {
                self.pane_feedback = Some(reason.to_owned());
                false
            }
        }
    }

    fn paste_native_text(&mut self, origin: &OperationOrigin, text: &str) -> bool {
        if !self.accepts_pane_input(origin, &Message::InputFocus(true))
            || origin.window == self.main_window && (self.settings_open || self.ssh_manager_open)
        {
            return false;
        }
        let Some(pane) = self
            .sessions
            .iter()
            .find(|session| session.id == origin.session)
            .and_then(|session| session.panes.iter().find(|pane| pane.id == origin.pane))
        else {
            return false;
        };
        let bracketed = pane.bracketed_paste;
        let Ok(text) = crate::escape_policy::normalized_paste(text) else {
            return false;
        };
        let bytes = crate::paste_bytes(&text, bracketed);
        if !self.try_write_pane_pty(&origin.session, &origin.pane, &bytes) {
            return false;
        }
        self.exclude_pinned_input(&origin.pane);
        true
    }
}

fn quoted_paths(paths: &[PathBuf]) -> Result<String, &'static str> {
    if paths.is_empty() || paths.len() > 128 {
        return Err("Drop between 1 and 128 paths.");
    }
    let mut text = String::new();
    for path in paths {
        let path = path
            .to_str()
            .filter(|path| path.starts_with('/') && !path.chars().any(char::is_control))
            .ok_or("The dropped path cannot be safely inserted into the terminal.")?;
        text.push_str(&crate::pty::shell_single_quote(path));
        text.push(' ');
    }
    Ok(text)
}

fn attachment_storage() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".local/share"))
                .filter(|path| path.is_absolute())
        })
        .map(|root| root.join("terminal-emulator/Imported attachments"))
}
