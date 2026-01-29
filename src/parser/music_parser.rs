use crate::parser::primitive_parser::{
    parse_byte_size_string, parse_i8, parse_int, parse_int_byte_sized_string, parse_u8, skip,
};
use crate::parser::song_parser::{
    convert_velocity, parse_beat_effects, parse_chord, parse_color, parse_duration,
    parse_measure_headers, parse_note_effects, Beat, GpVersion, Measure, Note, NoteEffect,
    NoteType, Song, Track, Voice, MAX_VOICES, QUARTER_TIME,
};
use nom::multi::count;
use nom::{IResult, Parser};

pub struct MusicParser {
    song: Song,
}

impl MusicParser {
    pub const fn new(song: Song) -> Self {
        Self { song }
    }
    pub fn take_song(&mut self) -> Song {
        std::mem::take(&mut self.song)
    }

    pub fn parse_music_data<'a>(&'a mut self, i: &'a [u8]) -> IResult<&'a [u8], ()> {
        let mut i = i;
        let song_version = self.song.version;

        if song_version >= GpVersion::GP5 {
            // skip directions & master reverb
            i = skip(i, 42);
        }

        let (i, (measure_count, track_count)) = (
            parse_int, // Measure count
            parse_int, // Track count
        )
            .parse(i)?;

        log::debug!(
            "Parsing music data -> track_count: {track_count} measure_count {measure_count}"
        );

        let song_tempo = self.song.tempo.value;
        let (i, measure_headers) =
            parse_measure_headers(measure_count, song_tempo, song_version)(i)?;
        self.song.measure_headers = measure_headers;

        let (i, tracks) = self.parse_tracks(track_count as usize)(i)?;
        self.song.tracks = tracks;

        let (i, _measures) = self.parse_measures(measure_count, track_count)(i)?;

        Ok((i, ()))
    }

    pub fn parse_tracks(
        &mut self,
        tracks_count: usize,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], Vec<Track>> + '_ {
        move |i| {
            log::debug!("Parsing {tracks_count} tracks");
            let mut i = i;
            let mut tracks = Vec::with_capacity(tracks_count);
            for index in 1..=tracks_count {
                let (inner, track) = self.parse_track(index)(i)?;
                i = inner;
                tracks.push(track);
            }
            // tracks done
            if self.song.version == GpVersion::GP5 {
                i = skip(i, 2);
            }

            if self.song.version > GpVersion::GP5 {
                i = skip(i, 1);
            }

            Ok((i, tracks))
        }
    }

    pub fn parse_track(
        &mut self,
        number: usize,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], Track> + '_ {
        move |i| {
            log::debug!("--------");
            log::debug!("Parsing track {number}");
            let mut i = skip(i, 1);
            let mut track = Track::default();

            if self.song.version >= GpVersion::GP5
                && (number == 1 || self.song.version == GpVersion::GP5)
            {
                i = skip(i, 1);
            };

            track.number = number as i32;

            // track name
            let (inner, name) = parse_byte_size_string(40)(i)?;
            i = inner;
            log::debug!("Track name:{name}");
            track.name = name;

            // string count
            let (inner, string_count) = parse_int(i)?;
            i = inner;
            log::debug!("String count: {string_count}");
            assert!(string_count > 0);

            // tunings
            let (inner, tunings) = count(parse_int, 7).parse(i)?;
            i = inner;
            log::debug!("Tunings: {tunings:?}");
            track.strings = tunings
                .iter()
                .enumerate()
                .filter(|(i, _)| (*i as i32) < string_count)
                .map(|(i, &t)| (i as i32 + 1, t))
                .collect();

            // midi port
            let (inner, port) = parse_int(i)?;
            log::debug!("Midi port: {port:?}");
            i = inner;
            track.midi_port = port as u8;

            // parse track channel info
            let (inner, channel_id) = self.parse_track_channel()(i)?;
            log::debug!("Midi channel id: {channel_id:?}");
            track.channel_id = channel_id as u8;
            i = inner;

            // fret
            let (inner, fret_count) = parse_int(i)?;
            log::debug!("Fret count: {fret_count:?}");
            i = inner;
            track.fret_count = fret_count as u8;

            // offset
            let (inner, offset) = parse_int(i)?;
            log::debug!("Offset: {offset:?}");
            i = inner;
            track.offset = offset;

            // color
            let (inner, color) = parse_color(i)?;
            log::debug!("Color: {color:?}");
            i = inner;
            track.color = color;

            if self.song.version == GpVersion::GP5 {
                // skip 44
                i = skip(i, 44);
            } else if self.song.version == GpVersion::GP5_10 {
                // skip 49
                i = skip(i, 49);
            };

            if self.song.version > GpVersion::GP5 {
                let (inner, _) = parse_int_byte_sized_string(i)?;
                i = inner;
                let (inner, _) = parse_int_byte_sized_string(i)?;
                i = inner;
            };
            Ok((i, track))
        }
    }

    /// Read MIDI channel. MIDI channel in Guitar Pro is represented by two integers.
    /// First is zero-based number of channel, second is zero-based number of channel used for effects.
    pub fn parse_track_channel(&mut self) -> impl FnMut(&[u8]) -> IResult<&[u8], i32> + '_ {
        log::debug!("Parsing track channel");
        |i| {
            let (i, (mut gm_channel_1, mut gm_channel_2)) = (parse_int, parse_int).parse(i)?;
            gm_channel_1 -= 1;
            gm_channel_2 -= 1;

            log::debug!("Track channel gm1: {gm_channel_1} gm2: {gm_channel_2}");

            if let Some(channel) = self.song.midi_channels.get_mut(gm_channel_1 as usize) {
                // if not percussion - set effect channel
                if channel.channel_id != 9 {
                    channel.effect_channel_id = gm_channel_2 as u8;
                }
            } else {
                log::debug!("channel {gm_channel_1} not found");
                debug_assert!(false, "channel {gm_channel_1} not found");
            }
            Ok((i, gm_channel_1))
        }
    }

    /// Read measures. Measures are written in the following order:
    /// - measure 1/track 1
    /// - measure 1/track 2
    /// - ...
    /// - measure 1/track m
    /// - measure 2/track 1
    /// - measure 2/track 2
    /// - ...
    /// - measure 2/track m
    /// - ...
    /// - measure n/track 1
    /// - measure n/track 2
    /// - ...
    /// - measure n/track m
    pub fn parse_measures(
        &mut self,
        measure_count: i32,
        track_count: i32,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], ()> + '_ {
        move |i: &[u8]| {
            log::debug!("--------");
            log::debug!("Parsing measures");
            let mut start = QUARTER_TIME;
            let mut i = i;
            for measure_index in 0..measure_count as usize {
                // set header start
                self.song.measure_headers[measure_index].start = start;
                for track_index in 0..track_count as usize {
                    let (inner, measure) =
                        self.parse_measure(start, measure_index, track_index)(i)?;
                    i = inner;
                    // push measure on track
                    self.song.tracks[track_index].measures.push(measure);
                    if self.song.version >= GpVersion::GP5 {
                        i = skip(i, 1);
                    }
                }
                // update start with measure length
                let measure_length = self.song.measure_headers[measure_index].length();
                assert!(measure_length > 0, "Measure length is 0");
                start += measure_length;
            }
            Ok((i, ()))
        }
    }

    pub fn parse_measure(
        &mut self,
        measure_start: u32,
        measure_index: usize,
        track_index: usize,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], Measure> + '_ {
        move |i: &[u8]| {
            log::debug!("--------");
            log::debug!("Parsing measure {measure_index} for track {track_index}");
            let mut i = i;
            let mut measure = Measure {
                header_index: measure_index,
                track_index,
                ..Default::default()
            };
            let voice_count = if self.song.version >= GpVersion::GP5 {
                MAX_VOICES
            } else {
                1
            };
            for voice_index in 0..voice_count {
                // voices have the same start value
                let beat_start = measure_start;
                log::debug!("--------");
                log::debug!("Parsing voice {voice_index}");
                let (inner, voice) = self.parse_voice(beat_start, track_index, measure_index)(i)?;
                i = inner;
                measure.voices.push(voice);
            }
            Ok((i, measure))
        }
    }

    pub fn parse_voice(
        &mut self,
        mut beat_start: u32,
        track_index: usize,
        measure_index: usize,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], Voice> + '_ {
        move |i: &[u8]| {
            let mut i = i;
            let (inner, beats) = parse_int(i)?;
            i = inner;
            let mut voice = Voice {
                measure_index: measure_index as i16,
                ..Default::default()
            };
            log::debug!("--------");
            log::debug!("...with {beats} beats");
            for b in 1..=beats {
                log::debug!("--------");
                log::debug!("Parsing beat {b}");
                let (inner, beat) = self.parse_beat(beat_start, track_index, measure_index)(i)?;
                if !beat.empty {
                    beat_start += beat.duration.time();
                }
                i = inner;
                voice.beats.push(beat);
            }
            Ok((i, voice))
        }
    }

    pub fn parse_beat(
        &mut self,
        start: u32,
        track_index: usize,
        measure_index: usize,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], Beat> + '_ {
        move |i: &[u8]| {
            let mut i = i;
            let (inner, flags) = parse_u8(i)?;
            i = inner;

            // make new beat at starting time
            let mut beat = Beat {
                start,
                ..Default::default()
            };

            // beat type
            if (flags & 0x40) != 0 {
                let (inner, beat_type) = parse_u8(i)?;
                i = inner;
                beat.empty = beat_type & 0x02 == 0;
            }

            // beat duration is an eighth note
            let (inner, duration) = parse_duration(flags)(i)?;
            beat.duration = duration;
            i = inner;

            // beat chords
            if (flags & 0x02) != 0 {
                let track = &self.song.tracks[track_index];
                let (inner, chord) = parse_chord(track.strings.len() as u8)(i)?;
                i = inner;
                beat.effect.chord = Some(chord);
            }

            // beat text
            if (flags & 0x04) != 0 {
                let (inner, text) = parse_int_byte_sized_string(i)?;
                i = inner;
                log::debug!("Beat text: {text}");
                beat.text = text;
            }

            let mut note_effect = NoteEffect::default();
            // beat effect
            if (flags & 0x08) != 0 {
                let (inner, ()) = parse_beat_effects(&mut beat, &mut note_effect)(i)?;
                i = inner;
            }

            // parse mix change
            if (flags & 0x10) != 0 {
                let (inner, ()) = self.parse_mix_change(measure_index)(i)?;
                i = inner;
            }

            // parse notes
            let (inner, string_flags) = parse_u8(i)?;
            i = inner;
            let track = &self.song.tracks[track_index];
            log::debug!("Parsing notes for beat strings:{}, flags:{string_flags:08b}", track.strings.len());
            assert!(!track.strings.is_empty());
            for (string_id, string_value) in track.strings.iter().enumerate() {
                if string_flags & (1 << (7 - string_value.0)) > 0 {
                    log::debug!("Parsing note for string {}", string_id + 1);
                    let mut note = Note::new(note_effect.clone());
                    let (inner, ()) = self.parse_note(&mut note, string_value, track_index)(i)?;
                    i = inner;
                    beat.notes.push(note);
                }
            }

            if self.song.version >= GpVersion::GP5 {
                i = skip(i, 1);
                let (inner, read) = parse_u8(i)?;
                i = inner;
                if (read & 0x08) != 0 {
                    i = skip(i, 1);
                }
            }
            Ok((i, beat))
        }
    }

    /// Get note value of tied note
    fn get_tied_note_value(&self, string_index: i8, track_index: usize) -> i16 {
        let track = &self.song.tracks[track_index];
        for m in (0usize..track.measures.len()).rev() {
            for v in (0usize..track.measures[m].voices.len()).rev() {
                for b in 0..track.measures[m].voices[v].beats.len() {
                    for n in 0..track.measures[m].voices[v].beats[b].notes.len() {
                        if track.measures[m].voices[v].beats[b].notes[n].string == string_index {
                            return track.measures[m].voices[v].beats[b].notes[n].value;
                        }
                    }
                }
            }
        }
        -1
    }

    pub fn parse_mix_change(
        &mut self,
        measure_index: usize,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], ()> + '_ {
        move |i: &[u8]| {
            log::debug!("Parsing mix change");
            let mut i = i;

            // instrument
            let (inner, _) = parse_i8(i)?;
            i = inner;

            if self.song.version >= GpVersion::GP5 {
                i = skip(i, 16);
            }

            let (inner, (volume, pan, chorus, reverb, phaser, tremolo)) =
                (parse_i8, parse_i8, parse_i8, parse_i8, parse_i8, parse_i8).parse(i)?;
            i = inner;

            let tempo_name = if self.song.version >= GpVersion::GP5 {
                let (inner, tempo_name_tmp) = parse_int_byte_sized_string(i)?;
                log::debug!("Tempo name: {tempo_name_tmp}");
                i = inner;
                tempo_name_tmp
            } else {
                String::new()
            };

            let (inner, tempo_value) = parse_int(i)?;
            i = inner;

            if volume >= 0 {
                i = skip(i, 1);
            }
            if pan >= 0 {
                i = skip(i, 1);
            }
            if chorus >= 0 {
                i = skip(i, 1);
            }
            if reverb >= 0 {
                i = skip(i, 1);
            }
            if phaser >= 0 {
                i = skip(i, 1);
            }
            if tremolo >= 0 {
                i = skip(i, 1);
            }

            if tempo_value >= 0 {
                // update tempo value for all next measure headers
                self.song.measure_headers[measure_index..]
                    .iter_mut()
                    .for_each(|mh| {
                        mh.tempo.value = tempo_value as u32;
                        mh.tempo.name = Some(tempo_name.clone());
                    });
                i = skip(i, 1);
                if self.song.version > GpVersion::GP5 {
                    i = skip(i, 1);
                }
            }

            i = skip(i, 1);

            if self.song.version >= GpVersion::GP5 {
                i = skip(i, 1);
                if self.song.version > GpVersion::GP5 {
                    let (inner, _) =
                        (parse_int_byte_sized_string, parse_int_byte_sized_string).parse(i)?;
                    i = inner;
                }
            }

            Ok((i, ()))
        }
    }

    pub fn parse_note<'a>(
        &'a self,
        note: &'a mut Note,
        guitar_string: &'a (i32, i32),
        track_index: usize,
    ) -> impl FnMut(&[u8]) -> IResult<&[u8], ()> + 'a {
        move |i| {
            log::debug!("Parsing note {guitar_string:?}");
            let mut i = i;
            let (inner, flags) = parse_u8(i)?;
            i = inner;
            let string = guitar_string.0 as i8;
            note.string = string;
            note.effect.heavy_accentuated_note = (flags & 0x02) == 0x02;
            note.effect.ghost_note = (flags & 0x04) == 0x04;
            note.effect.accentuated_note = (flags & 0x40) == 0x40;

            // note type
            if (flags & 0x20) != 0 {
                let (inner, note_type) = parse_u8(i)?;
                i = inner;
                note.kind = NoteType::get_note_type(note_type);
            }

            // duration percent GP4
            if (flags & 0x01) != 0 && self.song.version <= GpVersion::GP4_06 {
                i = skip(i, 2);
            }

            // note velocity
            if (flags & 0x10) != 0 {
                let (inner, velocity) = parse_i8(i)?;
                i = inner;
                note.velocity = convert_velocity(i16::from(velocity));
            }

            // note value
            if (flags & 0x20) != 0 {
                let (inner, fret) = parse_i8(i)?;
                i = inner;

                let value = if note.kind == NoteType::Tie {
                    self.get_tied_note_value(string, track_index)
                } else {
                    i16::from(fret)
                };
                // value is between 0 and 99
                if (0..100).contains(&value) {
                    note.value = value;
                } else {
                    note.value = 0;
                }
            }

            // fingering
            if (flags & 0x80) != 0 {
                i = skip(i, 2);
            }

            if self.song.version >= GpVersion::GP5 {
                // duration percent GP5
                if (flags & 0x01) != 0 {
                    i = skip(i, 8);
                }

                // swap accidentals
                let (inner, swap) = parse_u8(i)?;
                i = inner;
                note.swap_accidentals = swap & 0x02 == 0x02;
            }

            if (flags & 0x08) != 0 {
                let (inner, ()) = parse_note_effects(note, self.song.version)(i)?;
                i = inner;
            }

            Ok((i, ()))
        }
    }
}
