use crate::ui::application::Message;
use iced::widget::{
    Container, Text, button, center, container, mouse_area, opaque, stack, tooltip,
};
use iced::{Color, Element, Length, Theme};

// Shared UI colors
pub const COLOR_DARK_RED: Color = Color::from_rgb8(200, 50, 50);

/// Colors the tablature draws with, taken from the active theme so the tab
/// follows the system light or dark setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TablatureColors {
    /// Notes, bar lines and effect glyphs.
    pub foreground: Color,
    /// String lines, which sit behind the notes.
    pub string_line: Color,
    /// Measure numbers and the beat being played.
    pub accent: Color,
}

impl TablatureColors {
    pub fn of(theme: &Theme) -> Self {
        let palette = theme.palette();
        Self {
            foreground: palette.text,
            // the strings recede behind the notes: mostly background, with
            // enough text mixed in to stay visible on either tone
            string_line: mix(palette.background, palette.text, 0.35),
            accent: COLOR_DARK_RED,
        }
    }
}

/// Blend `amount` of `over` into `base`.
fn mix(base: Color, over: Color, amount: f32) -> Color {
    Color {
        r: base.r + (over.r - base.r) * amount,
        g: base.g + (over.g - base.g) * amount,
        b: base.b + (over.b - base.b) * amount,
        a: base.a,
    }
}

pub fn untitled_text_table_box() -> Container<'static, Message> {
    let message = "Tips:\n \
        - use the space bar to play/pause\n \
        - use ctrl+up/down to change the tempo\n \
        - use left/right to navigate measures\n \
        - use s to toggle solo mode\n \
        - use F11 to toggle fullscreen";
    let text = Text::new(message);

    Container::new(text)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(20)
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

pub fn modal<'a, Message>(
    base: impl Into<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    on_blur: Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    stack![
        base.into(),
        opaque(
            mouse_area(center(opaque(content)).style(|_theme| {
                container::Style {
                    background: Some(
                        Color {
                            a: 0.8,
                            ..Color::BLACK
                        }
                        .into(),
                    ),
                    ..container::Style::default()
                }
            }))
            .on_press(on_blur)
        )
    ]
    .into()
}
