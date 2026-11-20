//! The columns of a measure: the moments its beats start, shared by its voices.
//!
//! Like TuxGuitar's beats, which hold a slot for each voice, a column gathers
//! the beats of every voice starting together, so both voices line up on one
//! timeline. The first voice is laid out as it always was, one column per beat;
//! the other voices only add the beats that sound, their rests left unshown.

use super::beat::{Annotation, beat_annotations};
use crate::parser::song_parser::{Beat, Measure};
use std::collections::HashMap;

/// The beats of every voice starting at the same tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Column {
    pub(super) start: u32,
    /// `(voice, beat)` indexes into the measure, the first voice first.
    pub(super) beats: Vec<(usize, usize)>,
}

impl Column {
    /// Whether a voice other than the first sounds in this column.
    pub(super) fn has_other_voices(&self) -> bool {
        self.beats.iter().any(|&(voice, _)| voice > 0)
    }
}

/// Whether a beat of `voice` is drawn: every beat of the first voice, and
/// only the beats that sound in the others.
pub(super) fn is_drawn(voice: usize, beat: &Beat) -> bool {
    voice == 0 || !beat.notes.is_empty()
}

/// The columns of a measure, in time order.
///
/// Each beat of the first voice opens its own column, even one sharing its
/// start with the beat before it, so a measure with a single voice keeps one
/// column per beat. The sounding beats of the other voices join the first
/// column starting with them, or open one of their own in time order.
pub(super) fn measure_columns(measure: &Measure) -> Vec<Column> {
    let mut columns: Vec<Column> = measure.voices.first().map_or_else(Vec::new, |voice| {
        voice
            .beats
            .iter()
            .enumerate()
            .map(|(index, beat)| Column {
                start: beat.start,
                beats: vec![(0, index)],
            })
            .collect()
    });
    for (voice_index, voice) in measure.voices.iter().enumerate().skip(1) {
        for (beat_index, beat) in voice.beats.iter().enumerate() {
            if !is_drawn(voice_index, beat) {
                continue;
            }
            if let Some(column) = columns.iter_mut().find(|c| c.start == beat.start) {
                column.beats.push((voice_index, beat_index));
            } else {
                let at = columns.partition_point(|c| c.start <= beat.start);
                columns.insert(
                    at,
                    Column {
                        start: beat.start,
                        beats: vec![(voice_index, beat_index)],
                    },
                );
            }
        }
    }
    columns
}

/// The column holding `tick`: the last one starting at or before it.
pub(super) fn column_at_tick(columns: &[Column], tick: u32) -> usize {
    columns
        .partition_point(|column| column.start <= tick)
        .saturating_sub(1)
}

/// The annotations of a column: those of its beats, each listed once.
pub(super) fn column_annotations(measure: &Measure, column: &Column) -> Vec<Annotation> {
    let mut annotations: Vec<Annotation> = column
        .beats
        .iter()
        .flat_map(|&(voice, beat)| beat_annotations(&measure.voices[voice].beats[beat]))
        .collect();
    annotations.sort_unstable();
    annotations.dedup();
    annotations
}

/// The beats a voice draws, with the column each one sits in.
pub(super) fn voice_beats<'a>(
    measure: &'a Measure,
    columns: &[Column],
    voice: usize,
) -> impl Iterator<Item = (usize, &'a Beat, usize)> {
    let column_of: Vec<Option<usize>> = (0..measure.voices[voice].beats.len())
        .map(|beat| {
            columns
                .iter()
                .position(|column| column.beats.contains(&(voice, beat)))
        })
        .collect();
    measure.voices[voice]
        .beats
        .iter()
        .enumerate()
        .filter_map(move |(index, beat)| column_of[index].map(|column| (index, beat, column)))
}

/// The rows the notes of a drum track's other voices are drawn on, where
/// they would land on a row already used in their column.
///
/// A drum tab's strings are only rows for the kit pieces, and files often
/// put the hands of the first voice and the feet of the second on the same
/// row at the same moment, which would print one number over the other.
/// Like TuxGuitar's `allocatePercussionNotesToStrings`, such a note moves to
/// the nearest free row, below first; with no row free it stays. Only the
/// beats with a moved note are listed, with the row of each of their notes.
pub(super) fn separated_drum_rows(
    measure: &Measure,
    columns: &[Column],
    string_count: usize,
) -> HashMap<(usize, usize), Vec<i8>> {
    let mut moved = HashMap::new();
    let string_count = i8::try_from(string_count).unwrap_or(i8::MAX);
    for column in columns {
        if !column.has_other_voices() {
            continue;
        }
        let beat_of = |&(voice, beat): &(usize, usize)| &measure.voices[voice].beats[beat];
        // the first voice keeps its rows
        let mut taken: Vec<i8> = column
            .beats
            .iter()
            .filter(|&&(voice, _)| voice == 0)
            .flat_map(|key| beat_of(key).notes.iter().map(|note| note.string))
            .collect();
        for &key in column.beats.iter().filter(|&&(voice, _)| voice > 0) {
            let notes = &beat_of(&key).notes;
            let mut rows = Vec::with_capacity(notes.len());
            let mut any_moved = false;
            for note in notes {
                let row = if taken.contains(&note.string) {
                    (1..string_count)
                        .flat_map(|distance| [note.string + distance, note.string - distance])
                        .find(|row| (1..=string_count).contains(row) && !taken.contains(row))
                        .unwrap_or(note.string)
                } else {
                    note.string
                };
                any_moved |= row != note.string;
                taken.push(row);
                rows.push(row);
            }
            if any_moved {
                moved.insert(key, rows);
            }
        }
    }
    moved
}
