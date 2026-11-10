//! The application layout: control bar, tablature and status bar.

use iced::advanced::text::Shaping::Auto;
use iced::widget::space::horizontal;
use iced::widget::{Text, column, container, pick_list, row, rule, slider, text};
use iced::{Alignment, Border, Element, Length, Theme};

use crate::audio::midi_player::AudioPlayer;
use crate::parser::song_parser::{MeasureHeader, QUARTER_TIME};
use crate::ui::icons::{
    count_in_icon, metronome_icon, mute_icon, open_icon, pause_icon, play_icon, solo_icon,
    stop_icon,
};
use crate::ui::selections::TempoSelection;
use crate::ui::track_picker::TrackPicker;
use crate::ui::song_info::SongDisplayInfo;
use crate::ui::utils::{
    TablatureColors, action_gated, action_toggle, modal, untitled_text_table_box,
};

use super::app::{Message, RuxApplication};

impl RuxApplication {
    pub(super) fn view(&self) -> Element<'_, Message> {
        let tablature_view = self.song.as_ref().map_or_else(
            || untitled_text_table_box().into(),
            |song| song.tablature.view(),
        );
        let tablature = container(tablature_view).id(self.tablature_id.clone());

        let base: Element<Message> = if self.is_fullscreen {
            column![tablature].spacing(20).padding(10).into()
        } else {
            column![
                self.control_bar(),
                tablature,
                rule::horizontal(1),
                self.status_bar(),
            ]
            .spacing(20)
            .padding(10)
            .into()
        };

        // add error modal if any
        if let Some(error_message) = &self.error_message {
            let error_view = text(error_message).size(20);
            modal(base, error_view, Message::ClearError)
        } else {
            base
        }
    }

    /// The bar above the tablature: open, transport and playback settings.
    fn control_bar(&self) -> Element<'_, Message> {
        let open_file = action_gated(
            open_icon(),
            "Open file",
            (!self.tab_file_is_loading).then_some(Message::OpenFileDialog),
        );
        let controls = row![
            open_file,
            horizontal(),
            self.transport_controls(),
            horizontal(),
            self.playback_settings(),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        container(controls)
            .padding(10)
            .style(|theme: &Theme| container::Style {
                // a muted border that reads on either theme tone
                border: Border::default()
                    .color(TablatureColors::of(theme).string_line)
                    .width(1),
                ..Default::default()
            })
            .into()
    }

    /// Play/pause, stop and where playback stands, once a song can play.
    fn transport_controls(&self) -> Element<'_, Message> {
        let Some(audio_player) = &self.audio_player else {
            return row![horizontal()].into();
        };
        let (icon, message) = if audio_player.is_playing() {
            (pause_icon(), "Pause")
        } else {
            (play_icon(), "Play")
        };
        let play_button = action_gated(icon, message, Some(Message::PlayPause));
        let stop_button = action_gated(stop_icon(), "Stop", Some(Message::StopPlayer));
        let counter = self
            .song
            .as_ref()
            .map(|song| {
                let tablature = &song.tablature;
                position_label(
                    &tablature.song.measure_headers,
                    tablature.focused_measure(),
                    tablature.measure_count(),
                )
            })
            .unwrap_or_default();
        row![play_button, stop_button, text(counter).size(14)]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
    }

    /// Tempo, volume, metronome, count-in, solo, mute and track, once a
    /// song is open.
    fn playback_settings(&self) -> Element<'_, Message> {
        let Some(song) = &self.song else {
            return row![horizontal()].into();
        };
        let tempo_percentage = pick_list(
            TempoSelection::PRESET,
            Some(&self.prefs.tempo),
            Message::TempoSelected,
        )
        .text_size(14)
        .padding([5, 10]);
        let solo_mode = action_toggle(
            solo_icon(),
            "Solo",
            Message::ToggleSolo,
            self.audio_player
                .as_ref()
                .is_some_and(|p| p.solo_track_id().is_some()),
        );
        let mute_mode = action_toggle(
            mute_icon(),
            "Mute",
            Message::ToggleMute,
            self.audio_player
                .as_ref()
                .is_some_and(|p| p.is_track_muted(song.track.index)),
        );
        let metronome_mode = action_toggle(
            metronome_icon(),
            "Metronome",
            Message::ToggleMetronome,
            self.prefs.metronome,
        );
        let count_in_mode = action_toggle(
            count_in_icon(),
            "Count-in",
            Message::ToggleCountIn,
            self.prefs.count_in,
        );
        let solo_track = self
            .audio_player
            .as_ref()
            .and_then(AudioPlayer::solo_track_id);
        let track_mutes: Vec<bool> = song
            .tracks
            .iter()
            .map(|track| {
                self.audio_player
                    .as_ref()
                    .is_some_and(|p| p.is_track_muted(track.index))
            })
            .collect();
        let track_pick_list = TrackPicker::new(
            song.tracks.as_slice(),
            &song.track,
            move |index| solo_track == Some(index),
            move |index| track_mutes.get(index).copied().unwrap_or(false),
            Message::TrackSelected,
            Message::ToggleTrackSolo,
            Message::ToggleTrackMute,
        )
        .view();
        let current_volume = self
            .audio_player
            .as_ref()
            .map_or(self.prefs.master_volume, AudioPlayer::master_volume);
        let volume_slider = slider(0.0..=1.0, current_volume, Message::MasterVolumeChanged)
            .step(0.01_f32)
            .width(100);

        row![
            text("Tempo").size(14),
            tempo_percentage,
            text("Volume").size(14),
            volume_slider,
            metronome_mode,
            count_in_mode,
            solo_mode,
            mute_mode,
            track_pick_list,
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    }

    /// The bar under the tablature: title, metadata and file format.
    fn status_bar(&self) -> Element<'_, Message> {
        let song_info = self.song.as_ref().map(|song| &song.info);
        let title_line = song_info
            .map(SongDisplayInfo::title_line)
            .unwrap_or_default();
        let metadata_line = song_info
            .and_then(SongDisplayInfo::metadata_line)
            .unwrap_or_default();
        let gp_version = song_info
            .map(|info| format!("{:?}", info.gp_version))
            .unwrap_or_default();
        let status = row![
            container(Text::new(title_line).shaping(Auto))
                .width(Length::FillPortion(1))
                .align_x(Alignment::Start),
            container(Text::new(metadata_line).shaping(Auto))
                .width(Length::FillPortion(1))
                .align_x(Alignment::Center),
            container(text(gp_version))
                .width(Length::FillPortion(1))
                .align_x(Alignment::End),
        ]
        .spacing(10);
        container(status).padding(4).into()
    }
}

/// Where playback stands: the focused measure out of all of them, and the
/// time it starts at out of the song's length.
fn position_label(headers: &[MeasureHeader], focused: usize, total_measures: usize) -> String {
    format!(
        "Measure {}/{} \u{2022} {}/{}",
        focused + 1,
        total_measures,
        format_mmss(song_time_up_to_measure(headers, focused)),
        format_mmss(song_time_up_to_measure(headers, total_measures)),
    )
}

/// Seconds elapsed from the song's start up to (but not including) `measure_idx`.
/// Tempo changes across measures are honored. Repeats are ignored — we compute
/// the song's linear duration, not expanded playback time.
fn song_time_up_to_measure(headers: &[MeasureHeader], measure_idx: usize) -> f32 {
    headers
        .iter()
        .take(measure_idx)
        .enumerate()
        .map(|(i, h)| {
            let next_start = headers
                .get(i + 1)
                .map_or(h.start + h.length(), |next| next.start);
            let duration_ticks = next_start.saturating_sub(h.start) as f32;
            duration_ticks / QUARTER_TIME as f32 * 60.0 / h.tempo.value as f32
        })
        .sum()
}

fn format_mmss(seconds: f32) -> String {
    let total = seconds.max(0.0) as u32;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::song_parser::Tempo;

    fn measure(start: u32, bpm: u32) -> MeasureHeader {
        MeasureHeader {
            start,
            tempo: Tempo::new(bpm, None),
            ..Default::default()
        }
    }

    #[test]
    fn song_time_follows_tempo_changes() {
        let bar = QUARTER_TIME * 4;
        // two 4/4 bars at 120 bpm, then one at 60 bpm
        let headers = [
            measure(QUARTER_TIME, 120),
            measure(QUARTER_TIME + bar, 120),
            measure(QUARTER_TIME + 2 * bar, 60),
        ];
        assert_eq!(song_time_up_to_measure(&headers, 0), 0.0);
        assert_eq!(song_time_up_to_measure(&headers, 2), 4.0);
        assert_eq!(song_time_up_to_measure(&headers, 3), 8.0);
    }

    #[test]
    fn the_position_reads_as_measure_and_time() {
        let bar = QUARTER_TIME * 4;
        let headers = [measure(QUARTER_TIME, 120), measure(QUARTER_TIME + bar, 120)];
        assert_eq!(
            position_label(&headers, 1, 2),
            "Measure 2/2 \u{2022} 0:02/0:04"
        );
    }

    #[test]
    fn times_read_as_minutes_and_seconds() {
        assert_eq!(format_mmss(0.0), "0:00");
        assert_eq!(format_mmss(65.9), "1:05");
        assert_eq!(format_mmss(-3.0), "0:00");
    }
}
