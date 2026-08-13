use crate::parser::song_parser::{
    Beat, BeatStrokeDirection, BendEffect, HarmonicType, Note, NoteEffect, NoteType, SlapEffect,
    SlideType, Song, TimeSignature, TremoloPickingEffect,
};
use crate::ui::application::Message;
use iced::advanced::mouse;
use iced::advanced::text::Shaping::Auto;
use iced::mouse::{Cursor, Interaction};
use iced::widget::canvas::{Cache, Event, Frame, Geometry, LineDash, Path, Stroke, Text};
use iced::widget::text::Alignment;
use iced::widget::{Action, Canvas, canvas};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use std::rc::Rc;

// Unicode symbols for musical notation
const TEMPO_SIGN: char = '\u{1D15F}'; // 𝅗𝅥 https://unicodeplus.com/U+1D15F
const VIBRATO: &str = "\u{301C}\u{301C}"; // 〜〜 https://unicodeplus.com/U+301C
const HAMMER_ON: char = '\u{25E0}'; // ◠ https://unicodeplus.com/U+25E0
const HORIZONTAL_BAR: char = '\u{2015}'; // ― https://unicodeplus.com/U+2015
const SHIFT_SLIDE: char = '\u{27CD}'; // ⟍ https://unicodeplus.com/U+27CD
const LEGATO_SLIDE: char = '\u{27CB}'; // ⟋ https://unicodeplus.com/U+27CB
const TIE: char = '\u{2323}'; // ⌣ https://unicodeplus.com/U+2323

// Drawing constants

// Vertical layout above the staff (y grows downward):
//   y=3   MEASURE_ANNOTATION_Y   measure number / marker title
//   y=15  CHORD_ANNOTATION_Y     chord name
//   y=27  NOTE_EFFECT_ANNOTATION_Y  vibrato / hammer / slide labels
//   y=38  BEAT_TEXT_ANNOTATION_Y beat.text ("Verse", "fill", ...)
//   y=47  PICK_STROKE_Y          pick stroke direction symbols
//   y=60  FIRST_STRING_Y         first tab line (leaves room for the
//                                focus box to not sit on the first string)
// Tremolo picking slashes are drawn below the last string.
const MEASURE_ANNOTATION_Y: f32 = 3.0;
const CHORD_ANNOTATION_Y: f32 = 15.0;
const NOTE_EFFECT_ANNOTATION_Y: f32 = 27.0;
const BEAT_TEXT_ANNOTATION_Y: f32 = 38.0;
const PICK_STROKE_Y: f32 = 47.0;
const FIRST_STRING_Y: f32 = 60.0;

// Space below the last string (just enough for focus box clearance).
const BOTTOM_PADDING: f32 = 16.0;

// Distance between strings
const STRING_LINE_HEIGHT: f32 = 13.0;

// Measure notes padding
const MEASURE_NOTES_PADDING: f32 = 20.0;

// Length of a beat
const BEAT_LENGTH: f32 = 24.0;

// Width of one bend/release arrow
const BEND_ARROW_WIDTH: f32 = 10.0;

const HALF_BEAT_LENGTH: f32 = BEAT_LENGTH / 2.0 + 1.0;

// minimum measure width
const MIN_MEASURE_WIDTH: f32 = 60.0;

#[derive(Debug)]
pub struct CanvasMeasure {
    pub measure_id: usize,
    track_id: usize,
    song: Rc<Song>,
    is_focused: bool,
    focused_beat: usize,
    canvas_cache: Cache,
    measure_len: f32,
    // natural width of each beat and their sum (immutable song data,
    // computed once instead of on every redraw)
    beat_widths: Vec<f32>,
    natural_beats_len: f32,
    pub total_measure_len: f32,
    pub vertical_measure_height: f32,
    has_time_signature: bool,
    pub is_first_on_line: bool,
}

impl CanvasMeasure {
    pub fn new(
        measure_id: usize,
        track_id: usize,
        song: Rc<Song>,
        focused: bool,
        has_time_signature: bool,
    ) -> Self {
        let track = &song.tracks[track_id];
        let measure = &track.measures[measure_id];
        let measure_header = &song.measure_headers[measure_id];
        let beat_widths: Vec<f32> = measure.voices[0]
            .beats
            .iter()
            .map(beat_natural_width)
            .collect();
        let natural_beats_len: f32 = beat_widths.iter().sum();
        let measure_len = MIN_MEASURE_WIDTH.max(natural_beats_len);
        // total length of measure (padding on both sides)
        let mut total_measure_len = measure_len + MEASURE_NOTES_PADDING * 2.0;
        // extra space for time signature
        if has_time_signature {
            total_measure_len += BEAT_LENGTH;
        }
        // extra space for repeat open bar with dots
        if measure_header.repeat_open {
            total_measure_len += BEAT_LENGTH + HALF_BEAT_LENGTH;
        }
        // extra space for repeat close bar with dots
        if measure_header.repeat_close > 0 {
            total_measure_len += BEAT_LENGTH + HALF_BEAT_LENGTH;
        }
        let string_count = track.strings.len();
        // total height of measure (same for all measures in track)
        let vertical_measure_height = STRING_LINE_HEIGHT * (string_count - 1) as f32;
        let vertical_measure_height = vertical_measure_height + FIRST_STRING_Y + BOTTOM_PADDING;
        Self {
            measure_id,
            track_id,
            song,
            is_focused: focused,
            focused_beat: 0,
            canvas_cache: Cache::default(),
            measure_len,
            beat_widths,
            natural_beats_len,
            total_measure_len,
            vertical_measure_height,
            has_time_signature,
            is_first_on_line: false,
        }
    }

    pub const fn set_first_on_line(&mut self, value: bool) {
        self.is_first_on_line = value;
    }

    pub fn view(&self) -> Element<'_, Message> {
        let canvas = Canvas::new(self)
            .height(self.vertical_measure_height)
            .width(Length::Fixed(self.total_measure_len));
        canvas.into()
    }

    /// View with FillPortion width for stretching to fill the row.
    /// The portion is proportional to the natural width of the measure.
    pub fn view_fill(&self) -> Element<'_, Message> {
        // Use total_measure_len as the portion weight (rounded to u16)
        let portion = (self.total_measure_len.round() as u16).max(1);
        let canvas = Canvas::new(self)
            .height(self.vertical_measure_height)
            .width(Length::FillPortion(portion));
        canvas.into()
    }

    /// The fixed overhead width (padding, time signature, repeats) that doesn't scale with beats.
    fn overhead_width(&self) -> f32 {
        self.total_measure_len - self.measure_len
    }

    pub fn toggle_focused(&mut self) {
        // reset focus state
        self.is_focused = !self.is_focused;
        self.focused_beat = 0;
        // clear cache
        self.canvas_cache.clear();
    }

    pub fn focus_beat(&mut self, beat_id: usize) {
        if self.focused_beat != beat_id {
            self.focused_beat = beat_id;
            self.canvas_cache.clear();
        }
    }

    pub fn clear_canvas_cache(&self) {
        self.canvas_cache.clear();
    }

    /// The beat under the given x position, mirroring the layout used by
    /// `draw` (clicks in the leading padding select the first beat, clicks
    /// past the last beat the last one).
    fn beat_at_x(&self, x: f32, actual_width: f32) -> usize {
        let measure_header = &self.song.measure_headers[self.measure_id];
        let actual_measure_len = actual_width - self.overhead_width();
        let width_scale = if self.natural_beats_len > 0.0 {
            actual_measure_len / self.natural_beats_len
        } else {
            1.0
        };
        let mut beat_x = 0.0;
        if self.has_time_signature {
            beat_x += BEAT_LENGTH;
        }
        if measure_header.repeat_open {
            beat_x += BEAT_LENGTH;
        }
        beat_x += MEASURE_NOTES_PADDING;
        for (beat_id, width) in self.beat_widths.iter().enumerate() {
            beat_x += width * width_scale;
            if x < beat_x {
                return beat_id;
            }
        }
        self.beat_widths.len().saturating_sub(1)
    }
}

#[derive(Debug, Default)]
pub enum MeasureInteraction {
    #[default]
    None,
    Clicked,
}

impl canvas::Program<Message> for CanvasMeasure {
    type State = MeasureInteraction;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Option<Action<Message>> {
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event
            && let Some(cursor_position) = cursor.position_in(bounds)
        {
            let beat_id = self.beat_at_x(cursor_position.x, bounds.width);
            log::info!("Clicked on measure {} beat {beat_id}", self.measure_id);
            *state = MeasureInteraction::Clicked;
            return Some(Action::publish(Message::FocusMeasure(self.measure_id, beat_id)));
        }
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        // the cache will not redraw its geometry unless the dimensions of its layer change, or it is explicitly cleared.
        let tab = self.canvas_cache.draw(renderer, bounds.size(), |frame| {
            log::debug!("Re-drawing measure {}", self.measure_id);
            let track = &self.song.tracks[self.track_id];
            let strings = &track.strings;
            let string_count = strings.len();

            // use actual allocated width (may be larger than total_measure_len due to FillPortion)
            let actual_width = frame.width();
            // scale beat area: extra width goes to beat spacing
            let actual_measure_len = actual_width - self.overhead_width();

            // distance between lines of measures
            let vertical_measure_height = STRING_LINE_HEIGHT * (string_count - 1) as f32;

            // Positive x-values extend to the right, and positive y-values extend downwards.
            let measure_start_x = 0.0;
            let measure_start_y = FIRST_STRING_Y;

            // colors
            let color_gray = crate::ui::utils::COLOR_GRAY;
            let color_dark_red = crate::ui::utils::COLOR_DARK_RED;

            // draw focused box
            if self.is_focused {
                draw_focused_box(
                    frame,
                    actual_width,
                    vertical_measure_height,
                    measure_start_x,
                    measure_start_y,
                );
            }

            // draw string lines first (apply rest on top)
            for (string_id, _fret) in strings.iter().enumerate() {
                // down position
                let local_start_y = string_id as f32 * STRING_LINE_HEIGHT;
                // add 1 to x to avoid overlapping with vertical line
                let start_point =
                    Point::new(measure_start_x + 1.0, measure_start_y + local_start_y);
                // draw at the same y until end of container
                let end_point = Point::new(
                    measure_start_x + actual_width,
                    measure_start_y + local_start_y,
                );
                let line = Path::line(start_point, end_point);
                let stroke = Stroke::default().with_width(0.8).with_color(color_gray);
                frame.stroke(&line, stroke);
            }

            // measure headers
            let measure_header = &self.song.measure_headers[self.measure_id];
            let next_measure_header = &self.song.measure_headers.get(self.measure_id + 1);
            let previous_measure_header = if self.measure_id > 0 {
                Some(&self.song.measure_headers[self.measure_id - 1])
            } else {
                None
            };

            // display open measure bar
            if measure_header.repeat_open {
                draw_open_repeat(
                    frame,
                    measure_start_x,
                    measure_start_y,
                    vertical_measure_height,
                );
            } else if self.measure_id == 0 {
                draw_open_section(
                    frame,
                    measure_start_x,
                    measure_start_y,
                    vertical_measure_height,
                );
            } else {
                // draw first vertical line only for the first measure on a row
                // otherwise it doubles with the end line of the previous measure
                if self.is_first_on_line {
                    draw_measure_vertical_line(
                        frame,
                        vertical_measure_height,
                        measure_start_x,
                        measure_start_y,
                    );
                }
            }

            // display time signature (if first measure OR if it changed)
            if self.has_time_signature {
                draw_time_signature(
                    frame,
                    &measure_header.time_signature,
                    measure_start_x,
                    string_count,
                    measure_header.repeat_open, // need to offset if repeat dots present
                );
            }

            // capture tempo label len to adjust next annotations
            let mut tempo_label_len = 0;
            // display measure tempo (if first measure OR if it changed)
            if self.measure_id == 0
                || measure_header.tempo != previous_measure_header.unwrap().tempo
            {
                let tempo_sign = TEMPO_SIGN;
                let tempo_label = format!("{} = {}", tempo_sign, measure_header.tempo.value);
                tempo_label_len = tempo_label.chars().count() * 10;
                let tempo_text = Text {
                    shaping: Auto,
                    content: tempo_label,
                    color: Color::WHITE,
                    size: 11.0.into(),
                    position: Point::new(measure_start_x, MEASURE_ANNOTATION_Y),
                    ..Text::default()
                };
                frame.fill_text(tempo_text);
            }

            // marker annotation
            if let Some(marker) = &measure_header.marker {
                // measure marker label
                let marker_text = Text {
                    shaping: Auto,
                    content: marker.title.clone(),
                    color: color_dark_red,
                    size: 10.0.into(),
                    position: Point::new(
                        measure_start_x + MEASURE_NOTES_PADDING + tempo_label_len as f32,
                        MEASURE_ANNOTATION_Y,
                    ),
                    ..Text::default()
                };
                frame.fill_text(marker_text);
            }

            // measure count label
            let measure_count_text = Text {
                shaping: Auto,
                content: format!("{}", self.measure_id + 1),
                color: color_dark_red,
                size: 10.0.into(),
                position: Point::new(measure_start_x, FIRST_STRING_Y - 15.0),
                ..Text::default()
            };
            frame.fill_text(measure_count_text);

            // alternative ending bracket (e.g., "1.", "2.", "1.2.")
            if measure_header.repeat_alternative > 0 {
                draw_alternative_ending(
                    frame,
                    measure_header.repeat_alternative,
                    measure_start_x,
                    actual_width,
                );
            }

            // add notes on top of strings
            let measure = &track.measures[self.measure_id];
            // TODO draw second voice if present (audio playback already handles all voices)
            let beats = &measure.voices[0].beats;
            let beats_len = beats.len();
            log::debug!("{beats_len} beats");
            let mut beat_start = measure_start_x;
            if self.has_time_signature {
                beat_start += BEAT_LENGTH;
            }
            if measure_header.repeat_open {
                beat_start += BEAT_LENGTH;
            }
            // beats with multi-movement bends get extra width for the arrows,
            // like TuxGuitar's getEffectWidth
            let width_scale = if self.natural_beats_len > 0.0 {
                actual_measure_len / self.natural_beats_len
            } else {
                1.0
            };
            let mut beat_position_x = beat_start + MEASURE_NOTES_PADDING;
            for (b_id, beat) in beats.iter().enumerate() {
                // pick color if beat under focus
                let beat_color = if self.is_focused && b_id == self.focused_beat {
                    color_dark_red
                } else {
                    Color::WHITE
                };
                let beat_width = self.beat_widths[b_id] * width_scale;
                // draw beat
                draw_beat(
                    frame,
                    beat_position_x,
                    beat_width,
                    width_scale,
                    measure_start_y,
                    vertical_measure_height,
                    beat,
                    beat_color,
                );
                beat_position_x += beat_width;
            }

            // draw close measure
            if measure_header.repeat_close > 0 {
                draw_close_repeat(
                    frame,
                    measure_start_x + actual_width,
                    measure_start_y,
                    vertical_measure_height,
                    measure_header.repeat_close,
                );
            } else if next_measure_header.is_none() {
                draw_end_section(
                    frame,
                    measure_start_x + actual_width,
                    measure_start_y,
                    vertical_measure_height,
                );
            } else {
                // vertical measure end
                draw_measure_vertical_line(
                    frame,
                    vertical_measure_height,
                    measure_start_x + actual_width, // end of measure
                    measure_start_y,
                );
            }
        });

        vec![tab]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: Cursor,
    ) -> Interaction {
        Interaction::default()
    }
}

fn draw_focused_box(
    frame: &mut Frame<Renderer>,
    total_measure_len: f32,
    vertical_measure_height: f32,
    measure_start_x: f32,
    measure_start_y: f32,
) {
    let padding = 8.0;

    let focused_box = Rectangle {
        x: measure_start_x + padding,
        y: measure_start_y - padding,
        width: total_measure_len - padding * 2.0,
        height: vertical_measure_height + padding * 2.0,
    };

    let Rectangle {
        x,
        y,
        width,
        height,
    } = focused_box;

    let top_left = Point::new(x, y);
    let rectangle_size = Size::new(width, height);
    let stroke = Stroke::default().with_width(1.0).with_color(Color::WHITE);
    frame.stroke_rectangle(top_left, rectangle_size, stroke);
}

fn draw_measure_vertical_line(
    frame: &mut Frame<Renderer>,
    vertical_measure_height: f32,
    measure_start_x: f32,
    measure_start_y: f32,
) {
    let start_point = Point::new(measure_start_x, measure_start_y);
    let end_point = Point::new(measure_start_x, measure_start_y + vertical_measure_height);
    let vertical_line = Path::line(start_point, end_point);
    let stroke = Stroke::default().with_width(1.5).with_color(Color::WHITE);
    frame.stroke(&vertical_line, stroke);
}

/// Natural width of a beat: base length plus room for bend arrows
/// (like TuxGuitar's `getEffectWidth`).
fn beat_natural_width(beat: &Beat) -> f32 {
    let bend_extra = beat
        .notes
        .iter()
        .filter_map(|n| n.effect.bend.as_ref())
        .map(|b| b.movements().len() as f32 * BEND_ARROW_WIDTH)
        .fold(0.0, f32::max);
    BEAT_LENGTH + bend_extra
}

/// Like TuxGuitar's `multipleBendConflicts`: with several bent notes in a
/// beat, only the lowest-string one keeps its amplitude label, and only
/// when all the bends start with the same movement.
fn multiple_bend_conflicts(beat: &Beat, note: &Note, movements: &[i32]) -> bool {
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

#[allow(clippy::too_many_arguments)]
fn draw_beat(
    frame: &mut Frame<Renderer>,
    beat_position_x: f32,
    width_per_beat: f32,
    width_scale: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
    beat: &Beat,
    beat_color: Color,
) {
    // Annotate chord effect
    if let Some(chord) = &beat.effect.chord {
        let note_effect_text = Text {
            shaping: Auto,
            content: chord.name.clone(),
            color: Color::WHITE,
            size: 8.0.into(),
            position: Point::new(beat_position_x + 3.0, CHORD_ANNOTATION_Y),
            ..Text::default()
        };
        frame.fill_text(note_effect_text);
    }
    if !beat.effect.stroke.is_empty() && !beat.notes.is_empty() {
        draw_stroke_arrow(frame, beat, beat_position_x, measure_start_y);
    }
    if beat.effect.pick_stroke != BeatStrokeDirection::None {
        draw_pick_stroke(frame, &beat.effect.pick_stroke, beat_position_x);
    }
    if beat.notes.iter().any(|n| n.effect.staccato) {
        draw_staccato_dot(frame, beat, beat_position_x, measure_start_y);
    }
    if let Some(tremolo_picking) = beat
        .notes
        .iter()
        .find_map(|n| n.effect.tremolo_picking.as_ref())
    {
        draw_tremolo_picking(
            frame,
            tremolo_picking,
            beat_position_x,
            measure_start_y + vertical_measure_height,
        );
    }

    // Annotate note effect above (same position for all notes)
    let mut beat_annotations = Vec::new();

    // draw notes for beat
    for note in &beat.notes {
        beat_annotations.extend(above_note_effect_annotation(&note.effect));
        let bend_movements = note.effect.bend.as_ref().map(BendEffect::movements);
        let show_bend_amplitude = bend_movements
            .as_deref()
            .is_some_and(|movements| !multiple_bend_conflicts(beat, note, movements));
        draw_note(
            frame,
            measure_start_y,
            beat_position_x,
            width_per_beat,
            width_scale,
            note,
            beat_color,
            bend_movements.as_deref(),
            show_bend_amplitude,
        );
    }

    // merge and display beat annotations
    if !beat_annotations.is_empty() {
        beat_annotations.sort_unstable();
        beat_annotations.dedup();
        let merged_annotations = beat_annotations.join("\n");
        let y_position = NOTE_EFFECT_ANNOTATION_Y - 4.0 * (beat_annotations.len() - 1) as f32;
        let note_effect_text = Text {
            shaping: Auto,
            content: merged_annotations,
            color: Color::WHITE,
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
            color: Color::WHITE,
            size: 8.0.into(),
            position: Point::new(beat_position_x + 3.0, BEAT_TEXT_ANNOTATION_Y),
            ..Text::default()
        };
        frame.fill_text(beat_text);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_note(
    frame: &mut Frame<Renderer>,
    measure_start_y: f32,
    beat_position_x: f32,
    width_per_beat: f32,
    width_scale: f32,
    note: &Note,
    beat_color: Color,
    bend_movements: Option<&[i32]>,
    show_bend_amplitude: bool,
) {
    // note label (pushed down on the right string)
    let note_label = note_value(note);
    let local_beat_position_y = (f32::from(note.string) - 1.0) * STRING_LINE_HEIGHT;
    // center the notes with more than one char
    let note_position_x = beat_position_x + 3.0 - note_label.chars().count() as f32 / 2.0;
    let note_position_y = measure_start_y + local_beat_position_y - 5.0;
    let note_text = Text {
        shaping: Auto,
        content: note_label,
        color: beat_color,
        size: 10.0.into(),
        position: Point::new(note_position_x, note_position_y),
        align_x: Alignment::Center,
        ..Text::default()
    };
    frame.fill_text(note_text);

    // small grace fret before the note, like TuxGuitar's paintEffects
    if let Some(grace) = &note.effect.grace {
        let grace_label = if grace.is_dead {
            "x".to_string()
        } else {
            grace.fret.to_string()
        };
        let grace_text = Text {
            shaping: Auto,
            content: grace_label,
            color: Color::WHITE,
            size: 7.0.into(),
            position: Point::new(note_position_x - 6.0, note_position_y + 2.0),
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
            movements,
            note_position_x,
            note_position_y,
            beat_position_x,
            width_per_beat,
            width_scale,
            measure_start_y,
            show_bend_amplitude,
        );
    } else {
        // Annotate some effects on the string after the note
        let inlined_annotation_width = 10.0;
        let inlined_annotation_label = inlined_note_effect_annotation(&note.effect);
        // note_x + half of inter-beat space - half of annotation width
        let annotation_position_x =
            note_position_x + width_per_beat / 2.0 - inlined_annotation_width / 2.0;
        let note_effect_text = Text {
            shaping: Auto,
            content: inlined_annotation_label,
            color: Color::WHITE,
            size: inlined_annotation_width.into(),
            position: Point::new(annotation_position_x, note_position_y),
            ..Text::default()
        };
        frame.fill_text(note_effect_text);
    }
}

// Amplitude labels in tones, indexed by bend value (half-semitone units)
const BEND_AMPLITUDES: [&str; 13] = [
    "", "¼", "½", "¾", "1", "1¼", "1½", "1¾", "2", "2¼", "2½", "2¾", "3",
];

/// Draw a bend like TuxGuitar's `paintBend`: one curved arrow per movement
/// rising to (or releasing from) a band above the staff, with the reached
/// amplitude in tones next to the arrow tip. A bend without movements is a
/// held bend, drawn as a dashed line at band height across the beat.
#[allow(clippy::too_many_arguments)]
fn draw_bend(
    frame: &mut Frame<Renderer>,
    movements: &[i32],
    note_position_x: f32,
    note_position_y: f32,
    beat_position_x: f32,
    width_per_beat: f32,
    width_scale: f32,
    measure_start_y: f32,
    show_amplitude: bool,
) {
    let stroke = Stroke::default().with_width(0.8).with_color(Color::WHITE);
    let arrow_size = 2.5;
    // shrink the arrows with the beat when the measure is compressed, so
    // they stay within the width reserved by beat_natural_width
    let arrow_width = BEND_ARROW_WIDTH * width_scale.min(1.0);
    // vertical extent: from the note's fret number up to a band above the staff
    let y_low = note_position_y + 4.0;
    let y_high = measure_start_y - 8.0;
    let amplitude_y = measure_start_y - 18.0;

    if movements.is_empty() {
        // held bend: dashed line at band height until the end of the beat
        let x_start = note_position_x - 4.0;
        let x_end = (beat_position_x + width_per_beat - 6.0).max(x_start + 5.0);
        let dashed = Stroke {
            line_dash: LineDash {
                segments: &[2.5, 2.5],
                offset: 0,
            },
            ..stroke
        };
        frame.stroke(
            &Path::line(Point::new(x_start, y_high), Point::new(x_end, y_high)),
            dashed,
        );
        return;
    }

    let mut x0 = note_position_x + 7.0;
    let mut first_movement = true;
    for &movement in movements {
        let release = movement < 0;
        let (y0, y1, direction) = if release {
            (y_high, y_low, -1.0)
        } else {
            (y_low, y_high, 1.0)
        };
        let x1 = x0 + arrow_width;

        // pre-bend: the note starts already bent, marked by a vertical
        // line with an upward arrowhead before the release
        if first_movement && release {
            frame.stroke(&Path::line(Point::new(x0, y0), Point::new(x0, y1)), stroke);
            frame.stroke(
                &Path::line(Point::new(x0, y0), Point::new(x0 - 2.0, y0 + 2.0)),
                stroke,
            );
            frame.stroke(
                &Path::line(Point::new(x0, y0), Point::new(x0 + 2.0, y0 + 2.0)),
                stroke,
            );
        }

        // curved arrow body
        let body = Path::new(|p| {
            p.move_to(Point::new(x0, y0));
            p.line_to(Point::new(x0 + 1.0, y0));
            p.bezier_curve_to(
                Point::new(x0 + 1.0, y0),
                Point::new(x1, y0),
                Point::new(x1, y1),
            );
        });
        frame.stroke(&body, stroke);

        // arrowhead
        let tip = Point::new(x1, y1);
        frame.stroke(
            &Path::line(
                tip,
                Point::new(x1 - arrow_size, y1 + arrow_size * direction),
            ),
            stroke,
        );
        frame.stroke(
            &Path::line(
                tip,
                Point::new(x1 + arrow_size, y1 + arrow_size * direction),
            ),
            stroke,
        );

        // amplitude reached by the movement (releases only label the pre-bend)
        if show_amplitude && (!release || first_movement) {
            let mut x_amplitude = if release { x0 } else { x1 };
            let amplitude = movement.unsigned_abs() as usize;
            if !amplitude.is_multiple_of(4) {
                // longer label (fraction), shift left to stay near the tip
                x_amplitude -= 4.0;
            }
            if let Some(label) = BEND_AMPLITUDES.get(amplitude) {
                let amplitude_text = Text {
                    shaping: Auto,
                    content: (*label).to_string(),
                    color: Color::WHITE,
                    size: 8.0.into(),
                    position: Point::new(x_amplitude, amplitude_y),
                    ..Text::default()
                };
                frame.fill_text(amplitude_text);
            }
        }

        first_movement = false;
        x0 = x1;
    }
}

fn draw_open_section(
    frame: &mut Frame<Renderer>,
    measure_start_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    let position_x = measure_start_x;

    // draw first thick one
    let start_point = Point::new(position_x, measure_start_y);
    let end_point = Point::new(position_x, measure_start_y + vertical_measure_height);
    let tick_vertical_line = Path::line(start_point, end_point);
    let stroke = Stroke::default().with_width(4.0).with_color(Color::WHITE);
    frame.stroke(&tick_vertical_line, stroke);

    // then thin one
    draw_measure_vertical_line(
        frame,
        vertical_measure_height,
        measure_start_x + 6.0,
        measure_start_y,
    );
}

fn draw_open_repeat(
    frame: &mut Frame<Renderer>,
    measure_start_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    draw_open_section(
        frame,
        measure_start_x,
        measure_start_y,
        vertical_measure_height,
    );
    // draw repeat dots
    draw_repeat_dots(
        frame,
        measure_start_x + HALF_BEAT_LENGTH,
        measure_start_y,
        vertical_measure_height,
    );
}

fn draw_close_repeat(
    frame: &mut Frame<Renderer>,
    measure_end_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
    repeat_count: i8,
) {
    draw_end_section(
        frame,
        measure_end_x,
        measure_start_y,
        vertical_measure_height,
    );
    // draw repeat dots
    draw_repeat_dots(
        frame,
        measure_end_x - HALF_BEAT_LENGTH,
        measure_start_y,
        vertical_measure_height,
    );
    // add repeat count text
    let repeat_count_text = Text {
        shaping: Auto,
        content: format!("x{repeat_count}"),
        color: Color::WHITE,
        size: 9.0.into(),
        position: Point::new(measure_end_x - 12.0, FIRST_STRING_Y - 15.0),
        ..Text::default()
    };
    frame.fill_text(repeat_count_text);
}

/// Picking direction above the staff, like TuxGuitar's `paintPickStroke`:
/// an up stroke is a `∨`, a down stroke the bracket-shaped `∏`.
fn draw_pick_stroke(
    frame: &mut Frame<Renderer>,
    direction: &BeatStrokeDirection,
    beat_position_x: f32,
) {
    let stroke = Stroke::default().with_width(0.8).with_color(Color::WHITE);
    let x = beat_position_x + 3.5;
    let y = PICK_STROKE_Y;
    match direction {
        BeatStrokeDirection::Up => {
            let tip = Point::new(x, y + 8.0);
            frame.stroke(&Path::line(Point::new(x - 3.0, y), tip), stroke);
            frame.stroke(&Path::line(Point::new(x + 3.0, y), tip), stroke);
        }
        BeatStrokeDirection::Down => {
            frame.stroke(
                &Path::line(Point::new(x - 3.0, y), Point::new(x - 3.0, y + 6.0)),
                stroke,
            );
            frame.stroke(
                &Path::line(Point::new(x + 3.0, y), Point::new(x + 3.0, y + 6.0)),
                stroke,
            );
            let top_bar = Path::line(Point::new(x - 3.0, y), Point::new(x + 3.0, y));
            frame.stroke(&top_bar, Stroke::default().with_width(2.0).with_color(Color::WHITE));
        }
        BeatStrokeDirection::None => {}
    }
}

/// Staccato dot above the top note of the beat (TuxGuitar only draws it in
/// score mode; the dot above the fret number is the tab equivalent).
fn draw_staccato_dot(
    frame: &mut Frame<Renderer>,
    beat: &Beat,
    beat_position_x: f32,
    measure_start_y: f32,
) {
    let min_string = beat.notes.iter().map(|n| n.string).min().unwrap_or(1);
    let top_note_y = measure_start_y + (f32::from(min_string) - 1.0) * STRING_LINE_HEIGHT - 5.0;
    let center = Point::new(beat_position_x + 3.5, top_note_y - 3.0);
    frame.fill(&Path::circle(center, 1.3), Color::WHITE);
}

/// Tremolo picking slashes below the tab, one per duration halving from an
/// eighth note, like TuxGuitar's tablature-only rendering.
fn draw_tremolo_picking(
    frame: &mut Frame<Renderer>,
    tremolo_picking: &TremoloPickingEffect,
    beat_position_x: f32,
    tab_bottom_y: f32,
) {
    let slashes = match tremolo_picking.duration.value {
        v if v >= 32 => 3,
        v if v >= 16 => 2,
        _ => 1,
    };
    let stroke = Stroke::default().with_width(1.2).with_color(Color::WHITE);
    let x = beat_position_x + 3.5;
    let mut y = tab_bottom_y + 5.0;
    for _ in 0..slashes {
        frame.stroke(
            &Path::line(Point::new(x - 3.5, y + 1.5), Point::new(x + 3.5, y - 1.5)),
            stroke,
        );
        y += 4.0;
    }
}

fn draw_stroke_arrow(
    frame: &mut Frame<Renderer>,
    beat: &Beat,
    beat_position_x: f32,
    measure_start_y: f32,
) {
    let min_string = beat.notes.iter().map(|n| n.string).min().unwrap_or(1);
    let max_string = beat.notes.iter().map(|n| n.string).max().unwrap_or(1);
    let top_y = measure_start_y + (f32::from(min_string) - 1.0) * STRING_LINE_HEIGHT;
    let bottom_y = measure_start_y + (f32::from(max_string) - 1.0) * STRING_LINE_HEIGHT;
    let arrow_x = beat_position_x + 10.0;
    let arrow_size = 3.0;

    let stroke = Stroke::default().with_width(0.8).with_color(Color::WHITE);

    // vertical line spanning the chord
    frame.stroke(
        &Path::line(Point::new(arrow_x, top_y), Point::new(arrow_x, bottom_y)),
        stroke,
    );

    // arrowhead: down stroke = pick goes low-to-high strings = arrowhead at top
    match beat.effect.stroke.direction {
        BeatStrokeDirection::Down => {
            let tip = Point::new(arrow_x, top_y - arrow_size);
            frame.stroke(
                &Path::line(Point::new(arrow_x - arrow_size, top_y), tip),
                stroke,
            );
            frame.stroke(
                &Path::line(Point::new(arrow_x + arrow_size, top_y), tip),
                stroke,
            );
        }
        BeatStrokeDirection::Up => {
            let tip = Point::new(arrow_x, bottom_y + arrow_size);
            frame.stroke(
                &Path::line(Point::new(arrow_x - arrow_size, bottom_y), tip),
                stroke,
            );
            frame.stroke(
                &Path::line(Point::new(arrow_x + arrow_size, bottom_y), tip),
                stroke,
            );
        }
        BeatStrokeDirection::None => {}
    }
}

fn draw_alternative_ending(
    frame: &mut Frame<Renderer>,
    repeat_alternative: u8,
    measure_start_x: f32,
    measure_width: f32,
) {
    let bracket_y = MEASURE_ANNOTATION_Y;
    let bracket_height = 10.0;
    let bracket_start = measure_start_x + 2.0;
    let bracket_end = measure_start_x + measure_width;

    let stroke = Stroke::default().with_width(1.0).with_color(Color::WHITE);

    // vertical line down
    let start = Point::new(bracket_start, bracket_y);
    let down = Point::new(bracket_start, bracket_y + bracket_height);
    frame.stroke(&Path::line(start, down), stroke);

    // horizontal line across
    let right = Point::new(bracket_end, bracket_y);
    frame.stroke(&Path::line(start, right), stroke);

    // build label from bitmask (e.g., 1 → "1.", 2 → "2.", 3 → "1.2.")
    let mut label = String::new();
    for bit in 0..8_u8 {
        if repeat_alternative & (1 << bit) != 0 {
            if !label.is_empty() {
                label.push('.');
            }
            label.push_str(&(bit + 1).to_string());
        }
    }
    label.push('.');

    let label_text = Text {
        shaping: Auto,
        content: label,
        color: Color::WHITE,
        size: 9.0.into(),
        position: Point::new(bracket_start + 3.0, bracket_y),
        ..Text::default()
    };
    frame.fill_text(label_text);
}

fn draw_repeat_dots(
    frame: &mut Frame<Renderer>,
    start_x: f32,
    start_y: f32,
    vertical_measure_height: f32,
) {
    // top dot
    let top_position_y = start_y + vertical_measure_height / 3.0;
    let center = Point::new(start_x, top_position_y);
    let circle = Path::circle(center, 1.0);

    frame.stroke(
        &circle,
        Stroke::default().with_width(2.0).with_color(Color::WHITE),
    );

    // bottom dot
    let bottom_position_y = start_y + (vertical_measure_height / 3.0) * 2.0;
    let center = Point::new(start_x, bottom_position_y);
    let circle = Path::circle(center, 1.0);

    frame.stroke(
        &circle,
        Stroke::default().with_width(2.0).with_color(Color::WHITE),
    );
}

fn draw_end_section(
    frame: &mut Frame<Renderer>,
    measure_end_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    // draw first thin one
    draw_measure_vertical_line(
        frame,
        vertical_measure_height,
        measure_end_x - 8.0,
        measure_start_y,
    );

    // then thick one
    let position_x = measure_end_x - 2.0;
    let start_point = Point::new(position_x, measure_start_y);
    let end_point = Point::new(position_x, measure_start_y + vertical_measure_height);
    let thick_vertical_line = Path::line(start_point, end_point);
    let stroke = Stroke::default().with_width(4.0).with_color(Color::WHITE);
    frame.stroke(&thick_vertical_line, stroke);
}

fn draw_time_signature(
    frame: &mut Frame<Renderer>,
    time_signature: &TimeSignature,
    measure_start_x: f32,
    string_count: usize,
    has_repeat: bool,
) {
    let position_x = if has_repeat {
        BEAT_LENGTH
    } else {
        HALF_BEAT_LENGTH
    };
    let position_y = if string_count > 4 {
        (STRING_LINE_HEIGHT * (string_count - 4) as f32) / 2.0
    } else {
        0.0
    };
    let numerator = time_signature.numerator;
    let denominator = time_signature.denominator.value;
    let tempo_text = Text {
        shaping: Auto,
        content: format!("{numerator}\n{denominator}"),
        color: Color::WHITE,
        size: 17.into(),
        position: Point::new(
            measure_start_x + position_x,
            (FIRST_STRING_Y - 1.0) + position_y,
        ),
        ..Text::default()
    };
    frame.fill_text(tempo_text);
}

// Similar to `https://www.tuxguitar.app/files/1.6.0/desktop/help/edit_effects.html`
fn above_note_effect_annotation(note_effect: &NoteEffect) -> Vec<&'static str> {
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

fn inlined_note_effect_annotation(note_effect: &NoteEffect) -> String {
    let mut annotation = String::new();
    if note_effect.hammer {
        // https://unicodeplus.com/U+25E0
        annotation.push(HAMMER_ON);
    }
    if let Some(slide) = &note_effect.slide {
        match slide {
            SlideType::IntoFromAbove => annotation.push(HORIZONTAL_BAR),
            SlideType::IntoFromBelow => annotation.push(HORIZONTAL_BAR),
            SlideType::ShiftSlideTo => annotation.push(SHIFT_SLIDE),
            SlideType::LegatoSlideTo => annotation.push(LEGATO_SLIDE),
            SlideType::OutDownwards => annotation.push(HORIZONTAL_BAR),
            SlideType::OutUpWards => annotation.push(LEGATO_SLIDE),
        }
    }
    annotation
}

fn note_value(note: &Note) -> String {
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
        NoteType::Tie => {
            // https://unicodeplus.com/U+2323
            TIE.into()
        }
        NoteType::Dead => "x".to_string(),
        NoteType::Unknown(i) => {
            log::warn!("NoteType Unknown({i})");
            String::new()
        }
    }
}
