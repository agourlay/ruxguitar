use crate::parser::song_parser::{
    Beat, HarmonicType, Note, NoteEffect, NoteType, SlapEffect, SlideType, Song, TimeSignature,
};
use crate::ui::application::Message;
use iced::advanced::mouse;
use iced::advanced::text::Shaping::Advanced;
use iced::alignment::Horizontal::Center;
use iced::event::Status;
use iced::mouse::{Cursor, Interaction};
use iced::widget::canvas::{Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{canvas, Canvas};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use std::ops::Div;
use std::rc::Rc;

// Drawing constants

// Measure label + marker
const MEASURE_ANNOTATION_Y: f32 = 3.0;

// Chord label level
const CHORD_ANNOTATION_Y: f32 = 15.0;

// Note effect level
const NOTE_EFFECT_ANNOTATION_Y: f32 = 27.0;

// First string level
const FIRST_STRING_Y: f32 = 50.0;

// Distance between strings
const STRING_LINE_HEIGHT: f32 = 13.0;

// Measure notes padding
const MEASURE_NOTES_PADDING: f32 = 20.0;

// Length of a beat
const BEAT_LENGTH: f32 = 24.0;

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
    pub total_measure_len: f32,
    pub vertical_measure_height: f32,
    has_time_signature: bool,
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
        let beat_count = measure.voices[0].beats.len();
        let measure_len = MIN_MEASURE_WIDTH.max(beat_count as f32 * BEAT_LENGTH);
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
        let vertical_measure_height = vertical_measure_height + FIRST_STRING_Y * 2.0;
        Self {
            measure_id,
            track_id,
            song,
            is_focused: focused,
            focused_beat: 0,
            canvas_cache: Cache::default(),
            measure_len,
            total_measure_len,
            vertical_measure_height,
            has_time_signature,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let canvas = Canvas::new(self)
            .height(self.vertical_measure_height)
            .width(Length::Fixed(self.total_measure_len));
        canvas.into()
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

    pub fn clear_canva_cache(&self) {
        self.canvas_cache.clear();
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
        event: Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> (Status, Option<Message>) {
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event {
            if let Some(_cursor_position) = cursor.position_in(bounds) {
                log::info!("Clicked on measure {:?}", self.measure_id);
                *state = MeasureInteraction::Clicked;
                return (
                    Status::Captured,
                    Some(Message::FocusMeasure(self.measure_id)),
                );
            };
        }
        (Status::Ignored, None)
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

            // distance between lines of measures
            let vertical_measure_height = STRING_LINE_HEIGHT * (string_count - 1) as f32;

            // Positive x-values extend to the right, and positive y-values extend downwards.
            let measure_start_x = 0.0;
            let measure_start_y = FIRST_STRING_Y;

            // colors
            let color_gray = Color::from_rgb8(0x40, 0x44, 0x4B);
            let color_dark_red = Color::from_rgb8(200, 50, 50);

            // draw focused box
            if self.is_focused {
                draw_focused_box(
                    frame,
                    self.total_measure_len,
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
                    measure_start_x + self.total_measure_len,
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
                // draw first vertical line only for measure at the of rows
                // otherwise it is doubling with the line at the end of the previous measure
                if bounds.x == MEASURE_NOTES_PADDING {
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
                let tempo_sign = std::char::from_u32(0x1D15F).unwrap(); // https://unicodeplus.com/U+1D15F
                let tempo_label = format!("{} = {}", tempo_sign, measure_header.tempo.value);
                tempo_label_len = tempo_label.chars().count() * 10;
                let tempo_text = Text {
                    shaping: Advanced, // required for printing unicode
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
                    shaping: Advanced, // required for printing unicode
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
                shaping: Advanced, // required for printing unicode
                content: format!("{}", self.measure_id + 1),
                color: color_dark_red,
                size: 10.0.into(),
                position: Point::new(measure_start_x, FIRST_STRING_Y - 15.0),
                ..Text::default()
            };
            frame.fill_text(measure_count_text);

            // add notes on top of strings
            let measure = &track.measures[self.measure_id];
            // TODO draw second voice if present?
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
            for (b_id, beat) in beats.iter().enumerate() {
                // pick color if beat under focus
                let beat_color = if self.is_focused && b_id == self.focused_beat {
                    color_dark_red
                } else {
                    Color::WHITE
                };
                // draw beat
                draw_beat(
                    frame,
                    self.measure_len,
                    beat_start,
                    measure_start_y,
                    beats_len,
                    b_id,
                    beat,
                    beat_color,
                );
            }

            // draw close measure
            if measure_header.repeat_close > 0 {
                draw_close_repeat(
                    frame,
                    measure_start_x + self.total_measure_len,
                    measure_start_y,
                    vertical_measure_height,
                    measure_header.repeat_close,
                );
            } else if next_measure_header.is_none() {
                draw_end_section(
                    frame,
                    measure_start_x + self.total_measure_len,
                    measure_start_y,
                    vertical_measure_height,
                );
            } else {
                // vertical measure end
                draw_measure_vertical_line(
                    frame,
                    vertical_measure_height,
                    measure_start_x + self.total_measure_len, // end of measure
                    measure_start_y,
                );
            }
        });

        vec![tab]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Interaction {
        match cursor {
            Cursor::Available(_point) => {
                if let Some(_cursor_position) = cursor.position_in(bounds) {
                    log::debug!("Mouse over measure {:?}", self.measure_id);
                }
            }
            Cursor::Unavailable => {}
        }
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

#[allow(clippy::too_many_arguments)]
fn draw_beat(
    frame: &mut Frame<Renderer>,
    measure_len: f32,
    measure_start_x: f32,
    measure_start_y: f32,
    beats_len: usize,
    b_id: usize,
    beat: &Beat,
    beat_color: Color,
) {
    // position to draw beat
    let width_per_beat = measure_len / beats_len as f32;
    let beat_position_offset = b_id as f32 * width_per_beat;
    let beat_position_x = measure_start_x + MEASURE_NOTES_PADDING + beat_position_offset;

    // Annotate chord effect
    if let Some(chord) = &beat.effect.chord {
        let note_effect_text = Text {
            shaping: Advanced, // required for printing unicode
            content: chord.name.clone(),
            color: Color::WHITE,
            size: 8.0.into(),
            position: Point::new(beat_position_x + 3.0, CHORD_ANNOTATION_Y),
            ..Text::default()
        };
        frame.fill_text(note_effect_text);
    };
    if !beat.effect.stroke.is_empty() {
        // TODO display correct arrow on the chord
    }

    // Annotate note effect above (same position for all notes)
    let mut beat_annotations = Vec::new();

    // draw notes for beat
    for note in &beat.notes {
        beat_annotations.extend(above_note_effect_annotation(&note.effect));
        draw_note(
            frame,
            measure_start_y,
            beat_position_x,
            width_per_beat,
            note,
            beat_color,
        );
    }

    // merge and display beat annotations
    if !beat_annotations.is_empty() {
        beat_annotations.sort_unstable();
        beat_annotations.dedup();
        let merged_annotations = beat_annotations.join("\n");
        let y_position = NOTE_EFFECT_ANNOTATION_Y - 4.0 * (beat_annotations.len() - 1) as f32;
        let note_effect_text = Text {
            shaping: Advanced, // required for printing unicode
            content: merged_annotations,
            color: Color::WHITE,
            size: 9.0.into(),
            position: Point::new(beat_position_x - 3.0, y_position),
            ..Text::default()
        };
        frame.fill_text(note_effect_text);
    }
}

fn draw_note(
    frame: &mut Frame<Renderer>,
    measure_start_y: f32,
    beat_position_x: f32,
    width_per_beat: f32,
    note: &Note,
    beat_color: Color,
) {
    // note label (pushed down on the right string)
    let note_label = note_value(note);
    let local_beat_position_y = (f32::from(note.string) - 1.0) * STRING_LINE_HEIGHT;
    // center the notes with more than one char
    let note_position_x = beat_position_x + 3.0 - note_label.chars().count() as f32 / 2.0;
    let note_position_y = measure_start_y + local_beat_position_y - 5.0;
    let note_text = Text {
        shaping: Advanced, // required for printing unicode
        content: note_label,
        color: beat_color,
        size: 10.0.into(),
        position: Point::new(note_position_x, note_position_y),
        horizontal_alignment: Center,
        ..Text::default()
    };
    frame.fill_text(note_text);

    // Annotate some effects on the string after the note
    let inlined_annotation_width = 10.0;
    let inlined_annotation_label = inlined_note_effect_annotation(&note.effect);
    // note_x + half of inter-beat space - half of annotation width
    let annotation_position_x =
        note_position_x + width_per_beat / 2.0 - inlined_annotation_width / 2.0;
    let note_effect_text = Text {
        shaping: Advanced, // required for printing unicode
        content: inlined_annotation_label,
        color: Color::WHITE,
        size: inlined_annotation_width.into(),
        position: Point::new(annotation_position_x, note_position_y),
        ..Text::default()
    };
    frame.fill_text(note_effect_text);
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
        shaping: Advanced, // required for printing unicode
        content: format!("x{repeat_count}"),
        color: Color::WHITE,
        size: 9.0.into(),
        position: Point::new(measure_end_x - 12.0, FIRST_STRING_Y - 15.0),
        ..Text::default()
    };
    frame.fill_text(repeat_count_text);
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
        (STRING_LINE_HEIGHT * (string_count - 4) as f32).div(2.0)
    } else {
        0.0
    };
    let numerator = time_signature.numerator;
    let denominator = time_signature.denominator.value;
    let tempo_text = Text {
        shaping: Advanced, // required for printing unicode
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
fn above_note_effect_annotation(note_effect: &NoteEffect) -> Vec<String> {
    let mut annotations: Vec<String> = vec![];
    if note_effect.accentuated_note {
        annotations.push(">".to_string());
    }
    if note_effect.heavy_accentuated_note {
        annotations.push("^".to_string());
    }
    if note_effect.palm_mute {
        annotations.push("P.M".to_string());
    }
    if note_effect.let_ring {
        annotations.push("L.R".to_string());
    }
    if note_effect.fade_in {
        annotations.push("<".to_string());
    }
    if let Some(harmonic) = &note_effect.harmonic {
        match harmonic.kind {
            HarmonicType::Natural => annotations.push("N.H".to_string()),
            HarmonicType::Artificial => annotations.push("A.H".to_string()),
            HarmonicType::Tapped => annotations.push("T.H".to_string()),
            HarmonicType::Pinch => annotations.push("P.H".to_string()),
            HarmonicType::Semi => annotations.push("S.H".to_string()),
        }
    }
    if note_effect.vibrato {
        let vibrato = std::char::from_u32(0x301C).unwrap().to_string(); // https://unicodeplus.com/U+301C
        annotations.push(vibrato.repeat(2));
    }
    if note_effect.trill.is_some() {
        annotations.push("Tr".to_string());
    }
    if note_effect.tremolo_picking.is_some() {
        annotations.push("T.P".to_string());
    }
    if note_effect.tremolo_bar.is_some() {
        annotations.push("T.B".to_string());
    }
    match note_effect.slap {
        SlapEffect::Tapping => annotations.push("T".to_string()),
        SlapEffect::None => (),
        _ => (),
    }
    annotations
}

fn inlined_note_effect_annotation(note_effect: &NoteEffect) -> String {
    let mut annotation = String::new();
    if note_effect.hammer {
        // https://unicodeplus.com/U+25E0
        annotation.push(std::char::from_u32(0x25E0).unwrap());
    }
    if let Some(slide) = &note_effect.slide {
        match slide {
            SlideType::IntoFromAbove => annotation.push(std::char::from_u32(0x2015).unwrap()), // https://unicodeplus.com/U+2015
            SlideType::IntoFromBelow => annotation.push(std::char::from_u32(0x2015).unwrap()), // https://unicodeplus.com/U+2015
            SlideType::ShiftSlideTo => annotation.push(std::char::from_u32(0x27CD).unwrap()), // https://unicodeplus.com/U+27CD
            SlideType::LegatoSlideTo => annotation.push(std::char::from_u32(0x27CB).unwrap()), // https://unicodeplus.com/U+27CB
            SlideType::OutDownwards => annotation.push(std::char::from_u32(0x2015).unwrap()), // https://unicodeplus.com/U+2015
            SlideType::OutUpWards => annotation.push(std::char::from_u32(0x27CB).unwrap()), // https://unicodeplus.com/U+27CB
        }
    }
    if let Some(bend) = &note_effect.bend {
        let direction_up = bend.direction() >= 0;
        // TODO display bend properly
        if direction_up {
            annotation.push(std::char::from_u32(0x2191).unwrap()); // https://unicodeplus.com/U+2191
        } else {
            annotation.push(std::char::from_u32(0x2193).unwrap()); // https://unicodeplus.com/U+2193
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
            std::char::from_u32(0x2323).unwrap().into()
        }
        NoteType::Dead => "x".to_string(),
        NoteType::Unknown(i) => {
            log::warn!("NoteType Unknown({i})");
            String::new()
        }
    }
}
