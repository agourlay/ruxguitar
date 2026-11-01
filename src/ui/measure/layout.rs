//! Measure geometry: annotation rows, beat widths and measure height.

use super::beat::{beat_annotations, grace_label};
use super::chord::{
    CHORD_DIAGRAM_HEIGHT, CHORD_FIRST_FRET_SPACE, CHORD_STRING_SPACING, chord_diagram_width,
    has_diagram,
};
use super::rhythm::stem_beams;
use crate::parser::song_parser::{
    Beat, BeatStrokeDirection, Duration, Measure, MeasureHeader, QUARTER_TIME,
};

// Annotation rows are stacked above the staff, top to bottom, and each one
// is only allocated when something on the line uses it (like TuxGuitar's
// TGTrackSpacing). Tremolo picking slashes are drawn below the last string.
const ROW_ALT_ENDING: f32 = 13.0;
pub(super) const ROW_MARKER: f32 = 15.0;
const ROW_SIGNATURE: f32 = 11.0;
pub(super) const ROW_CHORD: f32 = 11.0;
const ROW_TUPLET: f32 = 12.0;
const ROW_EFFECT_LINE: f32 = 12.0;
const ROW_TEXT: f32 = 11.0;
const ROW_PICK_STROKE: f32 = 10.0;
// Lyrics sit under the staff, below the footer.
pub(super) const ROW_LYRIC: f32 = 11.0;
// Labels drawn from their beat rightwards - sung words, chord names, beat
// text - all share a size, and are wider than a beat often enough that the
// beat carrying one claims the room: roughly a character of that font, plus
// a gap before whatever follows.
pub(super) const LABEL_CHAR_WIDTH: f32 = 4.6;
pub(super) const LABEL_GAP: f32 = 3.0;
// One long label may not stretch its measure without limit.
pub(super) const LABEL_MAX_EXTRA: f32 = BEAT_LENGTH * 2.0;
// Always-present gap between the last annotation row and the staff: holds
// the measure number, the repeat count and the focus box edge.
const STAFF_HEADER: f32 = 16.0;
// Bends reach higher above the staff for their amplitude labels.
const STAFF_HEADER_WITH_BEND: f32 = 20.0;
// Gap below the last string, holding the focus box edge.
const STAFF_FOOTER: f32 = 10.0;
// Tremolo picking slashes hang further below the staff.
const STAFF_FOOTER_WITH_TREMOLO: f32 = 19.0;
// Stems and their beams hang further still.
const STAFF_FOOTER_WITH_STEMS: f32 = 30.0;

/// Height of each annotation row above the staff, zero when unused.
///
/// Computed per measure, then merged across every measure of a line so
/// their staves stay aligned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowSpacing {
    pub(super) alt_ending: f32,
    pub(super) marker: f32,
    pub(super) signature: f32,
    pub(super) chord: f32,
    pub(super) tuplet: f32,
    pub(super) effects: f32,
    pub(super) text: f32,
    pub(super) pick_stroke: f32,
    pub(super) staff_header: f32,
    pub(super) staff_footer: f32,
    pub(super) lyric: f32,
}

impl Default for RowSpacing {
    fn default() -> Self {
        Self {
            alt_ending: 0.0,
            marker: 0.0,
            signature: 0.0,
            chord: 0.0,
            tuplet: 0.0,
            effects: 0.0,
            text: 0.0,
            pick_stroke: 0.0,
            staff_header: STAFF_HEADER,
            staff_footer: STAFF_FOOTER,
            lyric: 0.0,
        }
    }
}

impl RowSpacing {
    /// The alternative-ending bracket sits at the very top.
    pub(super) const ALT_ENDING_Y: f32 = 0.0;

    /// Which rows a measure uses, and how tall they need to be.
    pub(super) fn for_measure(
        measure: &Measure,
        header: &MeasureHeader,
        has_tempo_label: bool,
        has_signature_label: bool,
        has_lyrics: bool,
    ) -> Self {
        let mut spacing = Self::default();
        if has_lyrics {
            spacing.lyric = ROW_LYRIC;
        }
        if header.repeat_alternative > 0 {
            spacing.alt_ending = ROW_ALT_ENDING;
        }
        if has_tempo_label || header.marker.is_some() {
            spacing.marker = ROW_MARKER;
        }
        if has_signature_label {
            spacing.signature = ROW_SIGNATURE;
        }
        let beats = &measure.voices[0].beats;
        if beats.iter().any(|beat| beat.effect.chord.is_some()) {
            spacing.chord = ROW_CHORD;
            // a fingering shown as a grid needs the room above the name
            if beats
                .iter()
                .filter_map(|beat| beat.effect.chord.as_ref())
                .any(has_diagram)
            {
                spacing.chord += CHORD_DIAGRAM_HEIGHT;
            }
        }
        if beats.iter().any(|beat| beat.duration.is_tuplet()) {
            spacing.tuplet = ROW_TUPLET;
        }
        if beats.iter().any(|beat| !beat.text.is_empty()) {
            spacing.text = ROW_TEXT;
        }
        if beats
            .iter()
            .any(|beat| beat.effect.pick_stroke != BeatStrokeDirection::None)
        {
            spacing.pick_stroke = ROW_PICK_STROKE;
        }
        if beats
            .iter()
            .flat_map(|beat| &beat.notes)
            .any(|note| note.effect.bend.is_some())
        {
            spacing.staff_header = STAFF_HEADER_WITH_BEND;
        }
        if beats
            .iter()
            .flat_map(|beat| &beat.notes)
            .any(|note| note.effect.tremolo_picking.is_some())
        {
            spacing.staff_footer = STAFF_FOOTER_WITH_TREMOLO;
        }
        if beats.iter().any(|beat| stem_beams(beat).is_some()) {
            spacing.staff_footer = spacing.staff_footer.max(STAFF_FOOTER_WITH_STEMS);
        }
        // the effect row grows with the tallest annotation stack of the measure
        let effect_lines = beats
            .iter()
            .map(|beat| beat_annotations(beat).len())
            .max()
            .unwrap_or(0);
        spacing.effects = ROW_EFFECT_LINE * effect_lines as f32;
        spacing
    }

    /// Keep the largest of each row, so a line fits every measure on it.
    pub const fn merge(&mut self, other: Self) {
        self.alt_ending = self.alt_ending.max(other.alt_ending);
        self.marker = self.marker.max(other.marker);
        self.signature = self.signature.max(other.signature);
        self.chord = self.chord.max(other.chord);
        self.tuplet = self.tuplet.max(other.tuplet);
        self.effects = self.effects.max(other.effects);
        self.text = self.text.max(other.text);
        self.pick_stroke = self.pick_stroke.max(other.pick_stroke);
        self.staff_header = self.staff_header.max(other.staff_header);
        self.staff_footer = self.staff_footer.max(other.staff_footer);
        self.lyric = self.lyric.max(other.lyric);
    }

    pub(super) const fn marker_y(self) -> f32 {
        self.alt_ending
    }

    pub(super) const fn signature_y(self) -> f32 {
        self.marker_y() + self.marker
    }

    pub(super) const fn chord_y(self) -> f32 {
        self.signature_y() + self.signature
    }

    pub(super) const fn tuplet_y(self) -> f32 {
        self.chord_y() + self.chord
    }

    pub(super) const fn effects_y(self) -> f32 {
        self.tuplet_y() + self.tuplet
    }

    pub(super) const fn text_y(self) -> f32 {
        self.effects_y() + self.effects
    }

    pub(super) const fn pick_stroke_y(self) -> f32 {
        self.text_y() + self.text
    }

    /// First tab line: below every annotation row.
    pub(super) const fn first_string_y(self) -> f32 {
        self.pick_stroke_y() + self.pick_stroke + self.staff_header
    }
}

// Distance between strings
pub(super) const STRING_LINE_HEIGHT: f32 = 13.0;

// Measure notes padding
pub(super) const MEASURE_NOTES_PADDING: f32 = 20.0;

// Width given to one of each duration, from whole note down, as TuxGuitar's
// style does. Shorter notes are not given proportionally less room, or a run
// of sixteenths would be unreadable; each step down simply narrows a little.
pub(super) const DURATION_WIDTHS: [f32; 6] = [30.0, 25.0, 21.0, 20.0, 19.0, 18.0];

// Width of a plain quarter note, the reference the layout is built on.
pub(super) const BEAT_LENGTH: f32 = DURATION_WIDTHS[2];
// No beat is narrower than the shortest note's width, which still leaves
// room for a two digit fret.
pub(super) const MIN_BEAT_WIDTH: f32 = DURATION_WIDTHS[DURATION_WIDTHS.len() - 1];

// Width of one bend/release arrow
pub(super) const BEND_ARROW_WIDTH: f32 = 10.0;

// Approximate digit advance of the fret and grace fonts, used to keep the
// grace note clear of the note it precedes.
pub(super) const NOTE_DIGIT_WIDTH: f32 = 5.5;
pub(super) const GRACE_DIGIT_WIDTH: f32 = 4.0;
// Gap kept on both sides of a grace note.
pub(super) const GRACE_GAP: f32 = 1.5;

pub(super) const HALF_BEAT_LENGTH: f32 = BEAT_LENGTH / 2.0 + 1.0;

// minimum measure width
pub(super) const MIN_MEASURE_WIDTH: f32 = 60.0;

/// Width of one note of this duration, from the style table.
fn duration_width(duration: &Duration) -> f32 {
    // the table runs whole, half, quarter, eighth... so the index is how
    // many times the duration halves the whole note
    let index = f32::from(duration.value.max(1)).log2().round() as usize;
    DURATION_WIDTHS[index.min(DURATION_WIDTHS.len() - 1)]
}

/// Pixels per quarter note that give `duration` the width its style asks for.
///
/// A measure is spaced by its shortest note, so a run of sixteenths opens the
/// measure out and the longer notes in it stretch to match.
pub(super) fn spacing_for_quarter(duration: &Duration) -> f32 {
    let time = duration.time();
    if time == 0 {
        return BEAT_LENGTH;
    }
    QUARTER_TIME as f32 / time as f32 * duration_width(duration)
}

/// The room a beat's duration alone gives it: proportional to its length,
/// and never narrower than the shortest note's width. A beat flagged empty
/// carries no time, so it takes no room at all.
pub(super) fn beat_base_width(beat: &Beat, quarter_spacing: f32) -> f32 {
    if beat.empty {
        return 0.0;
    }
    let proportional = beat.duration.time() as f32 / QUARTER_TIME as f32 * quarter_spacing;
    proportional.max(MIN_BEAT_WIDTH)
}

/// Room a label needs beyond the width its beat already has, capped so that
/// one long label cannot stretch its measure without limit.
pub(super) fn label_extra_width(label: &str, base: f32) -> f32 {
    if label.is_empty() {
        return 0.0;
    }
    let width = label.chars().count() as f32 * LABEL_CHAR_WIDTH + LABEL_GAP;
    (width - base).clamp(0.0, LABEL_MAX_EXTRA)
}

/// Room a beat needs for the syllable sung on it.
///
/// Only claimed when the next beat sings too: with nothing beside it, a
/// long word may lean into the space that follows.
pub(super) fn lyric_extra_width(lyrics: &[String], beat_index: usize, base: f32) -> f32 {
    let next_sings = lyrics
        .get(beat_index + 1)
        .is_some_and(|next| !next.is_empty());
    if !next_sings {
        return 0.0;
    }
    label_extra_width(lyrics.get(beat_index).map_or("", String::as_str), base)
}

/// Total canvas height: annotation rows, the staff, and the footer.
pub(super) fn measure_height(rows: RowSpacing, string_count: usize) -> f32 {
    rows.first_string_y()
        + STRING_LINE_HEIGHT * (string_count - 1) as f32
        + rows.staff_footer
        + rows.lyric
}

/// Natural width of a beat: base length, room for bend arrows (like
/// TuxGuitar's `getEffectWidth`), and room for the grace note that the
/// next beat draws in the gap before it.
pub(super) fn beat_natural_width(
    beat: &Beat,
    next_beat: Option<&Beat>,
    quarter_spacing: f32,
) -> f32 {
    let base = beat_base_width(beat, quarter_spacing);
    // a beat flagged empty takes no time, so it takes no room either
    if base <= 0.0 {
        return 0.0;
    }
    let bend_extra = beat
        .notes
        .iter()
        .filter_map(|n| n.effect.bend.as_ref())
        .map(|b| b.movements().len() as f32 * BEND_ARROW_WIDTH)
        .fold(0.0, f32::max);
    let grace_extra = next_beat.map_or(0.0, grace_gap_width);
    // a diagram is wider than a beat, so it claims the room it needs
    let diagram_extra = beat
        .effect
        .chord
        .as_ref()
        .filter(|chord| has_diagram(chord))
        .map_or(0.0, |chord| {
            // the grid, the first-fret number on its left, and a gap after
            let needed = chord_diagram_width(chord) + CHORD_FIRST_FRET_SPACE + CHORD_STRING_SPACING;
            (needed - base).max(0.0)
        });
    // the name sits under the diagram, so the wider of the two governs, and
    // only where the next beat names a chord of its own to run into
    let name_extra = next_beat
        .and_then(|next| next.effect.chord.as_ref())
        .filter(|next| !next.name.is_empty())
        .and(beat.effect.chord.as_ref())
        .map_or(0.0, |chord| label_extra_width(&chord.name, base));
    let chord_extra = diagram_extra.max(name_extra);
    // beat text runs the same risk as a chord name
    let text_extra = next_beat
        .filter(|next| !next.text.is_empty())
        .map_or(0.0, |_| label_extra_width(&beat.text, base));
    base + bend_extra + grace_extra + chord_extra + text_extra
}

/// Room the grace notes of a beat need in the gap before it.
pub(super) fn grace_gap_width(beat: &Beat) -> f32 {
    beat.notes
        .iter()
        .filter_map(|note| note.effect.grace.as_ref())
        .map(|grace| {
            grace_label(grace).chars().count() as f32 * GRACE_DIGIT_WIDTH + GRACE_GAP * 2.0
        })
        .fold(0.0, f32::max)
}
