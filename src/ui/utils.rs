use crate::ui::application::Message;
use iced::theme::Palette;
use iced::widget::{
    Container, button, center, column, container, mouse_area, opaque, stack, text, tooltip,
};
use iced::{Alignment, Color, Element, Length, Theme};

// Shared UI colors
pub const COLOR_DARK_RED: Color = Color::from_rgb8(200, 50, 50);

/// The application's theme on a light or dark tone: iced's, with the
/// tablature's accent in place of its blue, so the controls, the selection
/// and the tab share one color.
pub fn rux_theme(dark: bool) -> Theme {
    let (name, base) = if dark {
        ("Ruxguitar Dark", Palette::DARK)
    } else {
        ("Ruxguitar Light", Palette::LIGHT)
    };
    Theme::custom(
        name,
        Palette {
            primary: COLOR_DARK_RED,
            ..base
        },
    )
}

/// A toggle button: neutral while off, inverted while on, so its state
/// reads at any size on either theme tone.
pub fn toggle_style(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if !on {
            return button::secondary(theme, status);
        }
        let palette = theme.palette();
        let background = match status {
            button::Status::Hovered => mix(palette.text, palette.background, 0.2),
            _ => palette.text,
        };
        button::Style {
            text_color: palette.background,
            ..button::secondary(theme, status).with_background(background)
        }
    }
}

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
    /// Background of the measures selected for looping.
    pub selection: Color,
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
            // a faint tint: the selection must not compete with the notes
            selection: mix(palette.background, palette.primary, 0.15),
        }
    }
}

/// Blend `amount` of `over` into `base`.
pub fn mix(base: Color, over: Color, amount: f32) -> Color {
    Color {
        r: base.r + (over.r - base.r) * amount,
        g: base.g + (over.g - base.g) * amount,
        b: base.b + (over.b - base.b) * amount,
        a: base.a,
    }
}

/// What shows before a tab is open: how to open one, and the shortcuts.
pub fn untitled_text_table_box<'a>(open: Option<Message>) -> Container<'a, Message> {
    const TIPS: [(&str, &str); 9] = [
        ("Space", "play or pause"),
        ("Left / Right", "previous or next measure"),
        ("Up / Down", "previous or next track"),
        ("1 - 9", "jump to a track"),
        ("Ctrl + Up / Down", "tempo up or down"),
        ("S / M", "solo or mute the track"),
        ("Ctrl + W", "close the tab"),
        ("F11", "fullscreen"),
        ("Drag on the tab", "loop the selected measures"),
    ];
    let keys = column(TIPS.map(|(key, _)| text(key).size(14).into()))
        .spacing(6)
        .align_x(Alignment::End);
    let actions = column(TIPS.map(|(_, action)| {
        text(action)
            .size(14)
            .style(|theme: &Theme| text::Style {
                color: Some(mix(theme.palette().background, theme.palette().text, 0.7)),
            })
            .into()
    }))
    .spacing(6);
    let open_button = button(text("Open a tab").size(16))
        .padding([8, 20])
        .on_press_maybe(open);
    let content = column![
        text("Ruxguitar").size(28),
        text("Guitar Pro tabs, from GP1 to GP7").size(14),
        open_button,
        text("or drop a file anywhere in the window").size(14),
        iced::widget::row![keys, actions].spacing(16),
    ]
    .spacing(14)
    .align_x(Alignment::Center);

    Container::new(content)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(20)
}

/// A toolbar action: neutral, or in the accent for the `main` one. Without
/// `on_press` it shows disabled.
pub fn action_gated<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    on_press: Option<Message>,
    main: bool,
) -> Element<'a, Message> {
    let style = if main {
        button::primary
    } else {
        button::secondary
    };
    let action = button(container(content).center_x(30)).style(style);

    if let Some(on_press) = on_press {
        tooltip(
            action.on_press(on_press),
            label,
            tooltip::Position::FollowCursor,
        )
        .style(container::rounded_box)
        .into()
    } else {
        action.into()
    }
}

/// A toolbar toggle, see [`toggle_style`].
pub fn action_toggle<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    on_press: Message,
    pressed: bool,
) -> Element<'a, Message> {
    let action = button(container(content).center_x(30)).style(toggle_style(pressed));

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
