//! A beat and its notes: fret numbers, grace notes and effect annotations.

use super::chord::{CHORD_DIAGRAM_HEIGHT, draw_chord_diagram, has_diagram};
use super::effects::{
    ANNOTATION_TEXT_SIZE, draw_bend, draw_hammer_arc, draw_pick_stroke, draw_slide, draw_slide_in,
    draw_slide_out, draw_staccato_dot, draw_stroke_arrow, draw_tie_arc, draw_tremolo_bar,
    draw_tremolo_picking, draw_trill, draw_vibrato,
};
use super::layout::{
    GRACE_DIGIT_WIDTH, GRACE_GAP, NOTE_DIGIT_WIDTH, ROW_EFFECT_LINE, STRING_LINE_HEIGHT, Staff,
    grace_width,
};
use super::rhythm::{draw_rest, is_rest};
use crate::parser::song_parser::{
    Beat, BeatStrokeDirection, BendEffect, GraceEffect, HarmonicType, Note, NoteEffect, NoteType,
    SlapEffect, SlideType,
};
use crate::ui::utils::TablatureColors;
use iced::advanced::text::Shaping::Auto;
use iced::widget::canvas::{Frame, Text};
use iced::widget::text::Alignment;
use iced::{Color, Point, Renderer};

/// An effect annotation stacked above a beat, one per line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Annotation {
    Text(&'static str),
    /// A wavy line along the beat, like TuxGuitar's `paintVibrato`.
    Vibrato,
    /// "Tr" followed by a wavy line, like TuxGuitar's `paintTrill`.
    Trill,
    /// A small V, like TuxGuitar's `paintTremoloBar`.
    TremoloBar,
}

/// The effect annotations of a column, one per line above the staff, from
/// `beat_x`; the wavy lines run along `span`.
pub(super) fn draw_annotations(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    staff: Staff,
    beat_x: f32,
    span: f32,
    annotations: &[Annotation],
) {
    let x = beat_x - 3.0;
    for (line, annotation) in annotations.iter().enumerate() {
        let y = staff.rows.effects_y() + line as f32 * ROW_EFFECT_LINE;
        match *annotation {
            Annotation::Text(content) => frame.fill_text(Text {
                shaping: Auto,
                content: content.to_string(),
                color: colors.foreground,
                size: ANNOTATION_TEXT_SIZE.into(),
                position: Point::new(x, y),
                ..Text::default()
            }),
            Annotation::Vibrato => draw_vibrato(frame, colors, x, y, span),
            Annotation::Trill => draw_trill(frame, colors, x, y, span),
            Annotation::TremoloBar => draw_tremolo_bar(frame, colors, x, y),
        }
    }
}

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
pub(super) fn beat_annotations(beat: &Beat) -> Vec<Annotation> {
    let mut annotations: Vec<Annotation> = beat
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
    /// The row each of its notes is drawn on, when one differs from its
    /// string (a drum note moved off a row already used); empty otherwise.
    pub(super) rows: &'a [i8],
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
    for (index, note) in beat.notes.iter().enumerate() {
        let bend_movements = note.effect.bend.as_ref().map(BendEffect::movements);
        let bend = bend_movements
            .as_deref()
            .map(|movements| (movements, !multiple_bend_conflicts(beat, note, movements)));
        let row = slot.rows.get(index).copied().unwrap_or(note.string);
        draw_note(frame, colors, staff, slot, note, row, bend);
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
    row: i8,
    // the bend arrows, and whether they show their amplitude
    bend: Option<(&[i32], bool)>,
) {
    let beat_position_x = slot.x();
    let measure_start_y = staff.top;
    // note label (pushed down on the right string)
    let note_label = note_value(note);
    let note_label_len = note_label.chars().count();
    let local_beat_position_y = (f32::from(row) - 1.0) * STRING_LINE_HEIGHT;
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

    // a slide into the note comes from before it, and before its grace note
    if let Some(slide) = note.effect.slide.as_ref().filter(|slide| slide.is_into()) {
        let note_half = note_label_len as f32 * NOTE_DIGIT_WIDTH / 2.0;
        let end_x = note_position_x - note_half - grace_width(note) - GRACE_GAP;
        draw_slide_in(
            frame,
            colors,
            end_x,
            string_y,
            *slide == SlideType::IntoFromBelow,
        );
    }

    // like TuxGuitar's paintEffects, the bend arrows are exclusive with the
    // inline slide/hammer glyphs: they would collide in the same span
    if let Some((movements, show_bend_amplitude)) = bend {
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
        let slide = note.effect.slide.as_ref().filter(|slide| !slide.is_into());
        if let Some(slide) = slide {
            match slide {
                SlideType::OutDownwards | SlideType::OutUpWards => draw_slide_out(
                    frame,
                    colors,
                    note_position_x,
                    string_y,
                    *slide == SlideType::OutDownwards,
                ),
                SlideType::ShiftSlideTo | SlideType::LegatoSlideTo => draw_slide(
                    frame,
                    colors,
                    note_position_x,
                    string_y,
                    next_note.map(|(x, value)| (x + 3.0, value)),
                    note.value,
                ),
                // drawn before the note
                SlideType::IntoFromBelow | SlideType::IntoFromAbove => {}
            }
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
pub(super) fn above_note_effect_annotation(note_effect: &NoteEffect) -> Vec<Annotation> {
    let mut texts: Vec<&'static str> = vec![];
    if note_effect.accentuated_note {
        texts.push(">");
    }
    if note_effect.heavy_accentuated_note {
        texts.push("^");
    }
    if note_effect.palm_mute {
        texts.push("P.M");
    }
    if note_effect.let_ring {
        texts.push("L.R");
    }
    if note_effect.fade_in {
        texts.push("<");
    }
    if let Some(harmonic) = &note_effect.harmonic {
        match harmonic.kind {
            HarmonicType::Natural => texts.push("N.H"),
            HarmonicType::Artificial => texts.push("A.H"),
            HarmonicType::Tapped => texts.push("T.H"),
            HarmonicType::Pinch => texts.push("P.H"),
            HarmonicType::Semi => texts.push("S.H"),
        }
    }
    match note_effect.slap {
        SlapEffect::Tapping => texts.push("T"),
        SlapEffect::Slapping => texts.push("S"),
        SlapEffect::Popping => texts.push("P"),
        SlapEffect::None => {}
    }
    let mut annotations: Vec<Annotation> = texts.into_iter().map(Annotation::Text).collect();
    if note_effect.vibrato {
        annotations.push(Annotation::Vibrato);
    }
    if note_effect.trill.is_some() {
        annotations.push(Annotation::Trill);
    }
    if note_effect.tremolo_bar.is_some() {
        annotations.push(Annotation::TremoloBar);
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
