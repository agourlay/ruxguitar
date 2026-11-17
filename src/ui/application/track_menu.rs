//! The toolbar's track picker and the menu it opens, with per-track solo and
//! mute, built from stock widgets.

use iced::advanced::text::Shaping::Auto;
use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, float, mouse_area, opaque, pick_list, row, scrollable, space, stack,
    text, tooltip,
};
use iced::{Alignment, Element, Length, Theme, Vector};

use crate::ui::icons::{mute_icon, solo_icon};

use super::app::{LoadedSong, Message, RuxApplication};

/// Fixed so the toolbar does not shift when switching between track names.
const PICKER_WIDTH: f32 = 200.0;
const MENU_WIDTH: f32 = 420.0;
const TEXT_SIZE: f32 = 14.0;

/// A single-line track name, clipped instead of wrapped when too long.
fn track_label<'a>(name: String) -> Element<'a, Message> {
    container(
        text(name)
            .size(TEXT_SIZE)
            .shaping(Auto)
            .wrapping(Wrapping::None),
    )
    .width(Length::Fill)
    .clip(true)
    .into()
}

/// A solo/mute toggle in a track row: grey when off, highlighted when on, so a
/// long list of tracks reads at a glance.
fn track_toggle<'a>(
    icon: Element<'a, Message>,
    label: &'a str,
    on_press: Message,
    is_on: bool,
) -> Element<'a, Message> {
    let toggle = button(container(icon).center_x(30))
        .on_press(on_press)
        .style(move |theme: &Theme, status| {
            if is_on {
                button::primary(theme, status)
            } else {
                button::secondary(theme, status)
            }
        });
    tooltip(toggle, label, tooltip::Position::FollowCursor)
        .style(container::rounded_box)
        .into()
}

impl RuxApplication {
    /// The current track, styled like the stock pick list; opens the track menu.
    pub(super) fn track_picker<'a>(&self, song: &'a LoadedSong) -> Element<'a, Message> {
        let is_open = self.track_menu_open;
        let content = row![
            track_label(song.track.to_string()),
            text("\u{25BE}").size(TEXT_SIZE)
        ]
        .spacing(5)
        .align_y(Alignment::Center);
        button(content)
            .width(PICKER_WIDTH)
            .padding([5, 10])
            .on_press(Message::ToggleTrackMenu)
            .style(move |theme: &Theme, status| {
                let is_hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                let status = match (is_open, is_hovered) {
                    (true, is_hovered) => pick_list::Status::Opened { is_hovered },
                    (false, true) => pick_list::Status::Hovered,
                    (false, false) => pick_list::Status::Active,
                };
                let style = pick_list::default(theme, status);
                button::Style {
                    background: Some(style.background),
                    text_color: style.text_color,
                    border: style.border,
                    ..button::Style::default()
                }
            })
            .into()
    }

    /// Lays the open track menu over `content`, right-aligned under the picker
    /// that ends the toolbar; a click anywhere else closes it.
    pub(super) fn with_track_menu<'a>(
        &'a self,
        content: impl Into<Element<'a, Message>>,
        gap_above: f32,
    ) -> Element<'a, Message> {
        let content = content.into();
        let Some(song) = self
            .song
            .as_ref()
            .filter(|_| self.track_menu_open && !self.is_fullscreen)
        else {
            return content;
        };
        let backdrop = opaque(
            mouse_area(space().width(Length::Fill).height(Length::Fill))
                .on_press(Message::CloseTrackMenu),
        );
        // float up over the gap so the menu hangs right under the toolbar
        let menu = float(self.track_menu(song)).translate(move |_, _| Vector::new(0.0, -gap_above));
        stack![content, backdrop, container(menu).align_right(Length::Fill)].into()
    }

    fn track_menu<'a>(&'a self, song: &'a LoadedSong) -> Element<'a, Message> {
        let audio_player = self.audio_player.as_ref();
        let rows = song.tracks.iter().map(|track| {
            let is_current = track.index == song.track.index;
            let select = button(track_label(track.to_string()))
                .width(Length::Fill)
                .on_press(Message::TrackSelected(track.clone()))
                .style(move |theme: &Theme, status| {
                    if is_current {
                        button::primary(theme, status)
                    } else {
                        button::text(theme, status)
                    }
                });
            row![
                select,
                track_toggle(
                    solo_icon(),
                    "Solo",
                    Message::ToggleTrackSolo(track.index),
                    audio_player.is_some_and(|p| p.is_track_solo(track.index)),
                ),
                track_toggle(
                    mute_icon(),
                    "Mute",
                    Message::ToggleTrackMute(track.index),
                    audio_player.is_some_and(|p| p.is_track_muted(track.index)),
                ),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into()
        });
        container(scrollable(column(rows).spacing(2).padding(6)))
            .width(MENU_WIDTH)
            .style(container::bordered_box)
            .into()
    }
}
