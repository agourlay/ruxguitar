//! Port of TuxGuitar's `MidiSequenceParser.java`.

use crate::audio::midi_event::{FIRST_TICK, MidiEvent};
use crate::audio::playback_order::playback_tick;
use crate::parser::song_parser::{
    Beat, BendEffect, BendPoint, GraceEffect, HarmonicEffect, HarmonicType, MIN_VELOCITY, Measure,
    MeasureHeader, MidiChannel, NATURAL_FREQUENCIES, Note, NoteType, QUARTER_TIME, SEMITONE_LENGTH,
    Song, Track, TremoloBarEffect, VELOCITY_INCREMENT,
};
use std::rc::Rc;

#[cfg(test)]
use crate::audio::playback_order::compute_playback_order;

use super::effects::{
    DEFAULT_DURATION_DEAD, TripletAdjustment, apply_duration_effect, apply_static_duration,
    apply_triplet_feel, apply_velocity_effect, compute_stroke_offsets, next_note_on_string,
    previous_note_on_string,
};

const DEFAULT_BEND: f32 = 64.0;
const DEFAULT_BEND_SEMI_TONE: f32 = 2.75;

// MIDI control change numbers. The coarse (MSB) controllers are the ones that
// matter here: their LSB counterparts leave the coarse byte at its default,
// which silently ignores the setting.
const CC_BANK_SELECT: i32 = 0x00;
const CC_VOLUME: i32 = 0x07;
const CC_PAN: i32 = 0x0A;
const CC_EXPRESSION: i32 = 0x0B;
const CC_REVERB: i32 = 0x5B;
const CC_TREMOLO: i32 = 0x5C;
const CC_CHORUS: i32 = 0x5D;
const CC_PHASER: i32 = 0x5F;

/// Pseudo track id carried by metronome events, so playback can gate them
/// on the metronome toggle instead of the regular mute/solo filtering.
pub const METRONOME_TRACK: u8 = u8::MAX;
/// Claves layered with both woodblocks: a woody metronome tock. A single
/// key is too quiet in TimGM6mb to cut through the mix, and the side
/// stick (TuxGuitar's choice) sounds like a drum hit.
pub const METRONOME_KEYS: [i32; 3] = [75, 76, 77];
const METRONOME_CHANNEL: i32 = 9;
pub const METRONOME_VELOCITY: i16 = 127;

/// Scale a raw Guitar Pro channel byte (0-16) to a MIDI value (0-127),
/// matching TuxGuitar's `toChannelShort`. Used for channel volume, pan,
/// chorus and reverb (raw 16 -> 127, raw 8 -> 63, raw 0 -> 0).
fn to_channel_short(value: i8) -> i32 {
    (i32::from(value) * 8 - 1).clamp(0, 127)
}

pub struct MidiBuilder {
    events: Vec<MidiEvent>,
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
            // like TuxGuitar, skip tracks with an unresolvable channel
            let Some(midi_channel) = song
                .midi_channels
                .iter()
                .find(|c| c.channel_id == track.channel_id)
            else {
                log::warn!(
                    "midi channel {} not found for track {track_id} - skipping track",
                    track.channel_id
                );
                continue;
            };
            // the synthesizer only has 16 channels: clamp out-of-range channels
            // (GP files can reference more) instead of crashing
            let midi_channel = if midi_channel.channel_id < 16 {
                midi_channel.clone()
            } else {
                log::warn!(
                    "clamping out-of-range midi channel {} for track {track_id}",
                    midi_channel.channel_id
                );
                let mut clamped = midi_channel.clone();
                clamped.channel_id = 15;
                clamped
            };
            self.add_track_events(
                song.tempo.value,
                track_id,
                track,
                &song.measure_headers,
                playback_order,
                &midi_channel,
            );
        }
        self.add_metronome_events(&song.measure_headers, playback_order);
        // Sort events by tick
        self.events.sort_by_key(|event| event.tick);
        self.events
    }

    /// One percussion click per beat of every played measure, like
    /// TuxGuitar's `addMetronome`; playback gates them on the toggle.
    fn add_metronome_events(
        &mut self,
        measure_headers: &[MeasureHeader],
        playback_order: &[(usize, i64)],
    ) {
        for (measure_index, tick_offset) in playback_order {
            let header = &measure_headers[*measure_index];
            let length = header.time_signature.denominator.time();
            let mut start = header.start;
            for _ in 1..=header.time_signature.numerator {
                let tick = playback_tick(start, *tick_offset);
                for key in METRONOME_KEYS {
                    self.add_note(
                        usize::from(METRONOME_TRACK),
                        key,
                        tick,
                        length,
                        METRONOME_VELOCITY,
                        METRONOME_CHANNEL,
                    );
                }
                start += length;
            }
        }
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
        for (playback_index, (measure_index, tick_offset)) in playback_order.iter().enumerate() {
            let measure = &track.measures[*measure_index];
            let measure_header = &measure_headers[*measure_index];

            // add song info events once for all tracks
            if track_id == 0 {
                // change tempo if necessary
                let measure_tempo = measure_header.tempo.value;
                if measure_tempo != prev_tempo {
                    let tick = playback_tick(measure_header.start, *tick_offset);
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
                playback_order,
                playback_index,
            );
            // shift events generated for this measure by tick_offset
            if *tick_offset != 0 {
                for event in &mut self.events[event_start..] {
                    event.tick = playback_tick(event.tick, *tick_offset);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_beat_events(
        &mut self,
        track_id: usize,
        track: &Track,
        measure: &Measure,
        measure_header: &MeasureHeader,
        midi_channel: &MidiChannel,
        strings: &[(i32, i32)],
        playback_order: &[(usize, i64)],
        playback_index: usize,
    ) {
        for (voice_id, voice) in measure.voices.iter().enumerate() {
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
                    // check the same voice in the next measure if it was the last beat
                    track
                        .measures
                        .get(voice.measure_index as usize + 1)
                        .and_then(|next_measure| next_measure.voices.get(voice_id))
                        .and_then(|next_voice| next_voice.beats.first())
                });
                // apply triplet feel adjustment to beat timing
                let triplet_adj =
                    apply_triplet_feel(beat, previous_beat, next_beat, measure_header.triplet_feel);
                self.add_notes(
                    track_id,
                    track,
                    playback_order,
                    playback_index,
                    voice_id,
                    measure_header,
                    midi_channel,
                    beat_id,
                    beat,
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
        playback_order: &[(usize, i64)],
        playback_index: usize,
        voice_id: usize,
        measure_header: &MeasureHeader,
        midi_channel: &MidiChannel,
        beat_id: usize,
        beat: &Beat,
        strings: &[(i32, i32)],
        triplet_adj: TripletAdjustment,
    ) {
        // GP files define an effect channel per track, but TuxGuitar doesn't use it for playback.
        let channel_id = midi_channel.channel_id;
        let tempo = measure_header.tempo.value;
        let track_offset = track.offset;
        let beat_duration = triplet_adj.duration;
        let stroke = &beat.effect.stroke;
        let stroke_increment = stroke.increment_for_duration(beat_duration);
        // pre-compute per-string stroke offsets (only for strings with non-tied notes)
        let stroke_offsets = compute_stroke_offsets(beat, stroke_increment, strings.len());
        for note in &beat.notes {
            if note.kind != NoteType::Tie {
                // skip notes referencing a string the track does not have
                let Some(&(_, string_tuning)) = usize::try_from(note.string)
                    .ok()
                    .and_then(|s| s.checked_sub(1))
                    .and_then(|s| strings.get(s))
                else {
                    log::warn!("invalid string {} for track {track_id}", note.string);
                    continue;
                };

                // note starts on beat (adjusted for triplet feel)
                let mut note_start = triplet_adj.start;

                // apply effects on duration
                let duration = apply_duration_effect(
                    track,
                    playback_order,
                    playback_index,
                    voice_id,
                    beat_id,
                    note,
                    measure_header.repeat_close > 0,
                    tempo,
                    beat_duration,
                );
                // degenerate durations (e.g. corrupt tuplet ratios) would emit
                // a NoteOn without NoteOff
                let mut duration = duration.max(1);

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

                // previous note played on the same string (crossing barlines
                // and repeats), for the hammer-on velocity attenuation
                let previous_note = if midi_channel.is_percussion() {
                    None
                } else {
                    previous_note_on_string(
                        track,
                        playback_order,
                        playback_index,
                        voice_id,
                        beat_id,
                        note.string,
                    )
                };

                // slide target: next note played on the same string, with its
                // tick expressed relative to the current playback entry since
                // this measure's events are shifted by its own offset later
                let slide_target = if note.effect.slide.is_some() {
                    next_note_on_string(
                        track,
                        playback_order,
                        playback_index,
                        voice_id,
                        beat_id,
                        note.string,
                    )
                    .map(|(m_move, next_beat, next_note)| {
                        let current_move = playback_order[playback_index].1;
                        let tick2 = i64::from(next_beat.start) + m_move - current_move;
                        (tick2.max(0) as u32, next_note.value)
                    })
                } else {
                    None
                };

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
                    slide_target,
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

    /// Play the effects that change a note's key or timing, and the extra
    /// notes they bring. Returns the key the note itself sounds at, or `None`
    /// when an effect already played it as repeated notes.
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
        slide_target: Option<(u32, i16)>,
        velocity: i16,
        midi_channel: &MidiChannel,
    ) -> Option<i32> {
        let channel_id = i32::from(midi_channel.channel_id);
        let is_percussion = midi_channel.is_percussion();
        let fret_key = |fret: i32| track_offset + fret + string_tuning;
        // the fretted key, before any effect
        let initial_key = fret_key(i32::from(note.value));

        if note.effect.fade_in {
            self.add_fade_in(track_id, channel_id, *note_start, *duration);
        }

        if let Some(grace) = &note.effect.grace {
            let grace_key = fret_key(i32::from(grace.fret));
            self.add_grace_note(
                track_id, channel_id, grace, grace_key, tempo, note_start, duration,
            );
        }

        // a trill or tremolo picking plays the note itself, as repeated notes
        if let Some(trill) = &note.effect.trill
            && !is_percussion
        {
            let trill_key = fret_key(i32::from(trill.fret));
            // the real fretted note sounds first, then alternates with the trill fret
            self.add_repeated_notes(
                track_id,
                channel_id,
                [initial_key, trill_key],
                trill.duration.time(),
                *note_start,
                *duration,
                velocity,
            );
            return None;
        }
        if let Some(tremolo_picking) = &note.effect.tremolo_picking {
            self.add_repeated_notes(
                track_id,
                channel_id,
                [initial_key, initial_key],
                tremolo_picking.duration.time(),
                *note_start,
                *duration,
                velocity,
            );
            return None;
        }

        if is_percussion {
            return Some(initial_key);
        }

        // pitch-modulating effects share the channel pitch wheel and are
        // mutually exclusive: bend > tremolo bar > slide > vibrato
        if let Some(bend_effect) = &note.effect.bend {
            self.add_bend(track_id, *note_start, *duration, channel_id, bend_effect);
        } else if let Some(tremolo_bar) = &note.effect.tremolo_bar {
            self.add_tremolo_bar(track_id, *note_start, *duration, channel_id, tremolo_bar);
        } else if note.effect.slide.is_some() {
            // the target must lie after the (triplet-adjusted) note start
            if let Some(target) = slide_target.filter(|(tick, _)| *tick > *note_start) {
                self.add_slide(track_id, channel_id, *note_start, note.value, target);
            }
        } else if note.effect.vibrato {
            self.add_vibrato(track_id, *note_start, *duration, channel_id);
        }

        let Some(harmonic) = &note.effect.harmonic else {
            return Some(initial_key);
        };
        Some(self.add_harmonic(
            track_id,
            channel_id,
            harmonic,
            note.value,
            initial_key,
            *note_start,
            *duration,
            velocity,
        ))
    }

    /// Raise the channel expression over the note, then restore it.
    fn add_fade_in(&mut self, track_id: usize, channel_id: i32, start: u32, duration: u32) {
        let mut expression = 31;
        let expression_increment = 1;
        let mut tick = start;
        let tick_increment = duration / ((127 - expression) / expression_increment);
        while tick < (start + duration) && expression < 127 {
            self.add_control_change(tick, track_id, channel_id, CC_EXPRESSION, expression as i32);
            tick += tick_increment;
            expression += expression_increment;
        }
        // normalize the expression
        self.add_control_change(start + duration, track_id, channel_id, CC_EXPRESSION, 127);
    }

    /// Play the grace note before the note. A grace note on the beat, or one
    /// with too little room before the note, delays the note instead.
    #[allow(clippy::too_many_arguments)]
    fn add_grace_note(
        &mut self,
        track_id: usize,
        channel_id: i32,
        grace: &GraceEffect,
        grace_key: i32,
        tempo: u32,
        note_start: &mut u32,
        duration: &mut u32,
    ) {
        let grace_length = grace.duration_time() as u32;
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
            grace.velocity,
            channel_id,
        );
    }

    /// Fill the note's span with notes of `step` ticks, alternating between
    /// the two keys. Like TuxGuitar, a span too short plays nothing.
    #[allow(clippy::too_many_arguments)]
    fn add_repeated_notes(
        &mut self,
        track_id: usize,
        channel_id: i32,
        keys: [i32; 2],
        mut step: u32,
        start: u32,
        duration: u32,
        velocity: i16,
    ) {
        let tick_limit = start + duration;
        let mut tick = start;
        let mut key_index = 0;
        while tick + 10 < tick_limit {
            if tick + step >= tick_limit {
                step = tick_limit - tick - 1;
            }
            self.add_note(track_id, keys[key_index], tick, step, velocity, channel_id);
            key_index = 1 - key_index;
            tick += step;
        }
    }

    /// Glide the pitch wheel from the note to the next one on its string.
    fn add_slide(
        &mut self,
        track_id: usize,
        channel_id: i32,
        start: u32,
        from_value: i16,
        (target_tick, target_value): (u32, i16),
    ) {
        let distance = i32::from(target_value) - i32::from(from_value);
        let length = (target_tick - start) as i32;
        let points = length / (QUARTER_TIME / 8) as i32;
        for p_offset in 1..=points {
            let tone = ((length / points) * p_offset * distance) as f32 / length as f32;
            let bend = DEFAULT_BEND as i32 + (tone * DEFAULT_BEND_SEMI_TONE * 2.0) as i32;
            let bend_tick = start as i32 + (length / points) * p_offset;
            self.add_pitch_bend(bend_tick as u32, track_id, channel_id, bend);
        }
        // normalise the bend
        self.add_pitch_bend(target_tick, track_id, channel_id, DEFAULT_BEND as i32);
    }

    /// Play a harmonic: its octave below, softer, and for a semi harmonic the
    /// fretted note too. Returns the key the harmonic itself sounds at.
    #[allow(clippy::too_many_arguments)]
    fn add_harmonic(
        &mut self,
        track_id: usize,
        channel_id: i32,
        harmonic: &HarmonicEffect,
        note_value: i16,
        initial_key: i32,
        start: u32,
        duration: u32,
        velocity: i16,
    ) -> i32 {
        if harmonic.kind == HarmonicType::Semi {
            let velocity = MIN_VELOCITY.max(velocity - VELOCITY_INCREMENT * 3);
            self.add_note(track_id, initial_key, start, duration, velocity, channel_id);
        }
        let key = harmonic_key(harmonic, note_value, initial_key);
        if key - 12 > 0 {
            let velocity = MIN_VELOCITY.max(velocity - VELOCITY_INCREMENT * 4);
            self.add_note(track_id, key - 12, start, duration, velocity, channel_id);
        }
        key
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
            // truncate the offset before adding, like TuxGuitar: adding first
            // rounds negative dips one unit lower
            let value = DEFAULT_BEND as i32
                + (f32::from(point.value) * DEFAULT_BEND_SEMI_TONE * 2.0) as i32;
            let bend_start = start + point.get_time(duration);
            self.add_pitch_bend(bend_start, track_id, channel_id, value);

            // look ahead to next bend point
            if let Some(next_point) = tremolo_bar.points.get(point_id + 1) {
                let next_value = DEFAULT_BEND as i32
                    + (f32::from(next_point.value) * DEFAULT_BEND_SEMI_TONE * 2.0) as i32;
                self.process_next_bend_values(
                    track_id, channel_id, value, next_value, bend_start, start, next_point,
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
        // clamp like TuxGuitar: harmonics can push the key past the MIDI
        // range, and the synthesizer silently drops out-of-range keys
        let key = key.clamp(0, 127);
        self.add_event(MidiEvent::new_note_on(
            start, track_id, key, velocity, channel,
        ));
        if duration > 0 {
            self.add_event(MidiEvent::new_note_off(
                start + duration,
                track_id,
                key,
                channel,
            ));
        }
    }

    fn add_tempo_change(&mut self, tick: u32, tempo: u32) {
        self.add_event(MidiEvent::new_tempo_change(tick, tempo));
    }

    /// Emit a control change (0xB0) for the given controller number.
    fn add_control_change(
        &mut self,
        tick: u32,
        track_id: usize,
        channel: i32,
        controller: i32,
        value: i32,
    ) {
        self.add_event(MidiEvent::new_midi_message(
            tick, track_id, channel, 0xB0, controller, value,
        ));
    }

    fn add_pitch_bend(&mut self, tick: u32, track_id: usize, channel: i32, value: i32) {
        // clamp to a valid MIDI data byte, like TuxGuitar's fix():
        // steep slides can otherwise overshoot the wheel range
        let value = value.clamp(0, 127);
        // GP uses a value between 0 and 128
        // MIDI uses a value between 0 and 16383 (128 * 128)
        let midi_value = value * 128;

        // the bend value must be split into two bytes and sent to the synthesizer.
        let data1 = midi_value & 0x7F;
        let data2 = midi_value >> 7;
        self.add_event(MidiEvent::new_midi_message(
            tick, track_id, channel, 0xE0, data1, data2,
        ));
    }

    fn add_program_selection(&mut self, tick: u32, track_id: usize, channel: i32, program: i32) {
        self.add_event(MidiEvent::new_midi_message(
            tick, track_id, channel, 0xC0, program, 0,
        ));
    }

    /// Set the pitch bend range to 12 semitones through RPN 0/0.
    fn add_pitch_bend_range(&mut self, tick: u32, track_id: usize, channel: i32) {
        // RPN MSB: select RPN group (usually 0)
        self.add_control_change(tick, track_id, channel, 0x65, 0);
        // RPN LSB: select RPN 0/0 (pitch bend sensitivity)
        self.add_control_change(tick, track_id, channel, 0x64, 0);
        // Data entry MSB: the range in semitones
        self.add_control_change(tick, track_id, channel, 0x06, 12);
        // Data entry LSB: cents (usually 0)
        self.add_control_change(tick, track_id, channel, 0x26, 0);
    }

    fn add_track_channel_midi_control(&mut self, track_id: usize, midi_channel: &MidiChannel) {
        // publish MIDI control messages for the track channel at the start
        let tick = FIRST_TICK;
        let channel = i32::from(midi_channel.channel_id);
        let mut control = |controller, value| {
            self.add_control_change(tick, track_id, channel, controller, value);
        };
        control(CC_VOLUME, to_channel_short(midi_channel.volume));
        control(CC_PAN, to_channel_short(midi_channel.balance));
        control(CC_EXPRESSION, 127);
        control(CC_CHORUS, to_channel_short(midi_channel.chorus));
        control(CC_REVERB, to_channel_short(midi_channel.reverb));
        control(CC_PHASER, to_channel_short(midi_channel.phaser));
        control(CC_TREMOLO, to_channel_short(midi_channel.tremolo));
        control(CC_BANK_SELECT, i32::from(midi_channel.bank));
        self.add_program_selection(tick, track_id, channel, midi_channel.instrument);
        self.add_pitch_bend_range(tick, track_id, channel);
    }

    fn add_event(&mut self, event: MidiEvent) {
        self.events.push(event);
    }
}

/// The key a harmonic sounds at, from the fretted key, like TuxGuitar.
fn harmonic_key(harmonic: &HarmonicEffect, note_value: i16, initial_key: i32) -> i32 {
    match harmonic.kind {
        // the harmonic of the fret the note sits on, if it has one
        HarmonicType::Natural => NATURAL_FREQUENCIES
            .iter()
            .find(|(harmonic_value, _)| note_value % 12 == (harmonic_value % 12) as i16)
            .map_or(initial_key, |(_, frequency)| {
                initial_key + frequency - i32::from(note_value)
            }),
        HarmonicType::Semi => initial_key + NATURAL_FREQUENCIES[0].1,
        HarmonicType::Artificial | HarmonicType::Pinch => {
            let data = harmonic.data.min(NATURAL_FREQUENCIES.len() - 1);
            initial_key + NATURAL_FREQUENCIES[data].1
        }
        // like TuxGuitar, an unmatched tap interval still plays an octave harmonic
        HarmonicType::Tapped => harmonic
            .right_hand_fret
            .and_then(|right_hand_fret| {
                NATURAL_FREQUENCIES.iter().find(|(harmonic_value, _)| {
                    i16::from(right_hand_fret) - note_value == *harmonic_value as i16
                })
            })
            .map_or(initial_key + NATURAL_FREQUENCIES[0].1, |(_, frequency)| {
                initial_key + frequency
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::midi_event::MidiEventType;
    use crate::parser::song_parser::DEFAULT_VELOCITY;

    const TRACK: usize = 0;
    const CHANNEL: i32 = 0;

    fn harmonic(kind: HarmonicType) -> HarmonicEffect {
        HarmonicEffect {
            kind,
            ..Default::default()
        }
    }

    /// The (tick, key, length) of each note played, in order.
    fn notes_played(events: &[MidiEvent]) -> Vec<(u32, i32, u32)> {
        events
            .iter()
            .filter_map(|event| match event.event {
                MidiEventType::NoteOn(_, key, _) => {
                    let off = events.iter().find(|other| {
                        other.tick >= event.tick
                            && matches!(other.event, MidiEventType::NoteOff(_, k) if k == key)
                    });
                    Some((event.tick, key, off.map_or(0, |off| off.tick - event.tick)))
                }
                _ => None,
            })
            .collect()
    }

    /// The (tick, wheel value) of each pitch bend, in GP's 0-127 scale.
    fn pitch_bends(events: &[MidiEvent]) -> Vec<(u32, i32)> {
        events
            .iter()
            .filter_map(|event| match event.event {
                MidiEventType::MidiMessage(_, 0xE0, data1, data2) => {
                    Some((event.tick, ((data2 << 7) | data1) / 128))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_natural_harmonic_sounds_at_its_fret_harmonic() {
        let natural = harmonic(HarmonicType::Natural);
        // the 12th fret harmonic is the open string an octave up
        assert_eq!(harmonic_key(&natural, 12, 40 + 12), 40 + 12);
        // the 5th fret harmonic is two octaves up
        assert_eq!(harmonic_key(&natural, 5, 40 + 5), 40 + 24);
        // a fret without a harmonic plays the fretted note
        assert_eq!(harmonic_key(&natural, 2, 42), 42);
    }

    #[test]
    fn an_artificial_harmonic_picks_its_interval_from_data() {
        let mut artificial = harmonic(HarmonicType::Artificial);
        artificial.data = 3;
        assert_eq!(harmonic_key(&artificial, 2, 42), 42 + 19);
        // out of range data is clamped to the last interval
        artificial.data = 99;
        assert_eq!(harmonic_key(&artificial, 2, 42), 42 + 31);
    }

    #[test]
    fn a_tapped_harmonic_follows_the_right_hand_fret() {
        let mut tapped = harmonic(HarmonicType::Tapped);
        tapped.right_hand_fret = Some(7);
        // tapped 5 frets above the note
        assert_eq!(harmonic_key(&tapped, 2, 42), 42 + 24);
        // an interval without a harmonic plays the octave
        tapped.right_hand_fret = Some(3);
        assert_eq!(harmonic_key(&tapped, 2, 42), 42 + 12);
    }

    #[test]
    fn a_semi_harmonic_also_plays_the_fretted_note() {
        let mut builder = MidiBuilder::new();
        let key = builder.add_harmonic(
            TRACK,
            CHANNEL,
            &harmonic(HarmonicType::Semi),
            2,
            42,
            0,
            100,
            DEFAULT_VELOCITY,
        );
        assert_eq!(key, 54);
        // the fretted note, then the harmonic's octave below
        assert_eq!(notes_played(&builder.events), [(0, 42, 100), (0, 42, 100)]);
    }

    #[test]
    fn repeated_notes_alternate_and_stop_at_the_note_end() {
        let mut builder = MidiBuilder::new();
        builder.add_repeated_notes(TRACK, CHANNEL, [40, 42], 100, 100, 400, DEFAULT_VELOCITY);
        assert_eq!(
            notes_played(&builder.events),
            [
                (100, 40, 100),
                (200, 42, 100),
                (300, 40, 100),
                (400, 42, 99)
            ]
        );
    }

    #[test]
    fn repeated_notes_need_room_to_play() {
        let mut builder = MidiBuilder::new();
        builder.add_repeated_notes(TRACK, CHANNEL, [40, 40], 100, 100, 10, DEFAULT_VELOCITY);
        assert!(builder.events.is_empty());
    }

    #[test]
    fn a_grace_note_on_the_beat_delays_the_note() {
        let grace = GraceEffect {
            duration: 2,
            is_on_beat: true,
            ..Default::default()
        };
        let length = grace.duration_time() as u32;
        let mut start = QUARTER_TIME * 4;
        let mut duration = QUARTER_TIME;
        let mut builder = MidiBuilder::new();
        builder.add_grace_note(TRACK, CHANNEL, &grace, 44, 120, &mut start, &mut duration);
        assert_eq!(start, QUARTER_TIME * 4 + length);
        assert_eq!(duration, QUARTER_TIME - length);
        assert_eq!(
            notes_played(&builder.events),
            [(QUARTER_TIME * 4, 44, length)]
        );
    }

    #[test]
    fn a_grace_note_before_the_beat_leaves_the_note_in_place() {
        let grace = GraceEffect {
            duration: 2,
            ..Default::default()
        };
        let length = grace.duration_time() as u32;
        let mut start = QUARTER_TIME * 4;
        let mut duration = QUARTER_TIME;
        let mut builder = MidiBuilder::new();
        builder.add_grace_note(TRACK, CHANNEL, &grace, 44, 120, &mut start, &mut duration);
        assert_eq!((start, duration), (QUARTER_TIME * 4, QUARTER_TIME));
        assert_eq!(
            notes_played(&builder.events),
            [(QUARTER_TIME * 4 - length, 44, length)]
        );
    }

    #[test]
    fn a_fade_in_ends_at_full_expression() {
        let mut builder = MidiBuilder::new();
        builder.add_fade_in(TRACK, CHANNEL, 0, QUARTER_TIME);
        let expressions: Vec<(u32, i32)> = builder
            .events
            .iter()
            .filter_map(|event| match event.event {
                MidiEventType::MidiMessage(_, 0xB0, CC_EXPRESSION, value) => {
                    Some((event.tick, value))
                }
                _ => None,
            })
            .collect();
        assert_eq!(expressions.first(), Some(&(0, 31)));
        assert_eq!(expressions.last(), Some(&(QUARTER_TIME, 127)));
        assert!(expressions.is_sorted_by_key(|(_, value)| *value));
    }

    #[test]
    fn a_slide_glides_to_the_target_then_resets_the_wheel() {
        let mut builder = MidiBuilder::new();
        builder.add_slide(TRACK, CHANNEL, 0, 5, (QUARTER_TIME, 7));
        let bends = pitch_bends(&builder.events);
        // a point every 32nd note, ending two semitones up before the reset
        assert_eq!(bends.len(), 9);
        assert!(bends[..8].is_sorted_by_key(|(_, value)| *value));
        assert_eq!(bends[7], (QUARTER_TIME, 64 + 11));
        assert_eq!(bends[8], (QUARTER_TIME, 64));
    }

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
