use crate::{Message, operation_origin::OperationOrigin};
use iced::advanced::input_method::{InputMethod, Preedit, Purpose};
use iced::advanced::{
    Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Operation, Tree, tree},
};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

pub(crate) fn surface<'a>(
    child: Element<'a, Message>,
    origin: OperationOrigin,
    enabled: bool,
    preedit: Option<Preedit>,
    caret: Rectangle,
    selection: SelectionSource<'a>,
) -> Element<'a, Message> {
    Element::new(InputSurface {
        child,
        origin,
        enabled,
        preedit,
        caret,
        selection,
    })
}

type SelectionResolver<'a> =
    dyn Fn(iced::Point, Rectangle) -> Option<(String, enzui_core::cancellation::CancelToken)> + 'a;

pub(crate) struct SelectionSource<'a> {
    pub(crate) epoch: crate::operation_origin::RequestGeneration,
    pub(crate) range: Option<crate::selection::SelectionRange>,
    pub(crate) resolve: Box<SelectionResolver<'a>>,
}

struct InputSurface<'a> {
    child: Element<'a, Message>,
    origin: OperationOrigin,
    enabled: bool,
    preedit: Option<Preedit>,
    caret: Rectangle,
    selection: SelectionSource<'a>,
}

struct SurfaceState {
    owner: OperationOrigin,
    epoch: crate::operation_origin::RequestGeneration,
    range: Option<crate::selection::SelectionRange>,
    copy_lifetime: enzui_core::cancellation::CancelToken,
}

impl Drop for SurfaceState {
    fn drop(&mut self) {
        self.copy_lifetime
            .cancel(enzui_core::cancellation::CancelCause::Cancelled);
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for InputSurface<'_> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<SurfaceState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(SurfaceState {
            owner: self.origin.clone(),
            epoch: self.selection.epoch.clone(),
            range: self.selection.range,
            copy_lifetime: enzui_core::cancellation::CancelToken::new(),
        })
    }
    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<SurfaceState>();
        if state.owner != self.origin
            || state.epoch != self.selection.epoch
            || state.range != self.selection.range
        {
            state
                .copy_lifetime
                .cancel(enzui_core::cancellation::CancelCause::Cancelled);
            state.copy_lifetime = enzui_core::cancellation::CancelToken::new();
            state.owner = self.origin.clone();
            state.epoch = self.selection.epoch.clone();
            state.range = self.selection.range;
        }
        tree.diff_children(std::slice::from_mut(&mut self.child));
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::pass_through(tree, 0);
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.child.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &iced::Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.child.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            viewport,
            renderer,
            operation,
        );
        let lifetime = tree
            .state
            .downcast_ref::<SurfaceState>()
            .copy_lifetime
            .share();
        enzui_widgets::text_drag::offer_read_only_selection(
            operation,
            None,
            layout.bounds().intersection(viewport).unwrap_or_default(),
            |point| {
                (self.selection.resolve)(point, layout.bounds())
                    .and_then(|(text, owner)| (!owner.is_terminal()).then_some((text, lifetime)))
            },
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
        let bounds = layout.bounds();
        if let Event::Mouse(iced::mouse::Event::ButtonPressed(button)) = event {
            let inside = cursor.is_over(bounds);
            if inside || self.enabled {
                shell.publish(Message::PaneInput(
                    self.origin.clone(),
                    Box::new(Message::InputFocus(inside)),
                ));
            }
            if inside && *button == iced::mouse::Button::Right {
                shell.publish(Message::PaneInput(
                    self.origin.clone(),
                    Box::new(Message::InputMenuOpen),
                ));
                shell.capture_event();
            }
        }
        if self.enabled {
            let mut caret = self.caret;
            caret.x = bounds.x + caret.x.min((bounds.width - caret.width).max(0.0));
            caret.y = bounds.y + caret.y.min((bounds.height - caret.height).max(0.0));
            shell.request_input_method(&InputMethod::Enabled {
                cursor: caret,
                purpose: Purpose::Terminal,
                preedit: self.preedit.as_ref().map(Preedit::as_ref),
            });
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.child.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: iced::Size,
    ) -> Vec<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.child.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}
