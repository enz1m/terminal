//! Shared rendering and input helpers for retained application workflows.

use enzui_core::collect::{CollectedNode, SemanticsCollector};
use iced::{Element, Point, Size};

fn headless_renderer() -> iced_test::renderer::Renderer {
    use iced::advanced::renderer::Headless;
    iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer")
}

pub(crate) fn nodes_of<'a, Message: 'a>(
    view: Element<'a, Message>,
    size: Size,
) -> Vec<CollectedNode> {
    use iced::advanced::widget::Operation;
    let mut renderer = headless_renderer();
    let mut ui = iced_test::runtime::user_interface::UserInterface::build(
        view,
        size,
        iced_test::runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    ui = ui.relayout(size, &mut renderer);
    let mut collector = SemanticsCollector::new();
    ui.operate(&renderer, &mut collector as &mut dyn Operation);
    crate::i18n::test_nodes(collector.into_collected())
}

pub(crate) fn click_at<'a, Message: 'a>(
    view: Element<'a, Message>,
    size: Size,
    at: Point,
) -> Vec<Message> {
    let mut ui = iced_test::Simulator::with_size(iced::Settings::default(), size, view);
    ui.point_at(at);
    let _ = ui.simulate(iced_test::simulator::click());
    ui.into_messages().collect()
}
