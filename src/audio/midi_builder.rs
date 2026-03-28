use crate::audio::FIRST_TICK;
/// Thanks to `TuxGuitar` for the reference implementation in `MidiSequenceParser.java`
use crate::audio::midi_event::MidiEvent;
use crate::parser::song_parser::{
    Beat, BendEffect, BendPoint, HarmonicType, MIN_VELOCITY, Measure, MeasureHeader, MidiChannel,
    Note, NoteType, QUARTER_TIME, SEMITONE_LENGTH, Song, Track, TremoloBarEffect, TripletFeel,
    VELOCITY_INCREMENT,
};
use std::collections::HashMap;
use std::rc::Rc;

const DEFAULT_DURATION_DEAD: u32 = 30;
const DEFAULT_DURATION_PM: u32 = 60;
const DEFAULT_BEND: f32 = 64.0;
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
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Parse song and record events
    pub fn build_for_song(mut self, song: &Rc<Song>) -> Vec<MidiEvent> {
        // compute playback order once (shared across all tracks)
        let playback_order = compute_playback_order(&song.measure_headers);
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
                &playback_order,
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
        let mut uses_triplet_feel = false;

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

            if measure_header.triplet_feel != TripletFeel::None {
                uses_triplet_feel = true;
            }
        }
        if uses_triplet_feel {
            log::warn!("Triplet feel not supported on track {track_id}");
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
    ) {
        let channel_id = midi_channel.channel_id;
        let tempo = measure_header.tempo.value;
        // TODO when to use effect channel instead?
        assert!(channel_id < 16);
        let track_offset = track.offset;
        let beat_duration = beat.duration.time();
        let stroke = &beat.effect.stroke;
        let _stroke_increment = stroke.increment_for_duration(beat_duration);
        for note in &beat.notes {
            if note.kind != NoteType::Tie {
                let (string_id, string_tuning) = strings[note.string as usize - 1];
                assert_eq!(string_id, i32::from(note.string));

                // note starts on beat
                let mut note_start = beat.start;

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

                // TODO apply stroke effect

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
        let event = MidiEvent::new_midi_message(tick, track_id, channel, 0xB0, 0x27, volume);
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
            i32::from(midi_channel.volume),
        );
        self.add_expression_selection(info_tick, track_id, i32::from(channel_id), 127);
        self.add_chorus_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            i32::from(midi_channel.chorus),
        );
        self.add_reverb_selection(
            info_tick,
            track_id,
            i32::from(channel_id),
            i32::from(midi_channel.reverb),
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
    velocity.min(127)
}

fn apply_duration_effect(
    track: &Track,
    measure_id: usize,
    beat_id: usize,
    note: &Note,
    first_next_beat: Option<&Beat>,
    tempo: u32,
    mut duration: u32,
) -> u32 {
    let note_type = &note.kind;
    let next_beats_in_next_measures = track.measures[measure_id..]
        .iter()
        .flat_map(|m| m.voices[0].beats.iter())
        .skip(beat_id + 1); // skip current and previous beats

    // handle chains of tie notes
    for next_beat in next_beats_in_next_measures {
        // filter for only next notes on matching string
        if let Some(next_note) = next_beat.notes.iter().find(|n| n.string == note.string) {
            if next_note.kind == NoteType::Tie {
                duration += next_beat.duration.time();
            } else {
                // stop chain
                break;
            }
        } else {
            // break chain of tie notes
            break;
        }
    }
    // hande let-ring
    if let Some(first_next_beat) = first_next_beat
        && note.effect.let_ring
    {
        duration += first_next_beat.duration.time();
    }
    if note_type == &NoteType::Dead {
        return apply_static_duration(tempo, DEFAULT_DURATION_DEAD, duration);
    }
    if note.effect.palm_mute {
        return apply_static_duration(tempo, DEFAULT_DURATION_PM, duration);
    }
    if note.effect.staccato {
        return (duration as f32 * 50.0 / 100.00) as u32;
    }
    duration
}

fn apply_static_duration(tempo: u32, duration: u32, maximum: u32) -> u32 {
    let value = tempo * duration / 60;
    value.min(maximum)
}

/// Tracks the state of repeat section navigation during playback order computation.
struct RepeatState {
    start_stack: Vec<usize>,    // stack of repeat_open indices (for nesting)
    visits: HashMap<usize, i8>, // how many times each repeat_close has been hit
    current_repetition: i8,     // 0-based: 0 = first play, 1 = first repeat, etc.
    jumping_back: bool,         // true when looping back to a repeat_open
}

impl RepeatState {
    fn new() -> Self {
        Self {
            start_stack: vec![0], // implicit start at measure 0
            visits: HashMap::new(),
            current_repetition: 0,
            jumping_back: false,
        }
    }

    /// Process a repeat_open marker. Pushes onto the stack on first entry,
    /// skips the push when looping back (to preserve the repetition counter).
    fn enter_repeat(&mut self, measure_index: usize) {
        if !self.jumping_back {
            self.current_repetition = 0;
            self.start_stack.push(measure_index);
        }
        self.jumping_back = false;
    }

    /// Check if the current repetition matches an alternative ending bitmask.
    fn matches_alternative(&self, repeat_alternative: u8) -> bool {
        // clamp to 7 to avoid shift overflow on u8
        let clamped = self.current_repetition.min(7);
        let bit = 1_u8 << clamped;
        repeat_alternative & bit != 0
    }

    /// Process a repeat_close marker. Returns the index to jump back to,
    /// or None if all repetitions are done.
    fn close_repeat(&mut self, measure_index: usize, repeat_close: i8) -> Option<usize> {
        let visits = self.visits.entry(measure_index).or_insert(0);
        if *visits < repeat_close {
            *visits += 1;
            self.current_repetition += 1;
            self.jumping_back = true;
            let repeat_start = *self.start_stack.last().unwrap_or(&0);
            // clear visit counts for inner repeats so they replay on next outer pass
            self.visits
                .retain(|&k, _| k <= repeat_start || k >= measure_index);
            Some(repeat_start)
        } else {
            // done repeating
            self.visits.remove(&measure_index);
            self.start_stack.pop();
            None
        }
    }
}

/// Compute the playback order of measures, expanding repeats and alternative endings.
///
/// Used by the MIDI builder to generate events at the correct ticks,
/// and by the tablature to map playback ticks back to visual measures.
/// Returns a Vec of (measure_index, tick_offset) pairs.
/// The tick_offset is the difference between the playback tick and the original measure tick.
pub fn compute_playback_order(headers: &[MeasureHeader]) -> Vec<(usize, i64)> {
    let mut order: Vec<(usize, i64)> = Vec::new();
    let mut running_tick: u32 = QUARTER_TIME; // same starting tick as parser
    let mut repeat = RepeatState::new();
    let mut i = 0;

    while i < headers.len() {
        let header = &headers[i];

        if header.repeat_open {
            repeat.enter_repeat(i);
        }

        // check alternative ending: skip this measure if it doesn't match current repetition
        if header.repeat_alternative != 0 && !repeat.matches_alternative(header.repeat_alternative)
        {
            // still check for repeat_close on this skipped measure
            if header.repeat_close > 0
                && let Some(jump_to) = repeat.close_repeat(i, header.repeat_close)
            {
                i = jump_to;
                continue;
            }
            i += 1;
            continue;
        }

        // add this measure to the playback order
        let tick_offset = i64::from(running_tick) - i64::from(header.start);
        order.push((i, tick_offset));
        running_tick += header.length();

        // handle repeat close
        if header.repeat_close > 0
            && let Some(jump_to) = repeat.close_repeat(i, header.repeat_close)
        {
            i = jump_to;
            continue;
        }

        i += 1;
    }

    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::midi_event::MidiEventType;
    use crate::parser::song_parser_tests::parse_gp_file;
    use std::collections::HashSet;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    #[test]
    fn test_midi_events_for_all_files() {
        let test_dir = Path::new("test-files");
        let gold_dir = Path::new("test-files/gold-generated-midi");
        for entry in std::fs::read_dir(test_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            let extension = path.extension().unwrap();
            if extension != "gp5" && extension != "gp4" {
                continue;
            }
            let file_name = path.file_name().unwrap().to_str().unwrap();
            eprintln!("Parsing file: {file_name}");
            let file_path = path.to_str().unwrap();
            let song = parse_gp_file(file_path)
                .unwrap_or_else(|err| panic!("Failed to parse file: {file_name}\n{err}"));
            let song = Rc::new(song);
            let builder = MidiBuilder::new();
            let events = builder.build_for_song(&song);
            assert!(!events.is_empty(), "No events found for {file_name}");

            // assert sorted by tick
            assert!(events.windows(2).all(|w| w[0].tick <= w[1].tick));
            assert_eq!(events[0].tick, 1);

            // check against golden file
            let gold_file_path = gold_dir.join(format!("{file_name}.txt"));
            if !gold_file_path.exists() {
                // create gold file
                let mut file = std::fs::File::create(&gold_file_path).unwrap();
                for event in &events {
                    writeln!(file, "{}", print_event(event)).unwrap();
                }
            }

            // verify against gold file
            validate_gold_rendered_result(&events, gold_file_path);
        }
    }

    fn print_event(event: &MidiEvent) -> String {
        format!("{:?} {:?} {:?}", event.tick, event.event, event.track)
    }

    fn validate_gold_rendered_result(events: &[MidiEvent], gold_path: PathBuf) {
        let gold = std::fs::read_to_string(&gold_path).expect("gold file not found!");
        let mut expected_lines = events.iter().map(print_event);
        for (i1, l1) in gold.lines().enumerate() {
            let l2 = expected_lines.next().unwrap();
            if l1.trim_end() != l2.trim_end() {
                println!("## GOLD line {} ##", i1 + 1);
                println!("{}", l1.trim_end());
                println!("## ACTUAL ##");
                println!("{}", l2.trim_end());
                println!("#####");
                assert_eq!(l1, l2, "line {i1} failed for {gold_path:?}");
            }
        }
    }

    #[test]
    fn test_midi_events_for_demo_song() {
        const FILE_PATH: &str = "test-files/Demo v5.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);

        assert_eq!(events.len(), 4693);
        assert_eq!(events[0].tick, 1);

        // assert number of tracks
        let track_count = song.tracks.len();
        let unique_tracks: HashSet<_> = events.iter().map(|event| event.track).collect();
        assert_eq!(unique_tracks.len(), track_count + 1); // plus None for info events

        // skip MIDI program messages
        let rhythm_track_events: Vec<_> = events
            .iter()
            .filter(|e| e.track == Some(0))
            .skip(10)
            .collect();

        // print 20 first for debugging
        // for (i, event) in rhythm_track_events.iter().enumerate().take(20) {
        //     eprintln!("{} {:?}", i, event);
        // }

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
            .skip(10)
            .collect();

        //print 100 first for debugging
        for (i, event) in solo_track_events.iter().enumerate().take(100) {
            eprintln!("{i} {event:?}");
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

        // pass some trill notes...

        // trill ON
        let event = &solo_track_events[30];
        assert_eq!(event.tick, 16080);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 69, 95)));

        // trill OFF
        let event = &solo_track_events[31];
        assert_eq!(event.tick, 16319);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 69)));

        // tremolo ON (repeated section)
        let event = &solo_track_events[32];
        assert_eq!(event.tick, 27840);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 60, 95)));

        // tremolo OFF
        let event = &solo_track_events[33];
        assert_eq!(event.tick, 27960);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 60)));

        // note ON (after all tremolo and repeated sections)
        let event = &solo_track_events[64];
        assert_eq!(event.tick, 77760);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 63, 95)));

        // note OFF
        let event = &solo_track_events[65];
        assert_eq!(event.tick, 78240);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 63)));

        // note ON hammer
        let event = &solo_track_events[66];
        assert_eq!(event.tick, 78240);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOn(2, 65, 70)));

        // note OFF hammer
        let event = &solo_track_events[67];
        assert_eq!(event.tick, 78720);
        assert_eq!(event.track, Some(1));
        assert!(matches!(event.event, MidiEventType::NoteOff(2, 65)));
    }

    #[test]
    fn test_midi_events_for_bleed() {
        const FILE_PATH: &str = "test-files/Meshuggah - Bleed.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);

        assert_eq!(events.len(), 44442);
        assert_eq!(events[0].tick, 1);

        // assert number of tracks
        let track_count = song.tracks.len();
        let unique_tracks: HashSet<_> = events.iter().map(|event| event.track).collect();
        assert_eq!(unique_tracks.len(), track_count);

        // skip MIDI program messages
        let rhythm_track_events: Vec<_> = events
            .iter()
            .filter(|e| e.track == Some(0))
            .skip(10)
            .collect();

        // print 60 first for debugging
        // for (i, event) in rhythm_track_events.iter().enumerate().take(100) {
        //     eprintln!("{} {:?}", i, event);
        // }

        let event = &rhythm_track_events[44];
        assert_eq!(event.tick, 4800);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

        let event = &rhythm_track_events[45];
        assert_eq!(event.tick, 4915);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

        let event = &rhythm_track_events[46];
        assert_eq!(event.tick, 5040);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

        let event = &rhythm_track_events[47];
        assert_eq!(event.tick, 5155);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

        let event = &rhythm_track_events[48];
        assert_eq!(event.tick, 5280);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

        let event = &rhythm_track_events[49];
        assert_eq!(event.tick, 5395);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

        let event = &rhythm_track_events[50];
        assert_eq!(event.tick, 5400);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

        let event = &rhythm_track_events[51];
        assert_eq!(event.tick, 5515);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

        let event = &rhythm_track_events[52];
        assert_eq!(event.tick, 5520);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

        let event = &rhythm_track_events[50];
        assert_eq!(event.tick, 5400);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

        let event = &rhythm_track_events[51];
        assert_eq!(event.tick, 5515);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

        let event = &rhythm_track_events[52];
        assert_eq!(event.tick, 5520);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

        let event = &rhythm_track_events[53];
        assert_eq!(event.tick, 5635);
        assert_eq!(event.track, Some(0));
        assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));
    }

    /// Helper to create a measure header with a given start tick
    fn make_header(start: u32, repeat_open: bool, repeat_close: i8) -> MeasureHeader {
        MeasureHeader {
            start,
            repeat_open,
            repeat_close,
            ..MeasureHeader::default()
        }
    }

    #[test]
    fn playback_order_no_repeats() {
        // 3 measures, no repeats → linear order, zero offsets
        let headers = vec![
            make_header(960, false, 0),
            make_header(4800, false, 0),
            make_header(8640, false, 0),
        ];
        let order = compute_playback_order(&headers);
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], (0, 0));
        assert_eq!(order[1], (1, 0));
        assert_eq!(order[2], (2, 0));
    }

    #[test]
    fn playback_order_simple_repeat() {
        // |: M0 | M1 :|  M2
        // Plays: M0 M1 M0 M1 M2
        let measure_len = 3840_u32; // default 4/4 length
        let headers = vec![
            make_header(960, true, 0),
            make_header(960 + measure_len, false, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 1, 2]);

        // tick offsets: first pass is 0, second pass shifts by 2 measures
        assert_eq!(order[0].1, 0); // M0 first time: no offset
        assert_eq!(order[1].1, 0); // M1 first time: no offset
        assert_eq!(order[2].1, i64::from(measure_len) * 2); // M0 second time: shifted
        assert_eq!(order[3].1, i64::from(measure_len) * 2); // M1 second time: shifted
        assert_eq!(order[4].1, i64::from(measure_len) * 2); // M2: shifted by the repeated section
    }

    #[test]
    fn playback_order_repeat_three_times() {
        // |: M0 :| x3  M1
        // Plays: M0 M0 M0 M1
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 2), // repeat_close=2 means 2 extra plays (3 total)
            make_header(960 + measure_len, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 0, 0, 1]);
    }

    #[test]
    fn playback_order_two_repeat_sections() {
        // |: M0 :|  |: M1 :|  M2
        // Plays: M0 M0 M1 M1 M2
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 1),
            make_header(960 + measure_len, true, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 0, 1, 1, 2]);
    }

    #[test]
    fn playback_order_alternative_endings() {
        // |: M0 | M1[1.] | M2[2.] :|  M3
        // Plays: M0 M1 M0 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 1, // bit 0 = 1st ending
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 2, // bit 1 = 2nd ending
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 2, 3]);
    }

    #[test]
    fn playback_order_three_alternatives() {
        // |: M0 | M1[1.] | M2[2.] | M3[3.] :|x3  M4
        // Plays: M0 M1 M0 M2 M0 M3 M4
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 1, // bit 0
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 2, // bit 1
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 3,
                repeat_alternative: 4, // bit 2
                repeat_close: 2,       // 2 extra repeats (3 total)
                ..MeasureHeader::default()
            },
            make_header(960 + measure_len * 4, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 2, 0, 3, 4]);
    }

    #[test]
    fn playback_order_nested_repeats() {
        // |: M0 |: M1 :| M2 :|  M3
        // Inner plays M1 twice each time outer loops.
        // Outer plays M0-M1-M1-M2 twice.
        // Plays: M0 M1 M1 M2 M0 M1 M1 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),                    // M0: outer repeat open
            make_header(960 + measure_len, true, 1),      // M1: inner repeat open+close
            make_header(960 + measure_len * 2, false, 1), // M2: outer repeat close
            make_header(960 + measure_len * 3, false, 0), // M3: after repeats
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 1, 2, 0, 1, 1, 2, 3]);
    }

    #[test]
    fn playback_order_repeat_close_without_open() {
        // M0 | M1 :|  M2
        // No explicit repeat_open — implicit start at measure 0
        // Plays: M0 M1 M0 M1 M2
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, false, 0),
            make_header(960 + measure_len, false, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 1, 2]);
    }

    #[test]
    fn playback_order_single_measure_repeat() {
        // |: M0 :|  — single measure repeated
        // Plays: M0 M0
        let headers = vec![make_header(960, true, 1)];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 0]);
    }

    #[test]
    fn playback_order_tick_offsets_are_consistent() {
        // Verify that tick offsets produce a monotonically increasing playback timeline
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            make_header(960 + measure_len, false, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        // compute playback ticks: original_start + offset
        let playback_ticks: Vec<i64> = order
            .iter()
            .map(|(idx, offset)| i64::from(headers[*idx].start) + offset)
            .collect();
        // verify monotonically increasing
        assert!(playback_ticks.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn playback_order_alternative_on_last_pass_with_close() {
        // |: M0 | M1[1.+2.] :|  — alternative plays on both passes
        // Plays: M0 M1 M0 M1
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 3, // bits 0+1 = both passes
                repeat_close: 1,
                ..MeasureHeader::default()
            },
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 1]);
    }

    #[test]
    fn playback_order_empty() {
        let headers: Vec<MeasureHeader> = vec![];
        let order = compute_playback_order(&headers);
        assert!(order.is_empty());
    }

    #[test]
    fn playback_order_damage_control() {
        const FILE_PATH: &str = "test-files/John Petrucci - Damage Control.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let headers = &song.measure_headers;

        // verify repeat markers were parsed
        // Repeat structure (1-indexed measures):
        //   M1: repeat_open
        //   M2: repeat_close=1, repeat_alternative=1 (1st ending)
        //   M3: repeat_alternative=2 (2nd ending)
        //   M8: repeat_open
        //   M9: repeat_close=1
        //   M74: repeat_open, repeat_close=7 (plays 8 times)
        assert!(headers[0].repeat_open);
        assert_eq!(headers[1].repeat_close, 1);
        assert_eq!(headers[1].repeat_alternative, 1);
        assert_eq!(headers[2].repeat_alternative, 2);
        assert!(headers[7].repeat_open);
        assert_eq!(headers[8].repeat_close, 1);
        assert!(headers[73].repeat_open);
        assert_eq!(headers[73].repeat_close, 7);

        let order = compute_playback_order(headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();

        // playback order should be longer than header count due to repeats
        assert!(
            order.len() > headers.len(),
            "Playback order ({}) should be > header count ({})",
            order.len(),
            headers.len()
        );

        // section 1: |: M0 | M1[1.] | M2[2.] → plays M0 M1 M0 M2
        assert_eq!(&indices[..4], &[0, 1, 0, 2]);

        // section 2: M3..M6 play linearly, then |: M7 | M8 :| → plays M7 M8 M7 M8
        // find where M7 (index 7) first appears after the first section
        let m7_positions: Vec<usize> = indices
            .iter()
            .enumerate()
            .filter(|(_, idx)| **idx == 7)
            .map(|(pos, _)| pos)
            .collect();
        assert_eq!(m7_positions.len(), 2, "M7 should appear twice (repeat)");
        // M8 should follow each M7
        assert_eq!(indices[m7_positions[0] + 1], 8);
        assert_eq!(indices[m7_positions[1] + 1], 8);

        // section 3: M74 (index 73) has repeat_close=7, so it plays 8 times
        let m73_count = indices.iter().filter(|&&idx| idx == 73).count();
        assert_eq!(m73_count, 8, "M74 should appear 8 times (repeat_close=7)");

        // verify all playback ticks are monotonically increasing
        let playback_ticks: Vec<i64> = order
            .iter()
            .map(|(idx, offset)| i64::from(headers[*idx].start) + offset)
            .collect();
        for window in playback_ticks.windows(2) {
            assert!(
                window[0] < window[1],
                "Playback ticks not monotonically increasing: {} >= {}",
                window[0],
                window[1]
            );
        }

        // verify all measure indices are valid
        for (idx, _) in &order {
            assert!(*idx < headers.len(), "Invalid measure index {idx}");
        }

        // build MIDI events and verify they are sorted
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);
        assert!(!events.is_empty());
        assert!(
            events.windows(2).all(|w| w[0].tick <= w[1].tick),
            "Events not sorted by tick"
        );
    }
}
