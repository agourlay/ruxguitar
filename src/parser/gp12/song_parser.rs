//! Guitar Pro 1 and 2 (`.gtp`) files.
//!
//! A port of TuxGuitar's `GP1InputStream` and `GP2InputStream`. Both formats
//! hold up to eight tracks on fixed MIDI channels, the last one for drums,
//! and store each measure's beats track after track.

use crate::RuxError;
use crate::parser::gp345::primitive_parser::{
    make_string, parse_bool, parse_i8, parse_int, parse_u8, skip,
};
use crate::parser::gp345::song_parser::convert_repeat_alternative;
use crate::parser::model::{
    Beat, BendEffect, BendPoint, Chord, DEFAULT_BANK, DEFAULT_PERCUSSION_BANK, DEFAULT_VELOCITY,
    Duration, GpVersion, HarmonicEffect, HarmonicType, KeySignature, Measure, MeasureHeader,
    MidiChannel, Note, NoteEffect, NoteType, QUARTER_TIME, SlapEffect, SlideType, Song, SongInfo,
    Tempo, TimeSignature, Track, TripletFeel, Voice, convert_velocity,
};
use nom::IResult;
use nom::bytes::complete::take;

/// The versions of the format, in the order they appeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Format {
    V1_00,
    V1_01,
    V1_02,
    V1_03,
    V1_04,
    V2,
}

impl Format {
    fn from_version(version: &[u8]) -> Option<Self> {
        Some(match version {
            b"FICHIER GUITARE PRO v1" => Self::V1_00,
            b"FICHIER GUITARE PRO v1.01" => Self::V1_01,
            b"FICHIER GUITARE PRO v1.02" => Self::V1_02,
            b"FICHIER GUITARE PRO v1.03" => Self::V1_03,
            b"FICHIER GUITARE PRO v1.04" => Self::V1_04,
            b"FICHIER GUITAR PRO v2.20" | b"FICHIER GUITAR PRO v2.21" => Self::V2,
            _ => return None,
        })
    }

    /// Up to version 1.02 a file holds a single track.
    const fn track_count(self) -> usize {
        match self {
            Self::V1_00 | Self::V1_01 | Self::V1_02 => 1,
            _ => 8,
        }
    }

    const fn gp_version(self) -> GpVersion {
        match self {
            Self::V2 => GpVersion::GP2,
            _ => GpVersion::GP1,
        }
    }
}

/// `(main, effect)` MIDI channels of each of the eight tracks.
const TRACK_CHANNELS: [(u8, u8); 8] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (8, 10),
    (11, 12),
    (13, 14),
    (9, 9),
];

/// The version string: its length, then a 30 bytes field.
fn parse_format(i: &[u8]) -> IResult<&[u8], Option<Format>> {
    let (i, length) = parse_u8(i)?;
    let (i, field) = take(30usize)(i)?;
    let version = &field[..usize::from(length).min(30)];
    Ok((i, Format::from_version(version)))
}

/// Whether the data holds a Guitar Pro 1 or 2 file.
pub fn is_gp12_data(data: &[u8]) -> bool {
    matches!(parse_format(data), Ok((_, Some(_))))
}

/// A string as a length byte, then a field as long as the string.
fn parse_byte_string(i: &[u8]) -> IResult<&[u8], String> {
    let (i, length) = parse_u8(i)?;
    let (i, field) = take(length)(i)?;
    Ok((i, make_string(field)))
}

/// A string preceded by its field size plus one, as TuxGuitar's
/// `readStringByteSizeOfByte`: [u8 field size + 1][u8 string length][field].
/// Without a field size, the field is as long as the string.
fn parse_sized_byte_string(i: &[u8]) -> IResult<&[u8], String> {
    let (i, size) = parse_u8(i)?;
    let (i, length) = parse_u8(i)?;
    let field_size = match size.saturating_sub(1) {
        0 => usize::from(length),
        size => usize::from(size),
    };
    let (i, field) = take(field_size)(i)?;
    let length = usize::from(length).min(field_size);
    Ok((i, make_string(&field[..length])))
}

/// A channel setting, stored as a MIDI value.
fn channel_setting(value: i32) -> i8 {
    value.clamp(0, 127) as i8
}

/// The key as a number of sharps, or of flats when negative. Like TuxGuitar,
/// 8 to 14 stand for 1 to 7 flats.
fn key_signature(key: i32) -> i8 {
    match key {
        8..=14 => (7 - key) as i8,
        key => key.clamp(-7, 7) as i8,
    }
}

fn verify_failure<T>(i: &[u8]) -> IResult<&[u8], T> {
    Err(nom::Err::Failure(nom::error::Error::new(
        i,
        nom::error::ErrorKind::Verify,
    )))
}

/// Parse a Guitar Pro 1 or 2 file.
pub fn parse_gp12_data(data: &[u8]) -> Result<Song, RuxError> {
    let (_, song) = parse_song(data)
        .map_err(|_err| RuxError::ParsingError("Failed to parse GP1/GP2 data".to_string()))?;
    Ok(song)
}

fn parse_song(i: &[u8]) -> IResult<&[u8], Song> {
    let (i, format) = parse_format(i)?;
    let Some(format) = format else {
        return verify_failure(i);
    };
    let track_count = format.track_count();

    let (i, name) = parse_sized_byte_string(i)?;
    let (i, artist) = parse_sized_byte_string(i)?;
    // who wrote the tab, often with a way to reach them
    let (i, writer) = parse_sized_byte_string(i)?;
    let (i, tempo) = parse_int(i)?;
    let (i, triplet_feel) = parse_int(i)?;
    let triplet_feel = triplet_feel == 1;
    let (mut i, key) = if format >= Format::V1_03 {
        parse_int(i)?
    } else {
        (i, 0)
    };
    let key = key_signature(key);

    let mut tracks = Vec::with_capacity(track_count);
    for (index, &(channel_id, _)) in TRACK_CHANNELS.iter().take(track_count).enumerate() {
        let (inner, string_count) = if format >= Format::V1_02 {
            parse_int(i)?
        } else {
            (i, 6)
        };
        i = inner;
        if !(1..=7).contains(&string_count) {
            return verify_failure(i);
        }
        let mut strings = Vec::with_capacity(string_count as usize);
        for number in 1..=string_count {
            let (inner, tuning) = parse_int(i)?;
            i = inner;
            strings.push((number, tuning));
        }
        tracks.push(Track {
            number: index as i32 + 1,
            channel_id,
            name: "Track 1".to_string(),
            strings,
            ..Default::default()
        });
    }

    let (mut i, measure_count) = parse_int(i)?;
    if !(1..=65536).contains(&measure_count) {
        return verify_failure(i);
    }

    let mut midi_channels = Vec::with_capacity(track_count);
    for (track, &(channel_id, effect_channel_id)) in tracks.iter_mut().zip(&TRACK_CHANNELS) {
        let (inner, channel) = parse_track(format, track, channel_id, effect_channel_id)(i)?;
        i = inner;
        midi_channels.push(channel);
    }
    if format >= Format::V1_03 {
        i = skip(i, 10);
    }

    let mut song = Song {
        version: format.gp_version(),
        song_info: SongInfo {
            name,
            artist,
            writer,
            ..Default::default()
        },
        triplet_feel: Some(triplet_feel),
        tempo: Tempo::new(u32::try_from(tempo).unwrap_or(0).max(1), None),
        key_signature: key,
        midi_channels,
        tracks,
        ..Default::default()
    };
    let (i, ()) =
        MeasureReader::new(format, &mut song).parse_measures(i, measure_count as usize)?;
    Ok((i, song))
}

/// A track's instrument and, from version 1.03, its name and mix.
fn parse_track(
    format: Format,
    track: &mut Track,
    channel_id: u8,
    effect_channel_id: u8,
) -> impl FnMut(&[u8]) -> IResult<&[u8], MidiChannel> + '_ {
    move |i| {
        let (mut i, instrument) = parse_int(i)?;
        let mut channel = MidiChannel {
            channel_id,
            effect_channel_id,
            // like TuxGuitar, drums ignore the program
            instrument: if channel_id == 9 {
                0
            } else {
                instrument.max(0)
            },
            volume: channel_setting(127),
            balance: channel_setting(64),
            chorus: 0,
            reverb: 0,
            phaser: 0,
            tremolo: 0,
            // the last track plays the drums
            bank: if channel_id == 9 {
                DEFAULT_PERCUSSION_BANK
            } else {
                DEFAULT_BANK
            },
        };
        if format >= Format::V1_03 {
            let (inner, fret_count) = parse_int(i)?;
            track.fret_count = fret_count.clamp(0, i32::from(u8::MAX)) as u8;
            let (inner, name) = parse_sized_byte_string(inner)?;
            track.name = name;
            let (inner, solo) = parse_bool(inner)?;
            track.solo = solo;
            let (inner, volume) = parse_int(inner)?;
            let (inner, balance) = parse_int(inner)?;
            let (inner, chorus) = parse_int(inner)?;
            let (inner, reverb) = parse_int(inner)?;
            let (inner, offset) = parse_int(inner)?;
            channel.volume = channel_setting(volume);
            channel.balance = channel_setting(balance);
            channel.chorus = channel_setting(chorus);
            channel.reverb = channel_setting(reverb);
            track.offset = offset;
            i = inner;
        }
        Ok((i, channel))
    }
}

/// The value of a duration byte: 0 is a quarter, each step halves it.
fn duration_of(value: i8) -> Duration {
    // clamp the exponent so corrupt values neither overflow nor reach zero
    let exponent = (i32::from(value) + 4).clamp(2, 8) as u32;
    Duration {
        value: (2_u16.pow(exponent) / 4),
        ..Default::default()
    }
}

/// Reads the measures, keeping what later beats refer back to.
struct MeasureReader<'s> {
    format: Format,
    song: &'s mut Song,
    /// Endings already used since the last repeat start.
    existing_alternatives: u8,
    /// The start of the last beat read on each track, which a tied beat
    /// takes its notes from.
    last_beat_starts: Vec<u32>,
}

impl<'s> MeasureReader<'s> {
    fn new(format: Format, song: &'s mut Song) -> Self {
        let track_count = song.tracks.len();
        Self {
            format,
            song,
            existing_alternatives: 0,
            last_beat_starts: vec![0; track_count],
        }
    }

    fn parse_measures<'a>(&mut self, mut i: &'a [u8], count: usize) -> IResult<&'a [u8], ()> {
        let triplet_feel = if self.song.triplet_feel == Some(true) {
            TripletFeel::Eighth
        } else {
            TripletFeel::None
        };
        let key_signature = KeySignature::new(self.song.key_signature, false);
        let mut start = QUARTER_TIME;
        for header_index in 0..count {
            let tempo = self
                .song
                .measure_headers
                .last()
                .map_or(self.song.tempo.value, |previous| previous.tempo.value);
            let header = MeasureHeader {
                start,
                tempo: Tempo::new(tempo, None),
                triplet_feel,
                key_signature,
                ..Default::default()
            };
            let (inner, ()) = self.parse_measure(i, header)?;
            i = inner;
            let length = self.song.measure_headers[header_index].length();
            if length == 0 {
                return verify_failure(i);
            }
            start += length;
        }
        Ok((i, ()))
    }

    /// A measure: its header, then the beats of each track.
    fn parse_measure<'a>(
        &mut self,
        i: &'a [u8],
        mut header: MeasureHeader,
    ) -> IResult<&'a [u8], ()> {
        let (i, numerator) = parse_u8(i)?;
        let (i, denominator) = parse_u8(i)?;
        if numerator == 0 || !denominator.is_power_of_two() || denominator > 64 {
            return verify_failure(i);
        }
        header.time_signature = TimeSignature {
            numerator,
            denominator: Duration {
                value: u16::from(denominator),
                ..Default::default()
            },
        };
        let mut i = skip(i, 6);

        let track_count = self.song.tracks.len();
        let mut beat_counts = Vec::with_capacity(track_count);
        for _ in 0..track_count {
            let (inner, _) = take(2usize)(i)?;
            let (inner, beats) = parse_u8(inner)?;
            // like TuxGuitar, a count above 127 means no beat
            beat_counts.push(if beats > 127 { 0 } else { usize::from(beats) });
            i = skip(inner, 9);
        }
        let i = skip(i, 2);

        let (mut i, flags) = parse_u8(i)?;
        header.repeat_open = flags & 0x01 != 0;
        if flags & 0x02 != 0 {
            let (inner, repeat_close) = parse_i8(i)?;
            i = inner;
            header.repeat_close = repeat_close;
        }
        if flags & 0x04 != 0 {
            let (inner, alternative) = parse_u8(i)?;
            i = inner;
            header.repeat_alternative =
                convert_repeat_alternative(self.existing_alternatives, alternative);
        }
        if header.repeat_open {
            self.existing_alternatives = 0;
        }
        self.existing_alternatives |= header.repeat_alternative;

        let header_index = self.song.measure_headers.len();
        let measure_start = header.start;
        let measure_end = measure_start + header.length();
        let key_signature = header.key_signature;
        self.song.measure_headers.push(header);

        for (track_index, beat_count) in beat_counts.into_iter().enumerate() {
            self.song.tracks[track_index].measures.push(Measure {
                key_signature,
                track_index,
                header_index,
                voices: vec![Voice {
                    measure_index: header_index as i16,
                    beats: Vec::with_capacity(beat_count),
                }],
                ..Default::default()
            });
            let mut start = measure_start;
            for _ in 0..beat_count {
                let (inner, beat) = self.parse_beat(i, track_index, header_index, start)?;
                i = inner;
                self.last_beat_starts[track_index] = start;
                start += beat.duration.time();
                self.current_beats(track_index).push(beat);
            }
            // like TuxGuitar, rests fill the end of a measure left short
            let rests = fill_with_rests(start, measure_end);
            self.current_beats(track_index).extend(rests);
        }
        Ok((i, ()))
    }

    fn current_beats(&mut self, track_index: usize) -> &mut Vec<Beat> {
        let measure = self.song.tracks[track_index]
            .measures
            .last_mut()
            .expect("no measure being parsed");
        &mut measure.voices[0].beats
    }

    fn parse_beat<'a>(
        &mut self,
        i: &'a [u8],
        track_index: usize,
        header_index: usize,
        start: u32,
    ) -> IResult<&'a [u8], Beat> {
        let i = skip(i, 4);
        let (i, value) = parse_i8(i)?;
        let mut beat = Beat {
            start,
            duration: duration_of(value),
            ..Default::default()
        };
        let (mut i, flags) = parse_u8(i)?;
        if self.format == Format::V2 {
            let (inner, flags2) = parse_u8(i)?;
            i = inner;
            if flags2 & 0x02 != 0 {
                let (inner, ()) = self.parse_mix_change(i, header_index)?;
                i = inner;
            }
            // brush stroke direction and speed, not imported
            if flags2 & 0x01 != 0 {
                i = skip(i, 2);
            }
        }

        beat.duration.dotted = flags & 0x10 != 0;
        if flags & 0x20 != 0 {
            beat.duration.tuplet_enters = 3;
            beat.duration.tuplet_times = 2;
            i = skip(i, 1);
        }

        let mut effect = NoteEffect::default();
        if flags & 0x04 != 0 {
            let (inner, ()) = parse_beat_effects(i, &mut effect)?;
            i = inner;
        }
        let string_count = self.song.tracks[track_index].strings.len();
        if flags & 0x02 != 0 {
            let (inner, chord) = self.parse_chord(i, string_count)?;
            i = inner;
            beat.effect.chord = Some(chord);
        }
        if flags & 0x01 != 0 {
            let (inner, text) = parse_byte_string(i)?;
            i = inner;
            beat.text = text;
        }

        if flags & 0x40 != 0 {
            // the notes of the previous beat ring on
            let previous = self.last_beat_starts[track_index];
            if previous < start {
                beat.notes = self.tied_notes(track_index, previous);
            }
        } else if flags & 0x08 == 0 {
            let (inner, notes) = self.parse_notes(i, string_count, &effect)?;
            i = inner;
            beat.notes = notes;
        }
        Ok((i, beat))
    }

    /// The notes of a beat: a byte telling which strings are played, one
    /// telling which have effects and, from GP2, one telling which have a
    /// grace note, then each note from the first string.
    fn parse_notes<'a>(
        &self,
        i: &'a [u8],
        string_count: usize,
        beat_effect: &NoteEffect,
    ) -> IResult<&'a [u8], Vec<Note>> {
        let (i, string_flags) = parse_u8(i)?;
        let (mut i, effect_flags) = parse_u8(i)?;
        let mut grace_flags = 0;
        if self.format == Format::V2 {
            let (inner, flags) = parse_u8(i)?;
            i = inner;
            grace_flags = flags;
        }
        let mut notes = Vec::new();
        for bit in (0..6).rev() {
            if string_flags & (1 << bit) != 0 {
                let has_effects = effect_flags & (1 << bit) != 0;
                let (inner, note) = self.parse_note(i, has_effects, beat_effect)?;
                i = inner;
                // bit 0 is the last string, so a track with fewer than six
                // strings has none for the highest bits
                let string = string_count as i32 - bit;
                if string >= 1 {
                    notes.push(Note {
                        string: string as i8,
                        ..note
                    });
                }
            }
            // a grace note, not imported; like TuxGuitar, it is read even
            // without a note on its string
            if grace_flags & (1 << bit) != 0 {
                i = skip(i, 3);
            }
        }
        Ok((i, notes))
    }

    /// A note: its fret, from GP2 its dynamic, then its effects if it has any.
    fn parse_note<'a>(
        &self,
        i: &'a [u8],
        has_effects: bool,
        beat_effect: &NoteEffect,
    ) -> IResult<&'a [u8], Note> {
        let (mut i, fret) = parse_u8(i)?;
        let mut velocity = DEFAULT_VELOCITY;
        if self.format == Format::V2 {
            let (inner, dynamic) = parse_u8(i)?;
            i = inner;
            // from pianissimo-pianissimo (1) to forte-fortissimo (8); TuxGuitar
            // passes other values on, which fall outside the MIDI range
            velocity = convert_velocity(i16::from(dynamic.clamp(1, 8)));
        }
        // unlike TuxGuitar, which lets a note's effects spill over to the
        // notes read after it, each note keeps its own
        let mut effect = beat_effect.clone();
        if has_effects {
            let (inner, ()) = parse_note_effects(i, &mut effect)?;
            i = inner;
        }
        let mut note = Note::new(effect);
        note.velocity = velocity;
        if fret < 100 {
            note.value = i16::from(fret);
            note.kind = NoteType::Normal;
        } else {
            note.kind = NoteType::Dead;
        }
        Ok((i, note))
    }

    /// The notes of the beat starting at `start`, tied over. Like TuxGuitar,
    /// it is looked for in the current measure, then the previous ones.
    fn tied_notes(&self, track_index: usize, start: u32) -> Vec<Note> {
        let track = &self.song.tracks[track_index];
        track
            .measures
            .iter()
            .rev()
            .find_map(|measure| measure.voices[0].beats.iter().find(|b| b.start == start))
            .map(|beat| {
                beat.notes
                    .iter()
                    .map(|previous| {
                        let mut note = Note::new(NoteEffect::default());
                        note.value = previous.value;
                        note.string = previous.string;
                        note.velocity = previous.velocity;
                        note.kind = NoteType::Tie;
                        note
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A chord: its name only up to version 1.03, then also its diagram.
    fn parse_chord<'a>(&self, i: &'a [u8], string_count: usize) -> IResult<&'a [u8], Chord> {
        let mut chord = Chord {
            strings: vec![-1; string_count],
            ..Default::default()
        };
        if self.format < Format::V1_04 {
            let (i, name) = parse_sized_byte_string(i)?;
            chord.name = name;
            return Ok((i, chord));
        }
        let (i, name) = parse_byte_string(i)?;
        chord.name = name;
        let i = skip(i, 1);
        let (mut i, kind) = parse_int(i)?;
        if kind < 12 {
            i = skip(i, 32);
        }
        let (mut i, first_fret) = parse_int(i)?;
        chord.first_fret = u32::try_from(first_fret).ok();
        if first_fret != 0 {
            for string in 0..6 {
                let (inner, fret) = parse_int(i)?;
                i = inner;
                if let Some(slot) = chord.strings.get_mut(string) {
                    *slot = fret.clamp(-1, i32::from(i8::MAX)) as i8;
                }
            }
        }
        Ok((i, chord))
    }

    /// A GP2 mix change. Only the tempo is kept, for the rest of the song.
    fn parse_mix_change<'a>(&mut self, i: &'a [u8], header_index: usize) -> IResult<&'a [u8], ()> {
        let (mut i, flags) = parse_u8(i)?;
        if flags & 0x20 != 0 {
            let (inner, tempo) = parse_int(i)?;
            i = skip(inner, 1);
            if let Some(header) = self.song.measure_headers.get_mut(header_index) {
                header.tempo.value = u32::try_from(tempo).unwrap_or(0).max(1);
            }
        }
        // reverb, chorus, balance and volume: a value and a transition each
        for flag in [0x10, 0x08, 0x04, 0x02] {
            if flags & flag != 0 {
                i = skip(i, 2);
            }
        }
        // instrument
        if flags & 0x01 != 0 {
            i = skip(i, 1);
        }
        Ok((i, ()))
    }
}

/// The rests filling a measure from `start` to `end`.
///
/// Like TuxGuitar, the gap is split into equal plain rests, the longest that
/// fit it exactly (three quarters for a 3/4 measure, not a dotted half);
/// a gap no plain rest divides is filled with the longest rests that fit.
fn fill_with_rests(mut start: u32, end: u32) -> Vec<Beat> {
    let durations: Vec<Duration> = [1, 2, 4, 8, 16, 32, 64]
        .into_iter()
        .map(|value| Duration {
            value,
            ..Default::default()
        })
        .collect();
    let mut rests = Vec::new();
    let gap = end.saturating_sub(start);
    let equal = durations
        .iter()
        .find(|d| d.time() <= gap && gap.is_multiple_of(d.time()));
    while start < end {
        let remaining = end - start;
        let Some(duration) = equal.or_else(|| durations.iter().find(|d| d.time() <= remaining))
        else {
            break;
        };
        rests.push(Beat {
            duration: duration.clone(),
            start,
            ..Default::default()
        });
        start += duration.time();
    }
    rests
}

/// Effects applying to every note of a beat.
fn parse_beat_effects<'a>(i: &'a [u8], effect: &mut NoteEffect) -> IResult<&'a [u8], ()> {
    let (i, kind) = parse_u8(i)?;
    match kind {
        1 | 2 => effect.vibrato = true,
        3 => return parse_bend(i, effect),
        4 => effect.fade_in = true,
        5 => effect.slap = SlapEffect::Tapping,
        6 => effect.slap = SlapEffect::Slapping,
        7 => effect.slap = SlapEffect::Popping,
        8 | 9 => {
            effect.harmonic = Some(HarmonicEffect {
                kind: if kind == 8 {
                    HarmonicType::Natural
                } else {
                    HarmonicType::Artificial
                },
                ..Default::default()
            });
        }
        _ => {}
    }
    Ok((i, ()))
}

/// Effects of a single note.
fn parse_note_effects<'a>(i: &'a [u8], effect: &mut NoteEffect) -> IResult<&'a [u8], ()> {
    let (i, kind) = parse_u8(i)?;
    match kind {
        1 | 2 => effect.hammer = true,
        3 | 4 => effect.slide = Some(SlideType::ShiftSlideTo),
        5 | 6 => return parse_bend(i, effect),
        _ => {}
    }
    Ok((i, ()))
}

/// A bend up to a number of semitones, held to the end of the note.
fn parse_bend<'a>(i: &'a [u8], effect: &mut NoteEffect) -> IResult<&'a [u8], ()> {
    let i = skip(i, 6);
    let (i, value) = parse_u8(i)?;
    let i = skip(i, 1);
    let semitones = (f32::from(value) / 8.0 - 26.0).max(1.0).round() as i8;
    effect.bend = Some(BendEffect {
        points: vec![
            BendPoint {
                position: 0,
                value: 0,
            },
            BendPoint {
                position: 6,
                value: semitones,
            },
            BendPoint {
                position: 12,
                value: semitones,
            },
        ],
    });
    Ok((i, ()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_gp_data;

    /// Builds the bytes of a GP1/GP2 file.
    #[derive(Default)]
    struct Bytes(Vec<u8>);

    impl Bytes {
        fn version(mut self, version: &str) -> Self {
            self.0.push(version.len() as u8);
            let mut field = version.as_bytes().to_vec();
            field.resize(30, 0);
            self.0.extend(field);
            self
        }

        fn byte(mut self, value: u8) -> Self {
            self.0.push(value);
            self
        }

        fn bytes(mut self, values: &[u8]) -> Self {
            self.0.extend(values);
            self
        }

        fn int(mut self, value: i32) -> Self {
            self.0.extend(value.to_le_bytes());
            self
        }

        fn ints(self, values: &[i32]) -> Self {
            values.iter().fold(self, |bytes, &value| bytes.int(value))
        }

        /// A string preceded by its field size plus one and its length.
        fn sized_string(mut self, value: &str) -> Self {
            self.0.push(value.len() as u8 + 1);
            self.0.push(value.len() as u8);
            self.0.extend(value.as_bytes());
            self
        }

        /// The start of a measure: its time signature, then the beat count
        /// of each track among their unknown bytes, and no repeat.
        fn measure(self, numerator: u8, denominator: u8, beat_counts: &[u8]) -> Self {
            let bytes = self.bytes(&[numerator, denominator]).bytes(&[0; 6]);
            beat_counts
                .iter()
                .fold(bytes, |bytes, &count| {
                    bytes.bytes(&[0, 0, count]).bytes(&[0; 9])
                })
                .bytes(&[0, 0])
                .byte(0)
        }

        /// A beat's unknown Int and its duration byte.
        fn beat(self, duration: i8) -> Self {
            self.int(0).byte(duration as u8)
        }
    }

    /// A GP2 song of one measure: the first track with a chord and its tie,
    /// a rest changing the tempo, the second a four strings bass.
    fn gp2_song() -> Vec<u8> {
        let mut bytes = Bytes::default()
            .version("FICHIER GUITAR PRO v2.21")
            .sized_string("Song")
            .sized_string("Artist")
            .sized_string("Tabbed by me")
            .ints(&[120, 0, 0]);
        for track in 0..8 {
            bytes = if track == 1 {
                bytes.int(4).ints(&[43, 38, 33, 28])
            } else {
                bytes.int(6).ints(&[64, 59, 55, 50, 45, 40])
            };
        }
        bytes = bytes.int(1);
        for track in 0..8 {
            bytes = bytes
                .ints(&[25 + track, 24])
                .sized_string(&format!("Track {track}"))
                .byte(0)
                .ints(&[127, 64, 0, 0, 0]);
        }
        bytes
            .bytes(&[0; 10])
            .measure(4, 4, &[3, 1, 0, 0, 0, 0, 0, 0])
            // a quarter on the two lowest strings, the fifth one hammered,
            // the sixth dead, and a grace note on the fourth, not played
            .beat(0)
            .bytes(&[0, 0, 0b0000_0011, 0b0000_0010, 0b0000_0100])
            .bytes(&[1, 2, 3])
            .bytes(&[3, 6, 1])
            .bytes(&[100, 6])
            // a quarter tied to it
            .beat(0)
            .bytes(&[0x40, 0])
            // a quarter rest moving to 90 BPM
            .beat(0)
            .bytes(&[0x08, 0x02, 0x20])
            .int(90)
            .byte(0)
            // a half note on the bass: the highest bit has no string there
            .beat(-1)
            .bytes(&[0, 0, 0b0001_0001, 0, 0])
            .bytes(&[5, 8, 7, 8])
            .0
    }

    fn notes(beat: &Beat) -> Vec<(i8, i16, String, i16)> {
        beat.notes
            .iter()
            .map(|note| {
                (
                    note.string,
                    note.value,
                    format!("{:?}", note.kind),
                    note.velocity,
                )
            })
            .collect()
    }

    fn rhythm(beats: &[Beat]) -> Vec<(u32, u16, bool)> {
        beats
            .iter()
            .map(|beat| (beat.start, beat.duration.value, beat.notes.is_empty()))
            .collect()
    }

    #[test]
    fn only_gp1_and_gp2_versions_are_detected() {
        assert!(is_gp12_data(&gp2_song()));
        let gp1 = Bytes::default().version("FICHIER GUITARE PRO v1.04").0;
        assert!(is_gp12_data(&gp1));
        let gp3 = Bytes::default().version("FICHIER GUITAR PRO v3.00").0;
        assert!(!is_gp12_data(&gp3));
    }

    #[test]
    fn keys_past_seven_sharps_are_flats() {
        assert_eq!(key_signature(0), 0);
        assert_eq!(key_signature(7), 7);
        assert_eq!(key_signature(8), -1);
        assert_eq!(key_signature(14), -7);
        assert_eq!(key_signature(15), 7);
        assert_eq!(key_signature(-3), -3);
    }

    #[test]
    fn channel_settings_stay_midi_values() {
        assert_eq!(channel_setting(64), 64);
        assert_eq!(channel_setting(100), 100);
        assert_eq!(channel_setting(200), 127);
        assert_eq!(channel_setting(-1), 0);
    }

    #[test]
    fn a_gp2_song_reads_its_info_tracks_and_channels() {
        let song = parse_gp_data(&gp2_song()).unwrap();
        assert_eq!(song.version, GpVersion::GP2);
        assert_eq!(song.song_info.name, "Song");
        assert_eq!(song.song_info.artist, "Artist");
        assert_eq!(song.song_info.writer, "Tabbed by me");
        assert_eq!(song.tracks.len(), 8);
        assert_eq!(song.tracks[1].name, "Track 1");
        assert_eq!(
            song.tracks[1].strings,
            vec![(1, 43), (2, 38), (3, 33), (4, 28)]
        );
        assert_eq!(song.midi_channels[1].instrument, 26);
        // the channel settings are MIDI values, kept as they are
        assert_eq!(song.midi_channels[0].volume, 127);
        assert_eq!(song.midi_channels[0].balance, 64);
        // the last track plays the drums
        let drums = &song.midi_channels[7];
        assert_eq!((drums.channel_id, drums.instrument), (9, 0));
        assert!(drums.is_percussion());
        assert!(!song.midi_channels[0].is_percussion());
    }

    #[test]
    fn a_gp2_measure_reads_notes_ties_and_rests() {
        let song = parse_gp_data(&gp2_song()).unwrap();
        let beats = &song.tracks[0].measures[0].voices[0].beats;
        assert_eq!(
            notes(&beats[0]),
            vec![(5, 3, "Normal".into(), 95), (6, 0, "Dead".into(), 95)]
        );
        assert!(beats[0].notes[0].effect.hammer);
        assert!(!beats[0].notes[1].effect.hammer);
        assert_eq!(
            notes(&beats[1]),
            vec![(5, 3, "Tie".into(), 95), (6, 0, "Tie".into(), 95)]
        );
        // the rest, then the one filling the measure
        assert_eq!(
            rhythm(beats),
            vec![
                (960, 4, false),
                (1920, 4, false),
                (2880, 4, true),
                (3840, 4, true)
            ]
        );
        assert_eq!(song.measure_headers[0].tempo.value, 90);
    }

    #[test]
    fn gp2_chord_diagrams_consume_six_frets_even_on_a_four_string_track() {
        let mut song = Song::default();
        let reader = MeasureReader::new(Format::V2, &mut song);
        let bytes = Bytes::default()
            .bytes(&[2, b'D', b'm', 0])
            .int(0)
            .bytes(&[0; 32])
            .int(5)
            .ints(&[-1, 5, 7, 6, 8, 0])
            // A chord with first fret zero omits the diagram.
            .bytes(&[1, b'G', 0])
            .ints(&[12, 0])
            .0;
        let (rest, dm) = reader.parse_chord(&bytes, 4).unwrap();
        assert_eq!(dm.name, "Dm");
        assert_eq!(dm.first_fret, Some(5));
        assert_eq!(dm.strings, [-1, 5, 7, 6]);
        let (rest, g) = reader.parse_chord(rest, 4).unwrap();
        assert_eq!(g.name, "G");
        assert_eq!(g.first_fret, Some(0));
        assert_eq!(g.strings, [-1; 4]);
        assert!(rest.is_empty());
    }

    #[test]
    fn gp2_beat_effects_distinguish_bass_techniques_and_harmonics() {
        for (code, slap, harmonic) in [
            (5, SlapEffect::Tapping, None),
            (6, SlapEffect::Slapping, None),
            (7, SlapEffect::Popping, None),
            (8, SlapEffect::None, Some(HarmonicType::Natural)),
            (9, SlapEffect::None, Some(HarmonicType::Artificial)),
        ] {
            let mut effect = NoteEffect::default();
            let bytes = [code, 0xff];
            let (rest, ()) = parse_beat_effects(&bytes, &mut effect).unwrap();
            assert_eq!(effect.slap, slap);
            assert_eq!(effect.harmonic.map(|h| h.kind), harmonic);
            assert_eq!(rest, &[0xff]);
        }
    }

    #[test]
    fn a_note_on_a_missing_string_is_dropped() {
        let song = parse_gp_data(&gp2_song()).unwrap();
        let bass = &song.tracks[1].measures[0].voices[0].beats;
        assert_eq!(notes(&bass[0]), vec![(4, 7, "Normal".into(), 127)]);
        assert_eq!(rhythm(bass), vec![(960, 2, false), (2880, 2, true)]);
    }

    #[test]
    fn a_track_without_beats_rests() {
        let song = parse_gp_data(&gp2_song()).unwrap();
        let beats = &song.tracks[2].measures[0].voices[0].beats;
        assert_eq!(rhythm(beats), vec![(960, 1, true)]);
    }

    #[test]
    fn a_first_version_song_has_a_single_six_strings_track() {
        let bytes = Bytes::default()
            .version("FICHIER GUITARE PRO v1")
            .sized_string("Old")
            .sized_string("")
            .sized_string("")
            // tempo and triplet feel, but no key
            .ints(&[100, 1])
            .ints(&[64, 59, 55, 50, 45, 40])
            // one measure, then the instrument only
            .ints(&[1, 30])
            .measure(3, 4, &[1])
            // a bent quarter chord with a name and a text
            .beat(0)
            .byte(0x04 | 0x02 | 0x01)
            .byte(3)
            .bytes(&[0; 6])
            .byte(240)
            .byte(0)
            .sized_string("Em")
            .byte(4)
            .bytes(b"riff")
            .bytes(&[0b0010_0001, 0])
            .bytes(&[12, 0])
            .0;
        let song = parse_gp_data(&bytes).unwrap();
        assert_eq!(song.version, GpVersion::GP1);
        assert_eq!(song.tracks.len(), 1);
        assert_eq!(song.tracks[0].name, "Track 1");
        assert_eq!(song.measure_headers[0].triplet_feel, TripletFeel::Eighth);
        let beats = &song.tracks[0].measures[0].voices[0].beats;
        assert_eq!(beats[0].text, "riff");
        assert_eq!(beats[0].effect.chord.as_ref().unwrap().name, "Em");
        assert_eq!(
            notes(&beats[0]),
            vec![
                (1, 12, "Normal".into(), DEFAULT_VELOCITY),
                (6, 0, "Normal".into(), DEFAULT_VELOCITY)
            ]
        );
        // a bend of 240 / 8 - 26 = 4 semitones, on every note of the beat
        for note in &beats[0].notes {
            let bend = note.effect.bend.as_ref().unwrap();
            let values: Vec<_> = bend.points.iter().map(|p| (p.position, p.value)).collect();
            assert_eq!(values, vec![(0, 0), (6, 4), (12, 4)]);
        }
        assert_eq!(rhythm(beats), vec![(960, 4, false), (1920, 2, true)]);
    }

    #[test]
    fn a_zero_time_signature_is_rejected() {
        let bytes = Bytes::default()
            .version("FICHIER GUITARE PRO v1")
            .sized_string("")
            .sized_string("")
            .sized_string("")
            .ints(&[100, 0])
            .ints(&[64, 59, 55, 50, 45, 40])
            .ints(&[1, 30])
            .measure(0, 0, &[0])
            .0;
        assert!(parse_gp_data(&bytes).is_err());
    }

    #[test]
    fn rests_split_a_gap_into_equal_plain_durations() {
        let values = |start: u32, end: u32| -> Vec<(u16, bool)> {
            fill_with_rests(start, end)
                .iter()
                .map(|beat| (beat.duration.value, beat.duration.dotted))
                .collect()
        };
        // a whole measure of 4/4, 3/4 and 6/4
        assert_eq!(values(0, 3840), vec![(1, false)]);
        assert_eq!(values(0, 2880), vec![(4, false); 3]);
        assert_eq!(values(0, 5760), vec![(2, false); 3]);
        // a dotted quarter's gap
        assert_eq!(values(0, 1440), vec![(8, false); 3]);
        assert_eq!(values(100, 100), vec![]);
    }
}
