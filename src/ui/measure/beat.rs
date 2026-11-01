//! A beat and its notes: fret numbers, grace notes and effect annotations.

use super::chord::{CHORD_DIAGRAM_HEIGHT, draw_chord_diagram, has_diagram};
use super::effects::{
    draw_bend, draw_hammer_arc, draw_pick_stroke, draw_slide, draw_staccato_dot, draw_stroke_arrow,
    draw_tie_arc, draw_tremolo_picking,
};
use super::layout::{GRACE_DIGIT_WIDTH, GRACE_GAP, NOTE_DIGIT_WIDTH, STRING_LINE_HEIGHT, Staff};
use super::rhythm::{draw_rest, is_rest};
use crate::parser::song_parser::{
    Beat, BeatStrokeDirection, BendEffect, GraceEffect, HarmonicType, Note, NoteEffect, NoteType,
    SlapEffect,
};
use crate::ui::utils::TablatureColors;
use iced::advanced::text::Shaping::Auto;
use iced::widget::canvas::{Frame, Text};
use iced::widget::text::Alignment;
use iced::{Color, Point, Renderer};

const VIBRATO: &str = "\u{301C}\u{301C}"; // 〜〜 https://unicodeplus.com/U+301C

/// Position and fret of the next note on `string` after `beat_index`,
/// within the measure, like TuxGuitar's `getNextNote`. Beats without a note
/// on the string are skipped.
pub(super) fn next_note_on_string(
    beats: &[Beat],
    beat_positions: &[f32],
    beat_index: usize,
    string: i8,
) -> Option<(f32, i16)> {
    beats
        .iter()
        .zip(beat_positions)
        .skip(beat_index + 1)
        .find_map(|(beat, &x)| {
            beat.notes
                .iter()
                .find(|note| note.string == string)
                .map(|note| (x, note.value))
        })
}

/// Position of the note a tie hangs from: the closest earlier note on the
/// same string, like TuxGuitar's `getNoteForTie`. A rest before it breaks
/// the tie, leaving nothing to hang from.
pub(super) fn tied_from_note_x(
    beats: &[Beat],
    beat_positions: &[f32],
    beat_index: usize,
    string: i8,
) -> Option<f32> {
    for (beat, &x) in beats.iter().zip(beat_positions).take(beat_index).rev() {
        if beat.notes.is_empty() {
            return None;
        }
        if beat.notes.iter().any(|note| note.string == string) {
            return Some(x);
        }
    }
    None
}

/// Effect annotations stacked above a beat, deduplicated across its notes.
/// Shared by the row sizing and the drawing so both agree on the height.
pub(super) fn beat_annotations(beat: &Beat) -> Vec<&'static str> {
    let mut annotations: Vec<&'static str> = beat
        .notes
        .iter()
        .flat_map(|note| above_note_effect_annotation(&note.effect))
        .collect();
    annotations.sort_unstable();
    annotations.dedup();
    annotations
}

/// Label of a grace note: its fret, or a cross when it is dead.
pub(super) fn grace_label(grace: &GraceEffect) -> String {
    if grace.is_dead {
        "x".to_string()
    } else {
        grace.fret.to_string()
    }
}

/// Like TuxGuitar's `multipleBendConflicts`: with several bent notes in a
/// beat, only the lowest-string one keeps its amplitude label, and only
/// when all the bends start with the same movement.
pub(super) fn multiple_bend_conflicts(beat: &Beat, note: &Note, movements: &[i32]) -> bool {
    beat.notes.iter().any(|other| {
        other.effect.bend.as_ref().is_some_and(|other_bend| {
            if other.string < note.string {
                return true;
            }
            let other_movements = other_bend.movements();
            !other_movements.is_empty()
                && !movements.is_empty()
                && other_movements.first() != movements.first()
        })
    })
}

/// A beat being drawn, with the measure around it.
pub(super) struct BeatSlot<'a> {
    /// The measure's beats, to reach neighbouring notes.
    pub(super) beats: &'a [Beat],
    /// Where each of the measure's beats starts.
    pub(super) positions: &'a [f32],
    pub(super) index: usize,
    /// Room for the glyphs drawn after the note, up to the next beat's grace note.
    pub(super) span: f32,
    /// How much the measure was stretched from its natural width.
    pub(super) width_scale: f32,
    /// The color of its fret numbers: the accent when it has the focus.
    pub(super) color: Color,
}

impl BeatSlot<'_> {
    fn beat(&self) -> &Beat {
        &self.beats[self.index]
    }

    pub(super) fn x(&self) -> f32 {
        self.positions[self.index]
    }
}

pub(super) fn draw_beat(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    staff: Staff,
    slot: &BeatSlot,
) {
    let beat = slot.beat();
    let beat_position_x = slot.x();
    let rows = staff.rows;
    // Annotate chord effect
    if let Some(chord) = &beat.effect.chord {
        let mut name_y = rows.chord_y();
        if has_diagram(chord) {
            draw_chord_diagram(frame, colors, chord, beat_position_x + 3.0, name_y);
            name_y += CHORD_DIAGRAM_HEIGHT;
        }
        let note_effect_text = Text {
            shaping: Auto,
            content: chord.name.clone(),
            color: colors.foreground,
            size: 8.0.into(),
            position: Point::new(beat_position_x + 3.0, name_y),
            ..Text::default()
        };
        frame.fill_text(note_effect_text);
    }
    if is_rest(beat) {
        draw_rest(
            frame,
            colors,
            beat,
            beat_position_x + 2.0,
            staff.top,
            staff.height,
        );
    }
    if !beat.effect.stroke.is_empty() && !beat.notes.is_empty() {
        draw_stroke_arrow(frame, colors, beat, beat_position_x, staff.top);
    }
    if beat.effect.pick_stroke != BeatStrokeDirection::None {
        draw_pick_stroke(
            frame,
            colors,
            &beat.effect.pick_stroke,
            beat_position_x,
            rows.pick_stroke_y(),
        );
    }
    if beat.notes.iter().any(|n| n.effect.staccato) {
        draw_staccato_dot(frame, colors, beat, beat_position_x, staff.top);
    }
    if let Some(tremolo_picking) = beat
        .notes
        .iter()
        .find_map(|n| n.effect.tremolo_picking.as_ref())
    {
        draw_tremolo_picking(
            frame,
            colors,
            tremolo_picking,
            beat_position_x,
            staff.bottom(),
        );
    }

    // draw notes for beat
    for note in &beat.notes {
        let bend_movements = note.effect.bend.as_ref().map(BendEffect::movements);
        let show_bend_amplitude = bend_movements
            .as_deref()
            .is_some_and(|movements| !multiple_bend_conflicts(beat, note, movements));
        draw_note(
            frame,
            colors,
            staff,
            slot,
            note,
            bend_movements.as_deref(),
            show_bend_amplitude,
        );
    }

    // merge and display beat annotations (same position for all notes)
    let annotations = beat_annotations(beat);
    if !annotations.is_empty() {
        let merged_annotations = annotations.join("\n");
        let y_position = rows.effects_y();
        let note_effect_text = Text {
            shaping: Auto,
            content: merged_annotations,
            color: colors.foreground,
            size: 9.0.into(),
            position: Point::new(beat_position_x - 3.0, y_position),
            ..Text::default()
        };
        frame.fill_text(note_effect_text);
    }

    // user-authored text attached to the beat (e.g. "Verse", "fill")
    if !beat.text.is_empty() {
        let beat_text = Text {
            shaping: Auto,
            content: beat.text.clone(),
            color: colors.foreground,
            size: 8.0.into(),
            position: Point::new(beat_position_x + 3.0, rows.text_y()),
            ..Text::default()
        };
        frame.fill_text(beat_text);
    }
}

fn draw_note(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    staff: Staff,
    slot: &BeatSlot,
    note: &Note,
    bend_movements: Option<&[i32]>,
    show_bend_amplitude: bool,
) {
    let beat_position_x = slot.x();
    let measure_start_y = staff.top;
    // note label (pushed down on the right string)
    let note_label = note_value(note);
    let note_label_len = note_label.chars().count();
    let local_beat_position_y = (f32::from(note.string) - 1.0) * STRING_LINE_HEIGHT;
    // center the notes with more than one char
    let note_position_x = beat_position_x + 3.0 - note_label_len as f32 / 2.0;
    let note_position_y = measure_start_y + local_beat_position_y - 5.0;
    let string_y = measure_start_y + local_beat_position_y;

    // a tied note carries no fret of its own: it is drawn as an arc hanging
    // back to the note it continues, like TuxGuitar
    if note.kind == NoteType::Tie {
        let from_x = tied_from_note_x(slot.beats, slot.positions, slot.index, note.string)
            .map_or(note_position_x - STRING_LINE_HEIGHT * 2.0, |x| x + 3.0);
        draw_tie_arc(frame, slot.color, from_x, note_position_x, string_y);
    } else {
        let note_text = Text {
            shaping: Auto,
            content: note_label,
            color: slot.color,
            size: 10.0.into(),
            position: Point::new(note_position_x, note_position_y),
            align_x: Alignment::Center,
            ..Text::default()
        };
        frame.fill_text(note_text);
    }

    // small grace fret before the note, like TuxGuitar's paintEffects.
    // both glyphs are centred, so offset by their half widths to keep the
    // grace clear of the note whatever their digit counts
    if let Some(grace) = &note.effect.grace {
        let label = grace_label(grace);
        let note_half = note_label_len as f32 * NOTE_DIGIT_WIDTH / 2.0;
        let grace_half = label.chars().count() as f32 * GRACE_DIGIT_WIDTH / 2.0;
        let grace_text = Text {
            shaping: Auto,
            content: label,
            color: colors.foreground,
            size: 7.0.into(),
            position: Point::new(
                note_position_x - note_half - grace_half - GRACE_GAP,
                note_position_y + 2.0,
            ),
            align_x: Alignment::Center,
            ..Text::default()
        };
        frame.fill_text(grace_text);
    }

    // like TuxGuitar's paintEffects, the bend arrows are exclusive with the
    // inline slide/hammer glyphs: they would collide in the same span
    if let Some(movements) = bend_movements {
        draw_bend(
            frame,
            colors,
            staff,
            slot,
            Point::new(note_position_x, note_position_y),
            movements,
            show_bend_amplitude,
        );
    } else {
        // slides and hammers reach for the note they land on, like
        // TuxGuitar's paintSlide and paintHammer
        let next_note = next_note_on_string(slot.beats, slot.positions, slot.index, note.string);
        if note.effect.slide.is_some() {
            draw_slide(
                frame,
                colors,
                note_position_x,
                string_y,
                next_note.map(|(x, value)| (x + 3.0, value)),
                note.value,
            );
        } else if note.effect.hammer {
            draw_hammer_arc(
                frame,
                colors,
                note_position_x,
                note_position_y,
                next_note.map(|(x, _)| x + 3.0),
            );
        }
    }
}

// Similar to `https://www.tuxguitar.app/files/1.6.0/desktop/help/edit_effects.html`
pub(super) fn above_note_effect_annotation(note_effect: &NoteEffect) -> Vec<&'static str> {
    let mut annotations: Vec<&'static str> = vec![];
    if note_effect.accentuated_note {
        annotations.push(">");
    }
    if note_effect.heavy_accentuated_note {
        annotations.push("^");
    }
    if note_effect.palm_mute {
        annotations.push("P.M");
    }
    if note_effect.let_ring {
        annotations.push("L.R");
    }
    if note_effect.fade_in {
        annotations.push("<");
    }
    if let Some(harmonic) = &note_effect.harmonic {
        match harmonic.kind {
            HarmonicType::Natural => annotations.push("N.H"),
            HarmonicType::Artificial => annotations.push("A.H"),
            HarmonicType::Tapped => annotations.push("T.H"),
            HarmonicType::Pinch => annotations.push("P.H"),
            HarmonicType::Semi => annotations.push("S.H"),
        }
    }
    if note_effect.vibrato {
        annotations.push(VIBRATO);
    }
    if note_effect.trill.is_some() {
        annotations.push("Tr");
    }
    if note_effect.tremolo_bar.is_some() {
        annotations.push("T.B");
    }
    match note_effect.slap {
        SlapEffect::Tapping => annotations.push("T"),
        SlapEffect::Slapping => annotations.push("S"),
        SlapEffect::Popping => annotations.push("P"),
        SlapEffect::None => {}
    }
    annotations
}

pub(super) fn note_value(note: &Note) -> String {
    match note.kind {
        NoteType::Rest => {
            log::debug!("NoteType Rest");
            String::new()
        }
        NoteType::Normal => {
            if note.effect.ghost_note {
                format!("({})", note.value)
            } else {
                note.value.to_string()
            }
        }
        // a tie draws an arc back to its note instead of a label
        NoteType::Tie => String::new(),
        NoteType::Dead => "x".to_string(),
        NoteType::Unknown(i) => {
            log::warn!("NoteType Unknown({i})");
            String::new()
        }
    }
}
