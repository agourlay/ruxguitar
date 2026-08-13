//! Pure musical-effect and timing helpers for the MIDI builder.
//! These are stateless transforms (velocity, duration, triplet feel, strokes)
//! with no MIDI/event-emitting side effects.

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
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_duration_effect(
    track: &Track,
    playback_order: &[(usize, i64)],
    playback_index: usize,
    voice_id: usize,
    beat_id: usize,
    note: &Note,
    repeat_close: bool,
    tempo: u32,
    duration: u32,
) -> u32 {
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
