//! Terminal titlebar methods.

use super::*;

impl App {
    /// Use the shell's resolved geometry to expose sidebar destinations only
    /// while their sidebar controls are off screen.
    pub(crate) fn titlebar(
        &self,
        geometry: &enzui_widgets::app_shell::ShellGeometry,
    ) -> Element<'_, Message> {
        self.titlebar_inner(!geometry.sidebar.is_inline())
    }

    /// Session commands share one inventory across inline and overflow presentations.
    pub(crate) fn titlebar_commands(&self) -> Vec<(&'static str, &'static str, String, Message)> {
        // (descriptor id, lucide icon, accessible name, message)
        let mut commands = Vec::new();
        if let Some(session) = self.sessions.get(self.active) {
            self.pane_commands(session, &mut commands);
        }
        // A session closes from its sidebar row (or Ctrl+Shift+W), and
        // Settings lives in the sidebar's band.
        commands.push((
            "session.history",
            "history",
            crate::i18n::label("History").to_owned(),
            Message::OpenHistory,
        ));
        commands
    }

    /// The pane commands for `session`: splitting from its first pane on;
    /// rename, close and ungroup once it is split.
    fn pane_commands(
        &self,
        session: &model::Session,
        commands: &mut Vec<(&'static str, &'static str, String, Message)>,
    ) {
        let split = session.panes.len() > 1;
        if split {
            commands.push((
                "pane.rename",
                "pencil",
                crate::i18n::label("Rename selected pane").to_owned(),
                Message::BeginPaneRename,
            ));
        }
        // Splitting is offered from the first pane on, not only once split.
        commands.extend([
            (
                "pane.split-horizontal",
                "columns-2",
                self.shortcut_label("pane.split-horizontal"),
                Message::NewPane,
            ),
            (
                "pane.split-vertical",
                "rows-2",
                self.shortcut_label("pane.split-vertical"),
                Message::NewPaneDown,
            ),
        ]);
        if split {
            commands.push((
                "pane.close",
                "x",
                crate::i18n::label("Close selected pane").to_owned(),
                Message::ClosePane,
            ));
            if let Some(member_index) = session.group_members.iter().position(|member| {
                session
                    .panes
                    .get(session.active_pane)
                    .is_some_and(|pane| member.pane_ids.iter().any(|id| id == &pane.id))
            }) {
                commands.push((
                    "pane.ungroup",
                    "ungroup",
                    crate::i18n::label("Ungroup selected pane").to_owned(),
                    Message::UngroupSession(session.id.clone(), member_index),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(app: &App) -> Vec<&'static str> {
        app.titlebar_commands()
            .into_iter()
            .map(|(id, ..)| id)
            .collect()
    }
}
