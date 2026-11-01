//! Pure musical-effect and timing helpers for the MIDI builder.
//! These are stateless transforms (velocity, duration, triplet feel, strokes)
//! with no MIDI/event-emitting side effects.

use super::builder::MeasureContext;
use crate::parser::song_parser::{
    Beat, BeatStrokeDirection, MIN_VELOCITY, MidiChannel, Note, NoteType, QUARTER_TIME, Track,
    TripletFeel, VELOCITY_INCREMENT,
};

pub(super) const DEFAULT_DURATION_DEAD: u32 = 30;
const DEFAULT_DURATION_PM: u32 = 60;

pub(super) fn apply_velocity_effect(
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

/// Extend the note duration through tie chains and let-ring, walking the
/// note's own voice over the expanded playback order (repeats included)
/// like TuxGuitar's `getRealNoteDuration`.
pub(super) fn apply_duration_effect(
    context: &MeasureContext,
    voice_id: usize,
    beat_id: usize,
    note: &Note,
    duration: u32,
) -> u32 {
    let MeasureContext {
        track,
        header,
        playback_order,
        playback_index,
        ..
    } = *context;
    let repeat_close = header.repeat_close > 0;
    let tempo = header.tempo.value;
    let (measure_id, current_move) = playback_order[playback_index];
    let note_beat = &track.measures[measure_id].voices[voice_id].beats[beat_id];
    let mut let_ring = note.effect.let_ring;
    let mut last_end =
        current_move + i64::from(note_beat.start) + i64::from(note_beat.duration.time());
    let mut real_duration = i64::from(duration);

    'walk: for (entry_id, (m_id, m_move)) in playback_order[playback_index..].iter().enumerate() {
        let in_next_measure = entry_id > 0;
        let skip_beats = if in_next_measure { 0 } else { beat_id + 1 };
        for beat in track.measures[*m_id].voices[voice_id]
            .beats
            .iter()
            .skip(skip_beats)
        {
            if beat.empty {
                continue;
            }
            if beat.notes.is_empty() {
                // a rest ends the chain
                break 'walk;
            }
            let beat_time = i64::from(beat.duration.time());
            let mut tied_on_string = false;
            for next_note in &beat.notes {
                if let_ring {
                    // let-ring does not survive a repeat close
                    if in_next_measure && repeat_close {
                        break 'walk;
                    }
                    // end of the let-ring chain: ring through the ending beat
                    if !next_note.effect.let_ring {
                        real_duration += beat_time;
                        break 'walk;
                    }
                }
                if next_note.string == note.string {
                    if next_note.kind == NoteType::Tie {
                        // gap-aware: also cover any distance since the chain's last end
                        real_duration += (m_move + i64::from(beat.start) - last_end) + beat_time;
                        last_end = m_move + i64::from(beat.start) + beat_time;
                        let_ring = next_note.effect.let_ring;
                        tied_on_string = true;
                    } else {
                        // a new note on the string ends the chain
                        break 'walk;
                    }
                }
            }
            // ring through beats without a tie continuation
            if let_ring && !tied_on_string {
                real_duration += beat_time;
                last_end += beat_time;
            }
        }
    }

    let duration = real_duration.max(0) as u32;
    if note.kind == NoteType::Dead {
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

pub(super) fn apply_static_duration(tempo: u32, duration: u32, maximum: u32) -> u32 {
    let value = tempo * duration / 60;
    value.min(maximum)
}

/// Find the next note on the same string, walking the note's voice over the
/// expanded playback order like TuxGuitar's `getNextNote` with `breakAtRest`:
/// the first beat where the voice plays either provides the note or ends the
/// search. Returns the playback move of the target's measure with the note.
pub(super) fn next_note_on_string<'a>(
    track: &'a Track,
    playback_order: &[(usize, i64)],
    playback_index: usize,
    voice_id: usize,
    beat_id: usize,
    string: i8,
) -> Option<(i64, &'a Beat, &'a Note)> {
    let mut skip_beats = beat_id + 1;
    for (m_id, m_move) in &playback_order[playback_index..] {
        for beat in track.measures[*m_id].voices[voice_id]
            .beats
            .iter()
            .skip(skip_beats)
        {
            if beat.empty {
                continue;
            }
            return beat
                .notes
                .iter()
                .find(|n| n.string == string)
                .map(|n| (*m_move, beat, n));
        }
        skip_beats = 0;
    }
    None
}

/// Find the previous note played on the same string, walking the note's voice
/// backwards over the expanded playback order like TuxGuitar's
/// `getPreviousNote` without `breakAtRest`: beats without a note on the
/// string are skipped.
pub(super) fn previous_note_on_string<'a>(
    track: &'a Track,
    playback_order: &[(usize, i64)],
    playback_index: usize,
    voice_id: usize,
    beat_id: usize,
    string: i8,
) -> Option<&'a Note> {
    let mut end_beats = Some(beat_id);
    for (m_id, _) in playback_order[..=playback_index].iter().rev() {
        let beats = &track.measures[*m_id].voices[voice_id].beats;
        let end = end_beats.take().unwrap_or(beats.len());
        for beat in beats[..end].iter().rev() {
            if beat.empty {
                continue;
            }
            if let Some(n) = beat.notes.iter().find(|n| n.string == string) {
                return Some(n);
            }
        }
    }
    None
}

/// Triplet feel adjustment for a beat's start and duration.
pub(super) struct TripletAdjustment {
    pub(super) start: u32,
    pub(super) duration: u32,
}

/// Apply triplet feel (swing) to a beat's timing.
/// Pairs of equal-duration notes are converted to a long-short triplet pattern.
pub(super) fn apply_triplet_feel(
    beat: &Beat,
    previous_beat: Option<&Beat>,
    next_beat: Option<&Beat>,
    triplet_feel: TripletFeel,
) -> TripletAdjustment {
    let beat_start = beat.start;
    let beat_duration = beat.duration.time();

    match triplet_feel {
        TripletFeel::None => TripletAdjustment {
            start: beat_start,
            duration: beat_duration,
        },
        TripletFeel::Eighth => apply_triplet_feel_for_duration(
            beat_start,
            beat_duration,
            previous_beat,
            next_beat,
            QUARTER_TIME / 2,
            QUARTER_TIME,
        ),
        TripletFeel::Sixteenth => apply_triplet_feel_for_duration(
            beat_start,
            beat_duration,
            previous_beat,
            next_beat,
            QUARTER_TIME / 4,
            QUARTER_TIME / 2,
        ),
    }
}

/// Apply triplet feel for a specific note duration level.
/// `target_duration` is the straight note duration to match (e.g., 480 for eighth, 240 for sixteenth).
/// `boundary` is the rhythmic boundary for pairing (e.g., 960 for eighth pairs, 480 for sixteenth pairs).
fn apply_triplet_feel_for_duration(
    beat_start: u32,
    beat_duration: u32,
    previous_beat: Option<&Beat>,
    next_beat: Option<&Beat>,
    target_duration: u32,
    boundary: u32,
) -> TripletAdjustment {
    if beat_duration != target_duration {
        return TripletAdjustment {
            start: beat_start,
            duration: beat_duration,
        };
    }

    // triplet duration = target_duration * 2 / 3
    let triplet_duration = target_duration * 2 / 3;

    // first beat of pair: on the boundary
    if beat_start.is_multiple_of(boundary) {
        // check that next beat is also the same duration (forming a pair)
        let next_qualifies = next_beat.is_none_or(|nb| {
            nb.start > beat_start + beat_duration || nb.duration.time() == target_duration
        });
        if next_qualifies {
            return TripletAdjustment {
                start: beat_start,
                duration: triplet_duration * 2, // long note
            };
        }
    }
    // second beat of pair: on the half-boundary
    else if beat_start.is_multiple_of(boundary / 2) {
        // check that previous beat is also the same duration
        let prev_qualifies = previous_beat.is_none_or(|pb| {
            pb.start < beat_start - beat_duration || pb.duration.time() == target_duration
        });
        if prev_qualifies {
            let adjusted_start = (beat_start - beat_duration) + triplet_duration * 2;
            return TripletAdjustment {
                start: adjusted_start,
                duration: triplet_duration, // short note
            };
        }
    }

    TripletAdjustment {
        start: beat_start,
        duration: beat_duration,
    }
}

/// Compute per-string stroke offsets for a beat, following TuxGuitar's approach:
/// only strings with non-tied notes receive incremental offsets.
pub(super) fn compute_stroke_offsets(
    beat: &Beat,
    stroke_increment: u32,
    string_count: usize,
) -> Vec<u32> {
    let mut offsets = vec![0_u32; string_count];
    if stroke_increment == 0 || beat.effect.stroke.direction == BeatStrokeDirection::None {
        return offsets;
    }

    // build bitmask of strings that have non-tied notes
    let mut strings_used: u32 = 0;
    for note in &beat.notes {
        if note.kind != NoteType::Tie {
            strings_used |= 1 << (note.string as u32 - 1);
        }
    }

    // assign cumulative offsets in stroke direction order
    let mut stroke_move: u32 = 0;
    for i in 0..string_count {
        let index = match beat.effect.stroke.direction {
            BeatStrokeDirection::Down => (string_count - 1) - i,
            BeatStrokeDirection::Up => i,
            BeatStrokeDirection::None => unreachable!(),
        };
        if strings_used & (1 << index) != 0 {
            offsets[index] = stroke_move;
            stroke_move += stroke_increment;
        }
    }

    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::song_parser::{
        DEFAULT_BANK, DEFAULT_PERCUSSION_BANK, DEFAULT_VELOCITY, Measure, MeasureHeader,
        NoteEffect, Voice,
    };

    fn channel(bank: u8) -> MidiChannel {
        MidiChannel {
            channel_id: 0,
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

    fn note(string: i8, kind: NoteType) -> Note {
        let mut note = Note::new(NoteEffect::default());
        note.string = string;
        note.kind = kind;
        note
    }

    /// A quarter note beat starting at `start`.
    fn beat(start: u32, notes: Vec<Note>) -> Beat {
        Beat {
            notes,
            start,
            ..Default::default()
        }
    }

    /// One measure per beat list, played once each in order.
    fn track(measures: Vec<Vec<Beat>>) -> (Track, Vec<(usize, i64)>) {
        let order = (0..measures.len()).map(|m| (m, 0)).collect();
        let measures = measures
            .into_iter()
            .map(|beats| Measure {
                voices: vec![Voice {
                    measure_index: 0,
                    beats,
                }],
                ..Default::default()
            })
            .collect();
        let track = Track {
            measures,
            ..Default::default()
        };
        (track, order)
    }

    /// The first note of the first beat, the one whose duration is measured.
    fn first_note(track: &Track) -> &Note {
        &track.measures[0].voices[0].beats[0].notes[0]
    }

    /// The duration of a quarter note in the first measure, at 120 bpm.
    fn duration_of(track: &Track, order: &[(usize, i64)], beat_id: usize, note: &Note) -> u32 {
        let header = MeasureHeader::default();
        let guitar = channel(DEFAULT_BANK);
        let context = MeasureContext {
            track_id: 0,
            track,
            header: &header,
            midi_channel: &guitar,
            playback_order: order,
            playback_index: 0,
        };
        apply_duration_effect(&context, 0, beat_id, note, QUARTER_TIME)
    }

    #[test]
    fn accents_and_ghosts_shift_the_velocity() {
        let guitar = channel(DEFAULT_BANK);
        let mut plain = note(1, NoteType::Normal);
        assert_eq!(
            apply_velocity_effect(&plain, None, &guitar),
            DEFAULT_VELOCITY
        );
        plain.effect.ghost_note = true;
        let ghost = apply_velocity_effect(&plain, None, &guitar);
        plain.effect.ghost_note = false;
        plain.effect.accentuated_note = true;
        let accent = apply_velocity_effect(&plain, None, &guitar);
        plain.effect.accentuated_note = false;
        plain.effect.heavy_accentuated_note = true;
        let heavy = apply_velocity_effect(&plain, None, &guitar);
        assert!(ghost < DEFAULT_VELOCITY);
        assert!(
            DEFAULT_VELOCITY < accent && accent < heavy,
            "{accent} {heavy}"
        );
    }

    #[test]
    fn velocity_stays_in_the_midi_range() {
        let mut loud = note(1, NoteType::Normal);
        loud.velocity = 127;
        loud.effect.heavy_accentuated_note = true;
        assert_eq!(
            apply_velocity_effect(&loud, None, &channel(DEFAULT_BANK)),
            127
        );
        let mut quiet = note(1, NoteType::Normal);
        quiet.velocity = MIN_VELOCITY;
        quiet.effect.ghost_note = true;
        assert_eq!(
            apply_velocity_effect(&quiet, None, &channel(DEFAULT_BANK)),
            MIN_VELOCITY
        );
    }

    #[test]
    fn a_note_after_a_hammer_on_is_softer_except_on_drums() {
        let mut hammer = note(1, NoteType::Normal);
        hammer.effect.hammer = true;
        let played = note(1, NoteType::Normal);
        let guitar = apply_velocity_effect(&played, Some(&hammer), &channel(DEFAULT_BANK));
        assert_eq!(guitar, DEFAULT_VELOCITY - 25);
        let drums =
            apply_velocity_effect(&played, Some(&hammer), &channel(DEFAULT_PERCUSSION_BANK));
        assert_eq!(drums, DEFAULT_VELOCITY);
    }

    #[test]
    fn a_static_duration_follows_the_tempo_up_to_the_note_length() {
        assert_eq!(apply_static_duration(120, DEFAULT_DURATION_DEAD, 1000), 60);
        assert_eq!(apply_static_duration(120, DEFAULT_DURATION_DEAD, 40), 40);
    }

    #[test]
    fn a_tie_extends_the_note_across_the_barline() {
        let (track, order) = track(vec![
            vec![beat(0, vec![note(1, NoteType::Normal)])],
            vec![beat(QUARTER_TIME, vec![note(1, NoteType::Tie)])],
        ]);
        let played = first_note(&track);
        assert_eq!(duration_of(&track, &order, 0, played), QUARTER_TIME * 2);
    }

    #[test]
    fn a_new_note_on_the_string_ends_the_tie_chain() {
        let (track, order) = track(vec![vec![
            beat(0, vec![note(1, NoteType::Normal)]),
            beat(QUARTER_TIME, vec![note(1, NoteType::Normal)]),
            beat(QUARTER_TIME * 2, vec![note(1, NoteType::Tie)]),
        ]]);
        assert_eq!(
            duration_of(&track, &order, 0, first_note(&track)),
            QUARTER_TIME
        );
    }

    #[test]
    fn let_ring_rings_through_the_beat_that_ends_it() {
        let mut ringing = note(1, NoteType::Normal);
        ringing.effect.let_ring = true;
        let (track, order) = track(vec![vec![
            beat(0, vec![ringing]),
            beat(QUARTER_TIME, vec![note(2, NoteType::Normal)]),
        ]]);
        assert_eq!(
            duration_of(&track, &order, 0, first_note(&track)),
            QUARTER_TIME * 2
        );
    }

    #[test]
    fn dead_palm_muted_and_staccato_notes_are_cut_short() {
        let (track, order) = track(vec![vec![beat(0, vec![note(1, NoteType::Normal)])]]);
        let dead = note(1, NoteType::Dead);
        assert_eq!(duration_of(&track, &order, 0, &dead), 60);
        let mut muted = note(1, NoteType::Normal);
        muted.effect.palm_mute = true;
        assert_eq!(duration_of(&track, &order, 0, &muted), 120);
        let mut staccato = note(1, NoteType::Normal);
        staccato.effect.staccato = true;
        assert_eq!(duration_of(&track, &order, 0, &staccato), QUARTER_TIME / 2);
    }

    #[test]
    fn the_next_note_search_stops_at_the_first_beat_played() {
        let (track, order) = track(vec![
            vec![beat(0, vec![note(1, NoteType::Normal)])],
            vec![
                beat(QUARTER_TIME, vec![note(2, NoteType::Normal)]),
                beat(QUARTER_TIME * 2, vec![note(1, NoteType::Normal)]),
            ],
        ]);
        // the next beat plays string 2 only: no target on string 1
        assert!(next_note_on_string(&track, &order, 0, 0, 0, 1).is_none());
        let (_, next_beat, _) = next_note_on_string(&track, &order, 0, 0, 0, 2).unwrap();
        assert_eq!(next_beat.start, QUARTER_TIME);
    }

    #[test]
    fn the_previous_note_search_skips_other_strings_across_measures() {
        let mut first = note(1, NoteType::Normal);
        first.value = 5;
        let (track, order) = track(vec![
            vec![beat(0, vec![first])],
            vec![
                beat(QUARTER_TIME, vec![note(2, NoteType::Normal)]),
                beat(QUARTER_TIME * 2, vec![note(1, NoteType::Normal)]),
            ],
        ]);
        let previous = previous_note_on_string(&track, &order, 1, 0, 1, 1).unwrap();
        assert_eq!(previous.value, 5);
        assert!(previous_note_on_string(&track, &order, 0, 0, 0, 1).is_none());
    }
}
