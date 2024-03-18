use crate::ui::application::Message;
use iced::widget::{button, container, tooltip, Container, Text};
use iced::{Color, Element, Length};

pub fn untitled_text_table_box() -> Container<'static, Message> {
    let message = "Tip: use the space bar to play/pause";
    let text = Text::new(message).color(Color::WHITE);
    let container = Container::new(text)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(20);
    container
}

pub fn action_gated<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let action = button(container(content).center_x(30));

    if let Some(on_press) = on_press {
        tooltip(
            action.on_press(on_press),
            label,
            tooltip::Position::FollowCursor,
        )
        .style(container::rounded_box)
        .into()
    } else {
        action.style(button::secondary).into()
    }
}

pub fn action_toggle<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    on_press: Message,
    pressed: bool,
) -> Element<'a, Message> {
    let action = button(container(content).center_x(30));

    let action = if pressed {
        action.style(button::secondary)
    } else {
        action
    };

    tooltip(
        action.on_press(on_press),
        label,
        tooltip::Position::FollowCursor,
    )
    .style(container::rounded_box)
    .into()
}
