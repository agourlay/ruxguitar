//! The values offered by the control bar's pick lists.

use crate::parser::song_parser::Song;
use crate::ui::tuning::tuning_label;
use std::fmt::Display;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TempoSelection {
    pub percentage: u32,
}

impl Default for TempoSelection {
    fn default() -> Self {
        Self::new(100)
    }
}

impl TempoSelection {
    pub const fn new(percentage: u32) -> Self {
        Self { percentage }
    }

    /// The next preset up, if any.
    pub fn faster(self) -> Option<Self> {
        let index = Self::PRESET.iter().position(|t| *t == self)?;
        Self::PRESET.get(index + 1).copied()
    }

    /// The next preset down, if any.
    pub fn slower(self) -> Option<Self> {
        let index = Self::PRESET.iter().position(|t| *t == self)?;
        index.checked_sub(1).map(|i| Self::PRESET[i])
    }

    pub const PRESET: [Self; 9] = [
        Self::new(25),
        Self::new(50),
        Self::new(60),
        Self::new(70),
        Self::new(80),
        Self::new(90),
        Self::new(100),
        Self::new(150),
        Self::new(200),
    ];
}

impl Display for TempoSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.percentage)
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TrackSelection {
    pub index: usize,
    name: String,
    tuning: Option<String>,
}

impl TrackSelection {
    pub const fn new(index: usize, name: String, tuning: Option<String>) -> Self {
        Self {
            index,
            name,
            tuning,
        }
    }
}

impl TrackSelection {
    /// One per track of the song, with the tuning of the pitched ones.
    pub fn all_of(song: &Song) -> Vec<Self> {
        song.tracks
            .iter()
            .enumerate()
            .map(|(index, track)| {
                let tuning = song
                    .midi_channels
                    .iter()
                    .find(|c| c.channel_id == track.channel_id)
                    .filter(|c| !c.is_percussion())
                    .and_then(|_| tuning_label(&track.strings));
                Self::new(index, track.name.clone(), tuning)
            })
            .collect()
    }
}

impl Display for TrackSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} - {}", self.index + 1, self.name)?;
        if let Some(tuning) = &self.tuning {
            write!(f, " ({tuning})")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::song_parser::{DEFAULT_BANK, DEFAULT_PERCUSSION_BANK, MidiChannel, Track};

    fn channel(channel_id: u8, bank: u8) -> MidiChannel {
        MidiChannel {
            channel_id,
            effect_channel_id: 0,
            instrument: 0,
            volume: 0,
            balance: 0,
            chorus: 0,
            reverb: 0,
            phaser: 0,
            tremolo: 0,
            bank,
        }
    }

    fn track(name: &str, channel_id: u8) -> Track {
        Track {
            name: name.to_string(),
            channel_id,
            strings: vec![(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)],
            ..Default::default()
        }
    }

    #[test]
    fn only_pitched_tracks_show_a_tuning() {
        let song = Song {
            tracks: vec![track("Guitar", 0), track("Drums", 9)],
            midi_channels: vec![
                channel(0, DEFAULT_BANK),
                channel(9, DEFAULT_PERCUSSION_BANK),
            ],
            ..Default::default()
        };
        let labels: Vec<String> = TrackSelection::all_of(&song)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(labels, ["1 - Guitar (Standard E)", "2 - Drums"]);
    }

    #[test]
    fn tempo_steps_through_the_presets() {
        let normal = TempoSelection::default();
        assert_eq!(normal.faster(), Some(TempoSelection::new(150)));
        assert_eq!(normal.slower(), Some(TempoSelection::new(90)));
    }

    #[test]
    fn tempo_stops_at_both_ends() {
        assert_eq!(TempoSelection::new(200).faster(), None);
        assert_eq!(TempoSelection::new(25).slower(), None);
    }

    #[test]
    fn a_restored_tempo_off_the_presets_does_not_step() {
        // a session may hold any percentage, the steps only know the presets
        assert_eq!(TempoSelection::new(75).faster(), None);
        assert_eq!(TempoSelection::new(75).slower(), None);
    }

    #[test]
    fn a_track_shows_its_tuning_when_known() {
        let guitar = TrackSelection::new(0, "Guitar".to_string(), Some("Drop D".to_string()));
        assert_eq!(guitar.to_string(), "1 - Guitar (Drop D)");
        let drums = TrackSelection::new(2, "Drums".to_string(), None);
        assert_eq!(drums.to_string(), "3 - Drums");
    }
}
