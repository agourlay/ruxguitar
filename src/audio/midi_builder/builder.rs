/// Thanks to `TuxGuitar` for the reference implementation in `MidiSequenceParser.java`
use crate::audio::midi_event::{FIRST_TICK, MidiEvent};
use crate::parser::song_parser::{
    Beat, BendEffect, BendPoint, HarmonicType, MIN_VELOCITY, Measure, MeasureHeader, MidiChannel,
    Note, NoteType, QUARTER_TIME, SEMITONE_LENGTH, Song, Track, TremoloBarEffect,
    VELOCITY_INCREMENT,
};
use std::rc::Rc;

#[cfg(test)]
use crate::audio::playback_order::compute_playback_order;

use super::effects::{
    DEFAULT_DURATION_DEAD, TripletAdjustment, apply_duration_effect, apply_static_duration,
    apply_triplet_feel, apply_velocity_effect, compute_stroke_offsets,
};

const DEFAULT_BEND: f32 = 64.0;
const DEFAULT_BEND_SEMI_TONE: f32 = 2.75;

/// Scale a raw Guitar Pro channel byte (0-16) to a MIDI value (0-127),
/// matching TuxGuitar's `toChannelShort`. Used for channel volume, pan,
/// chorus and reverb (raw 16 -> 127, raw 8 -> 63, raw 0 -> 0).
fn to_channel_short(value: i8) -> i32 {
    (i32::from(value) * 8 - 1).clamp(0, 127)
}

const NATURAL_FREQUENCIES: [(i32, i32); 6] = [
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
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Parse song and record events, computing playback order internally.
    #[cfg(test)]
    pub fn build_for_song(self, song: &Rc<Song>) -> Vec<MidiEvent> {
        let playback_order = compute_playback_order(&song.measure_headers);
        self.build_for_song_with_order(song, &playback_order)
    }

    /// Parse song and record events using a pre-computed playback order.
    pub fn build_for_song_with_order(
        mut self,
        song: &Rc<Song>,
        playback_order: &[(usize, i64)],
    ) -> Vec<MidiEvent> {
        for (track_id, track) in song.tracks.iter().enumerate() {
            log::debug!("building events for track {track_id}");
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
                playback_order,
                midi_channel,
            );
        }
        // Sort events by tick
        self.events.sort_by_key(|event| event.tick);
        self.events
    }

    fn add_track_events(
        &mut self,
        song_tempo: u32,
        track_id: usize,
        track: &Track,
        measure_headers: &[MeasureHeader],
        playback_order: &[(usize, i64)],
        midi_channel: &MidiChannel,
    ) {
        // add MIDI control events for the track channel
        self.add_track_channel_midi_control(track_id, midi_channel);

        let strings = &track.strings;
        let mut prev_tempo = song_tempo;
        assert_eq!(track.measures.len(), measure_headers.len());
        for (measure_index, tick_offset) in playback_order {
            let measure = &track.measures[*measure_index];
            let measure_header = &measure_headers[*measure_index];

            // add song info events once for all tracks
            if track_id == 0 {
                // change tempo if necessary
                let measure_tempo = measure_header.tempo.value;
                if measure_tempo != prev_tempo {
                    let tick = (i64::from(measure_header.start) + tick_offset) as u32;
                    self.add_tempo_change(tick, measure_tempo);
                    prev_tempo = measure_tempo;
                }
            }

            // record event count to shift new events by tick_offset
            let event_start = self.events.len();
            self.add_beat_events(
                track_id,
                track,
                measure,
                measure_header,
                midi_channel,
                strings,
            );
            // shift events generated for this measure by tick_offset
            if *tick_offset != 0 {
                for event in &mut self.events[event_start..] {
                    event.tick = (i64::from(event.tick) + tick_offset) as u32;
                }
            }
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
        let measure_id = measure.voices[0].measure_index as usize;
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
                let next_beat = beats.get(beat_id + 1).or_else(|| {
                    // check next measure if it was the last beat
                    track
                        .measures
                        .get(voice.measure_index as usize + 1)
                        .and_then(|next_measure| next_measure.voices[0].beats.first())
                });
                // apply triplet feel adjustment to beat timing
                let triplet_adj =
                    apply_triplet_feel(beat, previous_beat, next_beat, measure_header.triplet_feel);
                self.add_notes(
                    track_id,
                    track,
                    measure_id,
                    measure_header,
                    midi_channel,
                    previous_beat,
                    beat_id,
                    beat,
                    next_beat,
                    strings,
                    triplet_adj,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_notes(
        &mut self,
        track_id: usize,
        track: &Track,
        measure_id: usize,
        measure_header: &MeasureHeader,
        midi_channel: &MidiChannel,
        previous_beat: Option<&Beat>,
        beat_id: usize,
        beat: &Beat,
        next_beat: Option<&Beat>,
        strings: &[(i32, i32)],
        triplet_adj: TripletAdjustment,
    ) {
        let channel_id = midi_channel.channel_id;
        let tempo = measure_header.tempo.value;
        // GP files define an effect channel per track, but TuxGuitar doesn't use it for playback.
        assert!(channel_id < 16);
        let track_offset = track.offset;
        let beat_duration = triplet_adj.duration;
        let stroke = &beat.effect.stroke;
        let stroke_increment = stroke.increment_for_duration(beat_duration);
        // pre-compute per-string stroke offsets (only for strings with non-tied notes)
        let stroke_offsets = compute_stroke_offsets(beat, stroke_increment, strings.len());
        for note in &beat.notes {
            if note.kind != NoteType::Tie {
                let (string_id, string_tuning) = strings[note.string as usize - 1];
                assert_eq!(string_id, i32::from(note.string));

                // note starts on beat (adjusted for triplet feel)
                let mut note_start = triplet_adj.start;

                // apply effects on duration
                let mut duration = apply_duration_effect(
                    track,
                    measure_id,
                    beat_id,
                    note,
                    next_beat,
                    tempo,
                    beat_duration,
                );
                assert_ne!(duration, 0);

                // apply stroke effect: stagger note start times across strings
                let stroke_offset = stroke_offsets[note.string as usize - 1];
                if stroke_offset > 0 {
                    note_start += stroke_offset;
                    // like TuxGuitar, keep the full duration when the offset would
                    // consume it: a zero duration emits a NoteOn without NoteOff
                    if duration > stroke_offset {
                        duration -= stroke_offset;
                    }
                }

                // surrounding notes on the same string on the previous & next beat
                let previous_note =
                    previous_beat.and_then(|b| b.notes.iter().find(|n| n.string == note.string));
                let next_note =
                    next_beat.and_then(|b| b.notes.iter().find(|n| n.string == note.string));

                // pack with beat to propagate duration
                let next_note = next_beat.zip(next_note);

                // apply effects on velocity
                let velocity = apply_velocity_effect(note, previous_note, midi_channel);

                // apply effects on key
                if let Some(key) = self.add_key_effect(
                    track_id,
                    track_offset,
                    string_tuning,
                    &mut note_start,
                    &mut duration,
                    tempo,
                    note,
                    next_note,
                    velocity,
                    midi_channel,
                ) {
                    self.add_note(
                        track_id,
                        key,
                        note_start,
                        duration,
                        velocity,
                        i32::from(channel_id),
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_key_effect(
        &mut self,
        track_id: usize,
        track_offset: i32,
        string_tuning: i32,
        note_start: &mut u32,
        duration: &mut u32,
        tempo: u32,
        note: &Note,
        next_note_beat: Option<(&Beat, &Note)>,
        velocity: i16,
        midi_channel: &MidiChannel,
    ) -> Option<i32> {
        let channel_id = i32::from(midi_channel.channel_id);
        let is_percussion = midi_channel.is_percussion();

        // compute key without effect
        let initial_key = track_offset + i32::from(note.value) + string_tuning;

        // key with effect
        let mut key = initial_key;

        // fade in
        if note.effect.fade_in {
            let mut expression = 31;
            let expression_increment = 1;
            let mut tick = *note_start;
            let tick_increment = *duration / ((127 - expression) / expression_increment);
            while tick < (*note_start + *duration) && expression < 127 {
                self.add_expression(tick, track_id, channel_id, expression as i32);
                tick += tick_increment;
                expression += expression_increment;
            }
            // normalize the expression
            self.add_expression(*note_start + *duration, track_id, channel_id, 127);
        }

        // grace note
        if let Some(grace) = &note.effect.grace {
            let grace_key = track_offset + i32::from(grace.fret) + string_tuning;
            let grace_length = grace.duration_time() as u32;
            let grace_velocity = grace.velocity;
            let grace_duration = if grace.is_dead {
                apply_static_duration(tempo, DEFAULT_DURATION_DEAD, grace_length)
            } else {
                grace_length
            };
            let on_beat_duration = *note_start - grace_length;
            if grace.is_on_beat || on_beat_duration < QUARTER_TIME {
                *note_start = note_start.saturating_add(grace_length);
                *duration = duration.saturating_sub(grace_length);
            }
            self.add_note(
                track_id,
                grace_key,
                *note_start - grace_length,
                grace_duration,
                grace_velocity,
                channel_id,
            );
        }

        // trill
        if let Some(trill) = &note.effect.trill
            && !is_percussion
        {
            let trill_key = track_offset + i32::from(trill.fret) + string_tuning;
            let mut trill_length = trill.duration.time();

            let trill_tick_limit = *note_start + *duration;
            let mut real_key = false;
            let mut tick = *note_start;

            let mut counter = 0;
            while tick + 10 < trill_tick_limit {
                if tick + trill_length >= trill_tick_limit {
                    trill_length = trill_tick_limit - tick - 1;
                }
                let iter_key = if real_key { initial_key } else { trill_key };
                self.add_note(track_id, iter_key, tick, trill_length, velocity, channel_id);
                real_key = !real_key;
                tick += trill_length;
                counter += 1;
            }
            assert!(
                counter > 0,
                "No trill notes published! trill_length: {trill_length}, tick: {tick}, trill_tick_limit: {trill_tick_limit}"
            );

            // all notes published - the caller does not need to publish the note
            return None;
        }

        // tremolo picking
        if let Some(tremolo_picking) = &note.effect.tremolo_picking {
            let mut tp_length = tremolo_picking.duration.time();
            let mut tick = *note_start;
            let tp_tick_limit = *note_start + *duration;
            let mut counter = 0;
            while tick + 10 < tp_tick_limit {
                if tick + tp_length >= tp_tick_limit {
                    tp_length = tp_tick_limit - tick - 1;
                }
                self.add_note(track_id, initial_key, tick, tp_length, velocity, channel_id);
                tick += tp_length;
                counter += 1;
            }
            assert!(
                counter > 0,
                "No tremolo notes published! tp_length: {tp_length}, tick: {tick}, tp_tick_limit: {tp_tick_limit}"
            );
            // all notes published - the caller does not need to publish the note
            return None;
        }

        // bend
        if let Some(bend_effect) = &note.effect.bend
            && !is_percussion
        {
            self.add_bend(track_id, *note_start, *duration, channel_id, bend_effect);
        }

        // tremolo bar
        if let Some(tremolo_bar) = &note.effect.tremolo_bar
            && !is_percussion
        {
            self.add_tremolo_bar(track_id, *note_start, *duration, channel_id, tremolo_bar);
        }

        // slide
        if let Some(_slide) = &note.effect.slide
            && !is_percussion
            && let Some((next_beat, next_note)) = next_note_beat
        {
            let value_1 = i32::from(note.value);
            let value_2 = i32::from(next_note.value);

            let tick1 = *note_start;
            let tick2 = next_beat.start;

            // make slide
            let distance: i32 = value_2 - value_1;
            let length: i32 = (tick2 - tick1) as i32;
            let points = length / (QUARTER_TIME / 8) as i32;
            for p_offset in 1..=points {
                let tone = ((length / points) * p_offset) * distance / length;
                let bend = DEFAULT_BEND + (tone as f32 * DEFAULT_BEND_SEMI_TONE * 2.0);
                let bend_tick = tick1 as i32 + (length / points) * p_offset;
                self.add_pitch_bend(bend_tick as u32, track_id, channel_id, bend as i32);
            }

            // normalise the bend
            self.add_pitch_bend(tick2, track_id, channel_id, DEFAULT_BEND as i32);
        }

        // vibrato
        if note.effect.vibrato && !is_percussion {
            self.add_vibrato(track_id, *note_start, *duration, channel_id);
        }

        // harmonic
        if let Some(harmonic) = &note.effect.harmonic
            && !is_percussion
        {
            match harmonic.kind {
                HarmonicType::Natural => {
                    for (harmonic_value, harmonic_frequency) in NATURAL_FREQUENCIES {
                        if note.value % 12 == (harmonic_value % 12) as i16 {
                            key = (initial_key + harmonic_frequency) - i32::from(note.value);
                            break;
                        }
                    }
                }
                HarmonicType::Semi => {
                    let velocity = MIN_VELOCITY.max(velocity - VELOCITY_INCREMENT * 3);
                    self.add_note(
                        track_id,
                        initial_key,
                        *note_start,
                        *duration,
                        velocity,
                        channel_id,
                    );
                    key = initial_key + NATURAL_FREQUENCIES[0].1;
                }
                HarmonicType::Artificial | HarmonicType::Pinch => {
                    key = initial_key + NATURAL_FREQUENCIES[0].1;
                }
                HarmonicType::Tapped => {
                    if let Some(right_hand_fret) = harmonic.right_hand_fret {
                        for (harmonic_value, harmonic_frequency) in NATURAL_FREQUENCIES {
                            if i16::from(right_hand_fret) - note.value == harmonic_value as i16 {
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
                    *note_start,
                    *duration,
                    velocity,
                    channel_id,
                );
            }
        }

        Some(key)
    }

    fn add_vibrato(&mut self, track_id: usize, start: u32, duration: u32, channel_id: i32) {
        let end = start + duration;
        let mut next_start = start;
        while next_start < end {
            next_start = if next_start + 160 > end {
                end
            } else {
                next_start + 160
            };
            self.add_pitch_bend(next_start, track_id, channel_id, DEFAULT_BEND as i32);

            next_start = if next_start + 160 > end {
                end
            } else {
                next_start + 160
            };
            let value = DEFAULT_BEND + DEFAULT_BEND_SEMI_TONE / 2.0;
            self.add_pitch_bend(next_start, track_id, channel_id, value as i32);
        }
        self.add_pitch_bend(next_start, track_id, channel_id, DEFAULT_BEND as i32);
    }

    fn add_bend(
        &mut self,
        track_id: usize,
        start: u32,
        duration: u32,
        channel_id: i32,
        bend: &BendEffect,
    ) {
        for (point_id, point) in bend.points.iter().enumerate() {
            let value =
                DEFAULT_BEND + (f32::from(point.value) * DEFAULT_BEND_SEMI_TONE / SEMITONE_LENGTH);
            let value = value.clamp(0.0, 127.0) as i32;
            let bend_start = start + point.get_time(duration);
            self.add_pitch_bend(bend_start, track_id, channel_id, value);

            // look ahead to next bend point
            if let Some(next_point) = bend.points.get(point_id + 1) {
                let next_value = DEFAULT_BEND
                    + (f32::from(next_point.value) * DEFAULT_BEND_SEMI_TONE / SEMITONE_LENGTH);
                self.process_next_bend_values(
                    track_id,
                    channel_id,
                    value,
                    next_value as i32,
                    bend_start,
                    start,
                    next_point,
                    duration,
                );
            }
        }
        self.add_pitch_bend(start + duration, track_id, channel_id, DEFAULT_BEND as i32);
    }

    #[allow(clippy::too_many_arguments)]
    fn process_next_bend_values(
        &mut self,
        track_id: usize,
        channel_id: i32,
        mut value: i32,
        next_value: i32,
        mut bend_start: u32,
        start: u32,
        next_point: &BendPoint,
        duration: u32,
    ) {
        if value != next_value {
            let next_bend_start = start + next_point.get_time(duration);
            let width = (next_bend_start - bend_start) as f32 / (next_value - value).abs() as f32;
            let width = width as u32;
            // ascending
            if value < next_value {
                while value < next_value {
                    value += 1;
                    bend_start += width;
                    // clamp to 127
                    let value = value.min(127);
                    self.add_pitch_bend(bend_start, track_id, channel_id, value);
                }
            }
            // descending
            if value > next_value {
                while value > next_value {
                    value -= 1;
                    bend_start += width;
                    // clamp to 0
                    let value = value.max(0);
                    self.add_pitch_bend(bend_start, track_id, channel_id, value);
                }
            }
        }
    }

    fn add_tremolo_bar(
        &mut self,
        track_id: usize,
        start: u32,
        duration: u32,
        channel_id: i32,
        tremolo_bar: &TremoloBarEffect,
    ) {
        for (point_id, point) in tremolo_bar.points.iter().enumerate() {
            let value = DEFAULT_BEND + (f32::from(point.value) * DEFAULT_BEND_SEMI_TONE * 2.0);
            let value = value.clamp(0.0, 127.0) as i32;
            let bend_start = start + point.get_time(duration);
            self.add_pitch_bend(bend_start, track_id, channel_id, value);

            // look ahead to next bend point
            if let Some(next_point) = tremolo_bar.points.get(point_id + 1) {
                let next_value =
                    DEFAULT_BEND + (f32::from(next_point.value) * DEFAULT_BEND_SEMI_TONE * 2.0);
                self.process_next_bend_values(
                    track_id,
                    channel_id,
                    value,
                    next_value as i32,
                    bend_start,
                    start,
                    next_point,
                    duration,
                );
            }
        }
        self.add_pitch_bend(start + duration, track_id, channel_id, DEFAULT_BEND as i32);
    }

    fn add_note(
        &mut self,
        track_id: usize,
        key: i32,
        start: u32,
        duration: u32,
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

    fn add_tempo_change(&mut self, tick: u32, tempo: u32) {
        let event = MidiEvent::new_tempo_change(tick, tempo);
        self.add_event(event);
    }

    fn add_bank_selection(&mut self, tick: u32, track_id: usize, channel: i32, bank: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x00, bank);
        self.add_event(event);
    }

    fn add_volume_selection(&mut self, tick: u32, track_id: usize, channel: i32, volume: i32) {
        // Channel Volume coarse (CC 0x07); the prior 0x27 (LSB) left the coarse
        // byte at its default, so per-track volume was effectively ignored.
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x07, volume);
        self.add_event(event);
    }

    fn add_balance_selection(&mut self, tick: u32, track_id: usize, channel: i32, balance: i32) {
        // Pan controller (CC 0x0A).
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x0A, balance);
        self.add_event(event);
    }

    fn add_expression_selection(
        &mut self,
        tick: u32,
        track_id: usize,
        channel: i32,
        expression: i32,
    ) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x2B, expression);
        self.add_event(event);
    }

    fn add_chorus_selection(&mut self, tick: u32, track_id: usize, channel: i32, chorus: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x5D, chorus);
        self.add_event(event);
    }

    fn add_reverb_selection(&mut self, tick: u32, track_id: usize, channel: i32, reverb: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x5B, reverb);
        self.add_event(event);
    }

    fn add_pitch_bend(&mut self, tick: u32, track_id: usize, channel: i32, value: i32) {
        // GP uses a value between 0 and 128
        // MIDI uses a value between 0 and 16383 (128 * 128)
        let midi_value = value * 128;

        // the bend value must be split into two bytes and sent to the synthesizer.
        let data1 = midi_value & 0x7F;
        let data2 = midi_value >> 7;
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xE0, data1, data2);
        self.add_event(event);
    }

    fn add_expression(&mut self, tick: u32, track_id: usize, channel: i32, expression: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x0B, expression);
        self.add_event(event);
    }

    fn add_program_selection(&mut self, tick: u32, track_id: usize, channel: i32, program: i32) {
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xC0, program, 0);
        self.add_event(event);
    }

    fn add_pitch_bend_range(&mut self, tick: u32, track_id: usize, channel: i32) {
        // RPN MSB: Select RPN group (usually 0)
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x65, 0);
        self.add_event(event);

        // RPN LSB: Select RPN 0/0 (Pitch Bend Sensitivity)
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x64, 0);
        self.add_event(event);

        // Data Entry MSB: Set the value (Pitch Bend Range)
        // 12 semitones for the guitar
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x06, 12);
        self.add_event(event);

        // Data Entry LSB: Cents (usually 0)
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x26, 0);
        self.add_event(event);
    }

    fn add_track_channel_midi_control(&mut self, track_id: usize, midi_channel: &MidiChannel) {
        let channel_id = midi_channel.channel_id;
        // publish MIDI control messages for the track channel at the start
        let info_tick = FIRST_TICK;
        self.add_volume_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            to_channel_short(midi_channel.volume),
        );
        self.add_balance_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            to_channel_short(midi_channel.balance),
        );
        self.add_expression_selection(info_tick, track_id, i32::from(channel_id), 127);
        self.add_chorus_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            to_channel_short(midi_channel.chorus),
        );
        self.add_reverb_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            to_channel_short(midi_channel.reverb),
        );
        self.add_bank_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            i32::from(midi_channel.bank),
        );
        self.add_program_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            midi_channel.instrument,
        );
        self.add_pitch_bend_range(info_tick, track_id, i32::from(channel_id));
    }

    fn add_event(&mut self, event: MidiEvent) {
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::to_channel_short;

    #[test]
    fn channel_short_scaling() {
        // Raw Guitar Pro channel scale (0-16) -> MIDI (0-127), per TuxGuitar.
        assert_eq!(to_channel_short(0), 0); // silent / hard left
        assert_eq!(to_channel_short(8), 63); // half / center
        assert_eq!(to_channel_short(16), 127); // full / hard right
        // Out-of-range values are clamped.
        assert_eq!(to_channel_short(-1), 0);
        assert_eq!(to_channel_short(127), 127);
    }
}
