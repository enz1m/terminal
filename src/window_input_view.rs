use crate::{App, Message, grid_view, model};
use iced::{Element, Length, Rectangle};

pub(crate) fn find_id(window: iced::window::Id) -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::from(format!("terminal-find-{window}"))
}

impl App {
    pub(crate) fn input_surface<'a>(
        &self,
        session: &model::Session,
        pane: &'a model::Pane,
        window_start: usize,
        child: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let child = self.measured_restored_pane(pane, child);
        let child = self.host_trust_notice(pane, child);
        let Some(origin) = self
            .input_origin(&session.id, &pane.id)
            .filter(|origin| origin.window.is_some())
        else {
            return child;
        };
        let Some(epoch) = self.transcript_epochs.get(&pane.id).cloned() else {
            return child;
        };
        let window = origin.window.unwrap();
        let state = self.window_inputs.get(&window);
        let enabled = self.input_is_active(&origin)
            && self.ptys.contains_key(&origin.pane)
            && self.focused_window == Some(window)
            && state.is_some_and(|state| state.terminal_focus && !state.blurred)
            && !(self.main_window == Some(window) && (self.settings_open || self.ssh_manager_open));
        let preedit = state
            .and_then(|state| state.ime.as_ref())
            .filter(|(owner, _)| owner == &origin)
            .map(|(_, preedit)| preedit.clone());
        let geom = self.geometry();
        let (start, end) = self
            .scoped_grid_window(&origin)
            .unwrap_or((0, pane.transcript.len()));
        let (row, col) = pane
            .transcript
            .get(start..end)
            .and_then(|lines| lines.iter().enumerate().find(|(_, row)| row.is_prompt))
            .map_or((0, 0), |(row, line)| (row, line.cursor_col.max(0)));
        let inset = self.pane_inset(pane.alt_screen);
        let caret = Rectangle {
            x: inset + col as f32 * geom.cell_width,
            y: inset + row as f32 * geom.cell_height,
            width: geom.cell_width,
            height: geom.cell_height,
        };
        let drop_key = ("terminal-files", origin.clone(), epoch.clone());
        let copy_epoch = epoch.clone();
        let surface = origin.clone();
        let child = child.map(move |message| {
            if matches!(
                message,
                Message::PaneInput(..) | Message::PanePointer(..) | Message::InputMenuEvent(..)
            ) {
                return message;
            }
            if matches!(
                message,
                Message::GridCellPressed(_)
                    | Message::GridCellDragged(_)
                    | Message::GridCellReleased(_)
            ) {
                Message::PanePointer(
                    surface.clone(),
                    epoch.clone(),
                    window_start,
                    Box::new(message),
                )
            } else {
                Message::PaneInput(surface.clone(), Box::new(message))
            }
        });
        let drop_origin = origin.clone();
        let selection = self.selection_for(&pane.id);
        let copy_lifetime = origin.generation.cancellation();
        let child = crate::input_surface::surface(
            child,
            origin,
            enabled,
            preedit,
            caret,
            crate::input_surface::SelectionSource {
                epoch: copy_epoch,
                range: selection,
                resolve: Box::new(move |point, bounds| {
                    let selection = selection?;
                    let x = point.x - bounds.x - inset;
                    let y = point.y - bounds.y - inset;
                    if x < 0.0 || y < 0.0 || copy_lifetime.is_terminal() {
                        return None;
                    }
                    let row = window_start.saturating_add((y / geom.cell_height).floor() as usize);
                    let col = (x / geom.cell_width).floor() as usize;
                    selection.contains(row as i64, col).then(|| {
                        (
                            selection.selected_text(&pane.transcript),
                            copy_lifetime.share(),
                        )
                    })
                }),
            },
        );
        if self.ptys.contains_key(&drop_origin.pane)
            && !(drop_origin.window == self.main_window
                && (self.settings_open || self.ssh_manager_open))
        {
            let text_origin = drop_origin.clone();
            let child = enzui_widgets::file_drop::enz_data_drop_target(
                child,
                enzui_clipboard::Kind::Text.preferences(),
                move |data| Message::DataDropped(text_origin.clone(), data),
            )
            .key(drop_key.clone());
            enzui_widgets::file_drop::enz_file_drop_target_with_receipt(child, move |files| {
                Message::FilesDropped(drop_origin.clone(), files)
            })
            .copy_only(true)
            .key(drop_key)
            .into()
        } else {
            child
        }
    }

    pub(crate) fn input_menu_view<'a>(
        &'a self,
        window: Option<iced::window::Id>,
        child: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let menu = window
            .and_then(|window| self.window_inputs.get(&window))
            .and_then(|state| state.menu.as_ref())
            .filter(|menu| self.input_menu_live(&menu.origin));
        let origin = menu.map(|menu| menu.origin.clone());
        enzui_widgets::context_menu_host::enz_optional_point_context_menu_host(
            child,
            menu.map(|menu| (&menu.model, &menu.state)),
            move |event| {
                origin.as_ref().map_or(Message::InputMenuOpen, |origin| {
                    Message::InputMenuEvent(origin.clone(), event)
                })
            },
        )
        .into()
    }

    pub(crate) fn input_find_height(&self, window: iced::window::Id) -> f32 {
        let open = if self
            .input_context
            .as_ref()
            .is_some_and(|origin| origin.window == Some(window))
        {
            self.find.open
        } else {
            self.window_inputs
                .get(&window)
                .is_some_and(|state| state.find.open)
        };
        if open {
            self.titlebar_height() + self.enz_theme().spacing.sm()
        } else {
            0.0
        }
    }

    pub(crate) fn detached_find_bar(
        &self,
        window: iced::window::Id,
        session: &model::Session,
    ) -> Element<'_, Message> {
        use iced::widget::{Space, button, row, text_input};
        let Some(state) = self
            .window_inputs
            .get(&window)
            .filter(|state| state.find.open)
        else {
            return Space::new().height(0.0).into();
        };
        let Some(origin) = state
            .find_owner
            .as_deref()
            .and_then(|pane| self.input_origin(&session.id, pane))
        else {
            return Space::new().height(0.0).into();
        };
        let field = text_input(crate::i18n::label("Find in this pane"), &state.find.query)
            .id(find_id(window))
            .on_input(Message::FindQueryChanged)
            .on_submit(Message::FindNext);
        let bar = row![
            field,
            button(crate::i18n::label("Previous")).on_press(Message::FindPrev),
            button(crate::i18n::label("Next")).on_press(Message::FindNext),
            button(crate::i18n::label("Close find")).on_press(Message::CloseFind)
        ]
        .spacing(self.enz_theme().spacing.xs())
        .height(Length::Fixed(self.input_find_height(window)));
        let bar: Element<'_, Message> = bar.into();
        bar.map(move |message| Message::PaneInput(origin.clone(), Box::new(message)))
    }

    pub(crate) fn detached_panels<'a>(
        &'a self,
        window: iced::window::Id,
        session: &'a model::Session,
        size: iced::Size,
        strip: f32,
    ) -> Element<'a, Message> {
        if session.panes.len() == 1
            || crate::pane_layout::is_narrow_layout_with(
                size.width,
                session.panes.len(),
                &self.enz_theme().sizing,
            )
        {
            return session
                .panes
                .get(session.active_pane)
                .map_or_else(grid_view::empty_grid, |pane| {
                    self.detached_pane(window, session, size, strip, &pane.id)
                });
        }
        let grid = enzui_widgets::enz_pane_grid(
            &session.pane_grid,
            move |id, _| self.detached_pane(window, session, size, strip, id),
            Message::PaneGridEvent,
        )
        .id_base(format!("detached-pane-grid-{window}"))
        .seam_color(grid_view::pane_seam())
        .into_element();
        session
            .panes
            .get(session.active_pane)
            .map_or(grid_view::empty_grid(), |pane| {
                self.scope_element(&session.id, &pane.id, grid)
            })
    }
}
