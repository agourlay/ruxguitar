use crate::audio::midi_event::MidiEvent;
use crate::audio::FIRST_TICK;
use crate::parser::song_parser::{
    Beat, BendEffect, HarmonicType, Measure, MeasureHeader, MidiChannel, Note, NoteType, Song,
    Track, TremoloBarEffect, TripletFeel, MIN_VELOCITY, QUARTER_TIME, SEMITONE_LENGTH,
    VELOCITY_INCREMENT,
};
use std::rc::Rc;

/// Thanks to `TuxGuitar` for the reference implementation in `MidiSequenceParser.java`

const DEFAULT_DURATION_DEAD: usize = 30;
const DEFAULT_DURATION_PM: usize = 60;
const DEFAULT_BEND: usize = 64;
const DEFAULT_BEND_SEMI_TONE: f32 = 2.75;

pub const NATURAL_FREQUENCIES: [(i32, i32); 6] = [
    (12, 12), //AH12 (+12 frets)
    (9, 28),  //AH9 (+28 frets)
    (5, 24),  //AH5 (+24 frets)
    (7, 19),  //AH7 (+19 frets)
    (4, 28),  //AH4 (+28 frets)
    (3, 31),  //AH3 (+31 frets)
];

pub struct MidiBuilder {
    events: Vec<MidiEvent>, // events accumulated during build
}

impl MidiBuilder {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Parse song and record events
    pub fn build_for_song(mut self, song: &Rc<Song>) -> Vec<MidiEvent> {
        for (track_id, track) in song.tracks.iter().enumerate() {
            log::debug!("building events for track {}", track_id);
            let midi_channel = song
                .midi_channels
                .iter()
                .find(|c| c.channel_id == track.channel_id)
                .unwrap_or_else(|| {
                    panic!(
                        "midi channel {} not found for track {}",
                        track.channel_id, track_id
                    )
                });
            self.add_track_events(
                song.tempo.value,
                track_id,
                track,
                &song.measure_headers,
                midi_channel,
            );
        }
        // Sort events by tick
        self.events.sort_by_key(|event| event.tick);
        self.events
    }

    fn add_track_events(
        &mut self,
        song_tempo: i32,
        track_id: usize,
        track: &Track,
        measure_headers: &[MeasureHeader],
        midi_channel: &MidiChannel,
    ) {
        // add MIDI control events for the track channel
        self.add_track_channel_midi_control(track_id, midi_channel);

        let strings = &track.strings;
        let mut prev_tempo = song_tempo;
        assert_eq!(track.measures.len(), measure_headers.len());
        for (measure, measure_header) in track.measures.iter().zip(measure_headers) {
            // add song info events once for all tracks
            if track_id == 0 {
                // change tempo if necessary
                let measure_tempo = measure_header.tempo.value;
                if measure_tempo != prev_tempo {
                    let tick = measure_header.start as usize;
                    self.add_tempo_change(tick, measure_tempo);
                    prev_tempo = measure_tempo;
                }
            }
            self.add_beat_events(
                track_id,
                track,
                measure,
                measure_header,
                midi_channel,
                strings,
            )
        }
    }

    fn add_beat_events(
        &mut self,
        track_id: usize,
        track: &Track,
        measure: &Measure,
        measure_header: &MeasureHeader,
        midi_channel: &MidiChannel,
        strings: &[(i32, i32)],
    ) {
        for voice in &measure.voices {
            let beats = &voice.beats;
            for (beat_id, beat) in beats.iter().enumerate() {
                if beat.empty || beat.notes.is_empty() {
                    continue;
                }
                // extract surrounding beats
                let previous_beat = if beat_id == 0 {
                    None
                } else {
                    beats.get(beat_id - 1)
                };
                let next_beat = beats.get(beat_id + 1);
                self.add_notes(
                    track_id,
                    track,
                    measure_header,
                    midi_channel,
                    previous_beat,
                    beat,
                    next_beat,
                    strings,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_notes(
        &mut self,
        track_id: usize,
        track: &Track,
        measure_header: &MeasureHeader,
        midi_channel: &MidiChannel,
        previous_beat: Option<&Beat>,
        beat: &Beat,
        next_beat: Option<&Beat>,
        strings: &[(i32, i32)],
    ) {
        if measure_header.triplet_feel != TripletFeel::None {
            log::warn!("Triplet feel not supported");
        }
        let _stroke = &beat.effect.stroke;
        let mut start = beat.start as usize;
        let channel_id = midi_channel.channel_id;
        let tempo = measure_header.tempo.value;
        // TODO when to use effect channel instead?
        assert!(channel_id < 16);
        let track_offset = track.offset;
        let beat_duration = beat.duration.time() as usize;
        for (note_offset, note) in beat.notes.iter().enumerate() {
            if note.kind != NoteType::Tie {
                let (string_id, string_tuning) = strings[note.string as usize - 1];
                assert_eq!(string_id, note.string as i32);

                // compute key without effect
                let initial_key = track_offset + note.value as i32 + string_tuning;

                // surrounding notes on the same string on the previous & next beat
                let previous_note = previous_beat.and_then(|b| b.notes.get(note_offset));
                let next_note = next_beat.and_then(|b| b.notes.get(note_offset));

                // apply effects on duration
                let mut duration = apply_duration_effect(note, next_note, tempo, beat_duration);
                assert_ne!(duration, 0);

                // apply effects on velocity
                let velocity = apply_velocity_effect(note, previous_note, midi_channel);

                // apply effects on key
                if let Some(key) = self.add_key_effect(
                    track_id,
                    &mut start,
                    &mut duration,
                    tempo,
                    note,
                    next_note,
                    next_beat,
                    initial_key,
                    velocity,
                    midi_channel,
                ) {
                    self.add_note(track_id, key, start, duration, velocity, channel_id as i32)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_key_effect(
        &mut self,
        track_id: usize,
        start: &mut usize,
        duration: &mut usize,
        tempo: i32,
        note: &Note,
        next_note: Option<&Note>,
        next_beat: Option<&Beat>,
        initial_key: i32,
        velocity: i16,
        midi_channel: &MidiChannel,
    ) -> Option<i32> {
        let channel_id = midi_channel.channel_id;
        let is_percussion = midi_channel.is_percussion();

        // key with effect
        let mut key = initial_key;

        if note.effect.fade_in {
            // TODO fade_in
        }

        // grace note
        if let Some(grace) = &note.effect.grace {
            let grace_key = initial_key + grace.fret as i32;
            let grace_length = grace.duration_time() as usize;
            let grace_velocity = grace.velocity;
            let grace_duration = if grace.is_dead {
                apply_static_duration(tempo, DEFAULT_DURATION_DEAD, grace_length)
            } else {
                grace_length
            };
            let on_beat_duration = *start - grace_length;
            if grace.is_on_beat || on_beat_duration < QUARTER_TIME as usize {
                *start += grace_length;
                *duration -= grace_length;
            }
            self.add_note(
                track_id,
                grace_key,
                *start - grace_length,
                grace_duration,
                grace_velocity,
                channel_id as i32,
            )
        }

        if let Some(trill) = &note.effect.trill {
            if !is_percussion {
                let trill_key = trill.fret as i32 + initial_key - note.value as i32;
                let mut trill_length = trill.duration.time() as usize;

                let trill_tick_limit = *start + *duration;
                let mut real_key = false;
                let mut tick = *start;

                let mut counter = 0;
                while tick + 10 < trill_tick_limit {
                    if tick + trill_length >= trill_tick_limit {
                        trill_length = trill_tick_limit - tick - 1;
                    }
                    let iter_key = if real_key { initial_key } else { trill_key };
                    self.add_note(
                        track_id,
                        iter_key,
                        tick,
                        trill_length,
                        velocity,
                        channel_id as i32,
                    );
                    real_key = !real_key;
                    tick += trill_length;
                    counter += 1;
                }
                assert!(
                    counter > 0,
                    "No trill notes published! trill_length: {}, tick: {}, trill_tick_limit: {}",
                    trill_length,
                    tick,
                    trill_tick_limit
                );

                // all notes published - the caller does not need to publish the note
                return None;
            }
        }

        // tremolo picking
        if let Some(tremolo_picking) = &note.effect.tremolo_picking {
            let mut tp_length = tremolo_picking.duration.time() as usize;
            let mut tick = *start;
            let tp_tick_limit = *start + *duration;
            let mut counter = 0;
            while tick + 10 < tp_tick_limit {
                if tick + tp_length >= tp_tick_limit {
                    tp_length = tp_tick_limit - tick - 1;
                }
                self.add_note(
                    track_id,
                    initial_key,
                    tick,
                    tp_length,
                    velocity,
                    channel_id as i32,
                );
                tick += tp_length;
                counter += 1;
            }
            assert!(
                counter > 0,
                "No tremolo notes published! tp_length: {}, tick: {}, tp_tick_limit: {}",
                tp_length,
                tick,
                tp_tick_limit
            );
            // all notes published - the caller does not need to publish the note
            return None;
        }

        // bend
        if let Some(bend_effect) = &note.effect.bend {
            if !is_percussion {
                self.add_bend(
                    track_id,
                    *start,
                    *duration,
                    channel_id as usize,
                    bend_effect,
                )
            }
        }

        if let Some(tremolo_bar) = &note.effect.tremolo_bar {
            if !is_percussion {
                self.add_tremolo_bar(
                    track_id,
                    *start,
                    *duration,
                    channel_id as usize,
                    tremolo_bar,
                )
            }
        }

        // slide
        if let Some(_slide) = &note.effect.slide {
            if !is_percussion {
                if let Some((next_note, next_beat)) = next_note.zip(next_beat) {
                    let value_1 = note.value as i32;
                    let value_2 = next_note.value as i32;

                    let tick1 = *start;
                    let tick2 = next_beat.start as usize;

                    // make slide
                    let distance: i32 = value_2 - value_1;
                    let length: i32 = (tick2 - tick1) as i32;
                    let points = length / (QUARTER_TIME as usize / 8) as i32;
                    for p_offset in 1..=points {
                        let tone = ((length / points) * p_offset) * distance / length;
                        let bend =
                            DEFAULT_BEND as f32 + (tone as f32 * DEFAULT_BEND_SEMI_TONE * 2.0);
                        let bend_tick = tick1 as i32 + (length / points) * p_offset;
                        self.add_pitch_bend(
                            bend_tick as usize,
                            track_id,
                            channel_id as i32,
                            bend as i32,
                        );
                    }

                    // normalise the bend
                    self.add_pitch_bend(tick2, track_id, channel_id as i32, DEFAULT_BEND as i32);
                }
            }
        }

        // vibrato
        if note.effect.vibrato && !is_percussion {
            self.add_vibrato(track_id, *start, *duration, channel_id as usize);
        }

        // harmonic
        if let Some(harmonic) = &note.effect.harmonic {
            if !is_percussion {
                match harmonic.kind {
                    HarmonicType::Natural => {
                        for (harmonic_value, harmonic_frequency) in NATURAL_FREQUENCIES {
                            if note.value % 12 == (harmonic_value % 12) as i16 {
                                key = (initial_key + harmonic_frequency) - note.value as i32;
                                break;
                            }
                        }
                    }
                    HarmonicType::Semi => {
                        let velocity = MIN_VELOCITY.max(velocity - VELOCITY_INCREMENT * 3);
                        self.add_note(
                            track_id,
                            initial_key,
                            *start,
                            *duration,
                            velocity,
                            channel_id as i32,
                        );
                        key = initial_key + NATURAL_FREQUENCIES[0].1;
                    }
                    HarmonicType::Artificial | HarmonicType::Pinch => {
                        key = initial_key + NATURAL_FREQUENCIES[0].1;
                    }
                    HarmonicType::Tapped => {
                        if let Some(right_hand_fret) = harmonic.right_hand_fret {
                            for (harmonic_value, harmonic_frequency) in NATURAL_FREQUENCIES {
                                if right_hand_fret as i16 - note.value == harmonic_value as i16 {
                                    key = initial_key + harmonic_frequency;
                                    break;
                                }
                            }
                        }
                    }
                }
                if key - 12 > 0 {
                    let velocity = MIN_VELOCITY.max(velocity - VELOCITY_INCREMENT * 4);
                    self.add_note(
                        track_id,
                        key - 12,
                        *start,
                        *duration,
                        velocity,
                        channel_id as i32,
                    );
                }
            }
        }

        Some(key)
    }

    fn add_vibrato(&mut self, track_id: usize, start: usize, duration: usize, channel_id: usize) {
        let end = start + duration;
        let mut next_start = start;
        while next_start < end {
            next_start = if next_start + 160 > end {
                end
            } else {
                next_start + 160
            };
            self.add_pitch_bend(next_start, track_id, channel_id as i32, DEFAULT_BEND as i32);

            next_start = if next_start + 160 > end {
                end
            } else {
                next_start + 160
            };
            let value = DEFAULT_BEND as f32 + DEFAULT_BEND_SEMI_TONE / 2.0;
            self.add_pitch_bend(next_start, track_id, channel_id as i32, value as i32);
        }
        self.add_pitch_bend(next_start, track_id, channel_id as i32, DEFAULT_BEND as i32)
    }

    // TODO investigate why it does not sound good :(
    fn add_bend(
        &mut self,
        track_id: usize,
        start: usize,
        duration: usize,
        channel_id: usize,
        bend: &BendEffect,
    ) {
        for (point_id, point) in bend.points.iter().enumerate() {
            let value = DEFAULT_BEND as f32
                + (point.value as f32 * DEFAULT_BEND_SEMI_TONE / SEMITONE_LENGTH);
            let mut value = value.clamp(0.0, 127.0);
            let mut bend_start = start + point.get_time(duration);
            self.add_pitch_bend(bend_start, track_id, channel_id as i32, value as i32);

            // look ahead to next point
            if let Some(next_point) = bend.points.get(point_id + 1) {
                let next_value = DEFAULT_BEND as f32
                    + (next_point.value as f32 * DEFAULT_BEND_SEMI_TONE / SEMITONE_LENGTH);
                let next_bend_start = start + next_point.get_time(duration);
                if value != next_value {
                    let width = (next_bend_start - bend_start) as f32 / (next_value - value).abs();
                    // ascending
                    if value < next_value {
                        while value < next_value {
                            value += 1.0;
                            bend_start += width as usize;
                            self.add_pitch_bend(
                                bend_start,
                                track_id,
                                channel_id as i32,
                                value as i32,
                            );
                        }
                    }
                    // descending
                    if value > next_value {
                        while value > next_value {
                            value -= 1.0;
                            bend_start += width as usize;
                            self.add_pitch_bend(
                                bend_start,
                                track_id,
                                channel_id as i32,
                                value as i32,
                            );
                        }
                    }
                }
            }
        }
        self.add_pitch_bend(
            start + duration,
            track_id,
            channel_id as i32,
            DEFAULT_BEND as i32,
        )
    }

    fn add_tremolo_bar(
        &mut self,
        track_id: usize,
        start: usize,
        duration: usize,
        channel_id: usize,
        tremolo_bar: &TremoloBarEffect,
    ) {
        for (point_id, point) in tremolo_bar.points.iter().enumerate() {
            let value = DEFAULT_BEND as f32 + (point.value as f32 * DEFAULT_BEND_SEMI_TONE * 2.0);
            let mut value = value.clamp(0.0, 127.0);
            let mut bend_start = start + point.get_time(duration);
            self.add_pitch_bend(bend_start, track_id, channel_id as i32, value as i32);

            // look ahead to next point
            if let Some(next_point) = tremolo_bar.points.get(point_id + 1) {
                let next_value =
                    DEFAULT_BEND as f32 + (next_point.value as f32 * DEFAULT_BEND_SEMI_TONE * 2.0);
                let next_bend_start = start + next_point.get_time(duration);
                if value != next_value {
                    let width = (next_bend_start - bend_start) as f32 / (next_value - value).abs();
                    // ascending
                    if value < next_value {
                        while value < next_value {
                            value += 1.0;
                            bend_start += width as usize;
                            self.add_pitch_bend(
                                bend_start,
                                track_id,
                                channel_id as i32,
                                value as i32,
                            );
                        }
                    }
                    // descending
                    if value > next_value {
                        while value > next_value {
                            value -= 1.0;
                            bend_start += width as usize;
                            self.add_pitch_bend(
                                bend_start,
                                track_id,
                                channel_id as i32,
                                value as i32,
                            );
                        }
                    }
                }
            }
        }
        self.add_pitch_bend(
            start + duration,
            track_id,
            channel_id as i32,
            DEFAULT_BEND as i32,
        )
    }

    fn add_note(
        &mut self,
        track_id: usize,
        key: i32,
        start: usize,
        duration: usize,
        velocity: i16,
        channel: i32,
    ) {
        let note_on = MidiEvent::new_note_on(start, track_id, key, velocity, channel);
        self.add_event(note_on);
        if duration > 0 {
            let tick = start + duration;
            let note_off = MidiEvent::new_note_off(tick, track_id, key, channel);
            self.add_event(note_off);
        }
    }

    fn add_tempo_change(&mut self, tick: usize, tempo: i32) {
        let event = MidiEvent::new_tempo_change(tick, tempo);
        self.add_event(event);
    }

    fn add_bank_selection(&mut self, tick: usize, track_id: usize, channel: i32, bank: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x00, bank);
        self.add_event(event);
    }

    fn add_volume_selection(&mut self, tick: usize, track_id: usize, channel: i32, volume: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x27, volume);
        self.add_event(event);
    }

    fn add_expression_selection(
        &mut self,
        tick: usize,
        track_id: usize,
        channel: i32,
        expression: i32,
    ) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x2B, expression);
        self.add_event(event);
    }

    fn add_chorus_selection(&mut self, tick: usize, track_id: usize, channel: i32, chorus: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x5D, chorus);
        self.add_event(event);
    }

    fn add_reverb_selection(&mut self, tick: usize, track_id: usize, channel: i32, reverb: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x5B, reverb);
        self.add_event(event);
    }

    fn add_pitch_bend(&mut self, tick: usize, track_id: usize, channel: i32, value: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xE0, 0, value);
        self.add_event(event);
    }

    fn add_program_selection(&mut self, tick: usize, track_id: usize, channel: i32, program: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xC0, program, 0);
        self.add_event(event);
    }

    fn add_track_channel_midi_control(&mut self, track_id: usize, midi_channel: &MidiChannel) {
        let channel_id = midi_channel.channel_id;
        // publish MIDI control messages for the track channel at the start
        let info_tick = FIRST_TICK;
        self.add_volume_selection(
            info_tick,
            track_id,
            channel_id as i32,
            midi_channel.volume as i32,
        );
        self.add_expression_selection(info_tick, track_id, channel_id as i32, 127);
        self.add_chorus_selection(
            info_tick,
            track_id,
            channel_id as i32,
            midi_channel.chorus as i32,
        );
        self.add_reverb_selection(
            info_tick,
            track_id,
            channel_id as i32,
            midi_channel.reverb as i32,
        );
        self.add_bank_selection(
            info_tick,
            track_id,
            channel_id as i32,
            midi_channel.bank as i32,
        );
        self.add_program_selection(
            info_tick,
            track_id,
            channel_id as i32,
            midi_channel.instrument,
        );
    }

    fn add_event(&mut self, event: MidiEvent) {
        self.events.push(event);
    }
}

fn apply_velocity_effect(
    note: &Note,
    previous_note: Option<&Note>,
    midi_channel: &MidiChannel,
) -> i16 {
    let effect = &note.effect;
    let mut velocity = note.velocity;

    if !midi_channel.is_percussion() && previous_note.is_some_and(|n| n.effect.hammer) {
        velocity = MIN_VELOCITY.max(velocity - 25);
    }

    if effect.ghost_note {
        velocity = MIN_VELOCITY.max(velocity - VELOCITY_INCREMENT);
    } else if effect.accentuated_note {
        velocity = MIN_VELOCITY.max(velocity + VELOCITY_INCREMENT);
    } else if effect.heavy_accentuated_note {
        velocity = MIN_VELOCITY.max(velocity + VELOCITY_INCREMENT * 2);
    }
    if velocity > 127 {
        127
    } else {
        velocity
    }
}

fn apply_duration_effect(
    note: &Note,
    next_note: Option<&Note>,
    tempo: i32,
    mut duration: usize,
) -> usize {
    let note_type = &note.kind;
    if let Some(next_note) = next_note {
        if next_note.kind == NoteType::Tie {
            // approximation?
            duration += duration;
        }
    }
    if note_type == &NoteType::Dead {
        return apply_static_duration(tempo, DEFAULT_DURATION_DEAD, duration);
    }
    if note.effect.palm_mute {
        return apply_static_duration(tempo, DEFAULT_DURATION_PM, duration);
    }
    if note.effect.staccato {
        return ((duration * 50) as f64 / 100.00) as usize;
    }
    if note.effect.let_ring {
        return duration * 2;
    }
    duration
}

fn apply_static_duration(tempo: i32, duration: usize, maximum: usize) -> usize {
    let value = tempo as usize * duration / 60;
    if value < maximum {
        value
    } else {
        maximum
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::midi_event::MidiEventType;
    use crate::parser::song_parser_tests::parse_gp_file;
    use std::collections::HashSet;

    #[test]
    fn test_midi_events_for_all_gp5_song() {
        let test_dir = std::path::Path::new("test-files");
        for entry in std::fs::read_dir(test_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().unwrap() != "gp5" {
                continue;
            }
            let file_name = path.file_name().unwrap().to_str().unwrap();
            eprintln!("Parsing file: {}", file_name);
            let file_path = path.to_str().unwrap();
            let song = parse_gp_file(file_path)
                .unwrap_or_else(|err| panic!("Failed to parse file: {}\n{}", file_name, err));
            let song = Rc::new(song);
            let builder = MidiBuilder::new();
            let events = builder.build_for_song(&song);
            assert!(!events.is_empty(), "No events found for {}", file_name);

            // assert sorted by tick
            assert!(events.windows(2).all(|w| w[0].tick <= w[1].tick));
            assert_eq!(events[0].tick, 1);
        }
    }

    #[test]
    fn test_midi_events_for_demo_song() {
        const FILE_PATH: &str = "test-files/Demo v5.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);

        assert_eq!(events.len(), 4450);
        assert_eq!(events[0].tick, 1);
        assert_eq!(events.iter().last().unwrap().tick, 189_120);

        // assert number of tracks
        let track_count = song.tracks.len();
        let unique_tracks: HashSet<_> = events.iter().map(|event| event.track).collect();
        assert_eq!(unique_tracks.len(), track_count + 1); // plus None for info events

        // skip MIDI program messages
        let rhythm_track_events: Vec<_> = events
            .iter()
            .filter(|e| e.track == Some(0))
            .skip(6)
            .collect();

        // print 20 first for debugging
        for (i, event) in rhythm_track_events.iter().enumerate().take(20) {
            eprintln!("{} {:?}", i, event);
        }

        // C5 ON
        let event = &rhythm_track_events[0];
        assert_eq!(event.tick, 960);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 60, 95)));

        let event = &rhythm_track_events[1];
        assert_eq!(event.tick, 960);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 55, 95)));

        let event = &rhythm_track_events[2];
        assert_eq!(event.tick, 960);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 127)));

        // C5 OFF
        let event = &rhythm_track_events[3];
        assert_eq!(event.tick, 1440);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 60)));

        let event = &rhythm_track_events[4];
        assert_eq!(event.tick, 1440);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 55)));

        let event = &rhythm_track_events[5];
        assert_eq!(event.tick, 1440);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 48)));

        // single note `3` on string `1` (E2)
        let event = &rhythm_track_events[6];
        assert_eq!(event.tick, 1440);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 95)));

        // single note OFF (palm mute)
        let event = &rhythm_track_events[7];
        assert_eq!(event.tick, 1605);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 48)));

        // single note `3` on string `1` (E2)
        let event = &rhythm_track_events[8];
        assert_eq!(event.tick, 1920);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 95)));

        // single note OFF (palm mute)
        let event = &rhythm_track_events[9];
        assert_eq!(event.tick, 2085);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 48)));

        // C5 ON
        let event = &rhythm_track_events[10];
        assert_eq!(event.tick, 2400);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 60, 95)));

        let event = &rhythm_track_events[11];
        assert_eq!(event.tick, 2400);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 55, 95)));

        let event = &rhythm_track_events[12];
        assert_eq!(event.tick, 2400);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 127)));

        // skip MIDI program messages
        let solo_track_events: Vec<_> = events
            .iter()
            .filter(|e| e.track == Some(1))
            .skip(6)
            .collect();

        // print 20 first for debugging
        for (i, event) in solo_track_events.iter().enumerate().take(60) {
            eprintln!("{} {:?}", i, event);
        }

        // trill ON
        let event = &solo_track_events[0];
        assert_eq!(event.tick, 12480);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 72, 95)));

        // trill OFF
        let event = &solo_track_events[1];
        assert_eq!(event.tick, 12720);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 72)));

        // trill ON
        let event = &solo_track_events[2];
        assert_eq!(event.tick, 12720);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 69, 95)));

        // trill OFF
        let event = &solo_track_events[3];
        assert_eq!(event.tick, 12960);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 69)));

        // trill ON
        let event = &solo_track_events[4];
        assert_eq!(event.tick, 12960);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 72, 95)));

        // trill OFF
        let event = &solo_track_events[5];
        assert_eq!(event.tick, 13200);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 72)));

        // pass trill notes...

        // tremolo ON
        let event = &solo_track_events[32];
        assert_eq!(event.tick, 16320);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 60, 95)));

        // tremolo OFF
        let event = &solo_track_events[33];
        assert_eq!(event.tick, 16440);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 60)));

        // tremolo ON
        let event = &solo_track_events[34];
        assert_eq!(event.tick, 16440);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 60, 95)));

        // tremolo OFF
        let event = &solo_track_events[35];
        assert_eq!(event.tick, 16560);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 60)));

        // tremolo ON
        let event = &solo_track_events[36];
        assert_eq!(event.tick, 16560);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 60, 95)));

        // tremolo OFF
        let event = &solo_track_events[37];
        assert_eq!(event.tick, 16680);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 60)));

        // etc...
    }
}