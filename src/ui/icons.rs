//! Icons coming from <https://fontello.com/>

use iced::advanced::text::Shaping;
use iced::widget::text;
use iced::{Element, Font};

pub fn open_icon<'a, Message>() -> Element<'a, Message> {
    icon('\u{0f115}')
}

pub fn solo_icon<'a, Message>() -> Element<'a, Message> {
    text('S').into()
}

pub fn mute_icon<'a, Message>() -> Element<'a, Message> {
    text('M').into()
}

pub fn metronome_icon<'a, Message>() -> Element<'a, Message> {
    // musical symbols live outside the basic plane: needs advanced shaping
    text('\u{1D15F}').shaping(Shaping::Advanced).into()
}

pub fn count_in_icon<'a, Message>() -> Element<'a, Message> {
    text("1·2").into()
}

pub fn pause_icon<'a, Message>() -> Element<'a, Message> {
    icon('\u{0e802}')
}

pub fn play_icon<'a, Message>() -> Element<'a, Message> {
    icon('\u{0e800}')
}

pub fn stop_icon<'a, Message>() -> Element<'a, Message> {
    icon('\u{0e801}')
}

fn icon<'a, Message>(codepoint: char) -> Element<'a, Message> {
    const ICON_FONT: Font = Font::with_name("ruxguitar-icons");

    text(codepoint).font(ICON_FONT).into()
}
