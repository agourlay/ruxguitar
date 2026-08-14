use crate::parser::song_parser::{
    Beat, BeatStrokeDirection, BendEffect, Duration, GraceEffect, HarmonicType, Measure,
    MeasureHeader, Note, NoteEffect, NoteType, SlapEffect, SlideType, Song, TimeSignature,
    TremoloPickingEffect,
};
use crate::ui::application::Message;
use crate::ui::utils::TablatureColors;
use iced::advanced::mouse;
use iced::advanced::text::Shaping::Auto;
use iced::mouse::Cursor;
use iced::widget::canvas::{Cache, Event, Frame, Geometry, LineDash, Path, Stroke, Text};
use iced::widget::text::Alignment;
use iced::widget::{Action, Canvas, canvas};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use std::cell::Cell;
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

// Annotation rows are stacked above the staff, top to bottom, and each one
// is only allocated when something on the line uses it (like TuxGuitar's
// TGTrackSpacing). Tremolo picking slashes are drawn below the last string.
const ROW_ALT_ENDING: f32 = 13.0;
const ROW_MARKER: f32 = 15.0;
const ROW_CHORD: f32 = 11.0;
const ROW_TUPLET: f32 = 12.0;
const ROW_EFFECT_LINE: f32 = 12.0;
const ROW_TEXT: f32 = 11.0;
const ROW_PICK_STROKE: f32 = 10.0;
// Always-present gap between the last annotation row and the staff: holds
// the measure number, the repeat count and the focus box edge.
const STAFF_HEADER: f32 = 16.0;
// Bends reach higher above the staff for their amplitude labels.
const STAFF_HEADER_WITH_BEND: f32 = 20.0;
// Gap below the last string, holding the focus box edge.
const STAFF_FOOTER: f32 = 10.0;
// Tremolo picking slashes hang further below the staff.
const STAFF_FOOTER_WITH_TREMOLO: f32 = 19.0;

/// Height of each annotation row above the staff, zero when unused.
///
/// Computed per measure, then merged across every measure of a line so
/// their staves stay aligned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowSpacing {
    alt_ending: f32,
    marker: f32,
    chord: f32,
    tuplet: f32,
    effects: f32,
    text: f32,
    pick_stroke: f32,
    staff_header: f32,
    staff_footer: f32,
}

impl Default for RowSpacing {
    fn default() -> Self {
        Self {
            alt_ending: 0.0,
            marker: 0.0,
            chord: 0.0,
            tuplet: 0.0,
            effects: 0.0,
            text: 0.0,
            pick_stroke: 0.0,
            staff_header: STAFF_HEADER,
            staff_footer: STAFF_FOOTER,
        }
    }
}

impl RowSpacing {
    /// The alternative-ending bracket sits at the very top.
    const ALT_ENDING_Y: f32 = 0.0;

    /// Which rows a measure uses, and how tall they need to be.
    fn for_measure(measure: &Measure, header: &MeasureHeader, has_tempo_label: bool) -> Self {
        let mut spacing = Self::default();
        if header.repeat_alternative > 0 {
            spacing.alt_ending = ROW_ALT_ENDING;
        }
        if has_tempo_label || header.marker.is_some() {
            spacing.marker = ROW_MARKER;
        }
        let beats = &measure.voices[0].beats;
        if beats.iter().any(|beat| beat.effect.chord.is_some()) {
            spacing.chord = ROW_CHORD;
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
        self.chord = self.chord.max(other.chord);
        self.tuplet = self.tuplet.max(other.tuplet);
        self.effects = self.effects.max(other.effects);
        self.text = self.text.max(other.text);
        self.pick_stroke = self.pick_stroke.max(other.pick_stroke);
        self.staff_header = self.staff_header.max(other.staff_header);
        self.staff_footer = self.staff_footer.max(other.staff_footer);
    }

    const fn marker_y(self) -> f32 {
        self.alt_ending
    }

    const fn chord_y(self) -> f32 {
        self.marker_y() + self.marker
    }

    const fn tuplet_y(self) -> f32 {
        self.chord_y() + self.chord
    }

    const fn effects_y(self) -> f32 {
        self.tuplet_y() + self.tuplet
    }

    const fn text_y(self) -> f32 {
        self.effects_y() + self.effects
    }

    const fn pick_stroke_y(self) -> f32 {
        self.text_y() + self.text
    }

    /// First tab line: below every annotation row.
    const fn first_string_y(self) -> f32 {
        self.pick_stroke_y() + self.pick_stroke + self.staff_header
    }
}

// Distance between strings
const STRING_LINE_HEIGHT: f32 = 13.0;

// Measure notes padding
const MEASURE_NOTES_PADDING: f32 = 20.0;

// Length of a beat
const BEAT_LENGTH: f32 = 24.0;

// Width of one bend/release arrow
const BEND_ARROW_WIDTH: f32 = 10.0;

// Approximate digit advance of the fret and grace fonts, used to keep the
// grace note clear of the note it precedes.
const NOTE_DIGIT_WIDTH: f32 = 5.5;
const GRACE_DIGIT_WIDTH: f32 = 4.0;
// Gap kept on both sides of a grace note.
const GRACE_GAP: f32 = 1.5;

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
    // rows this measure needs, and the rows granted to its line
    row_needs: RowSpacing,
    row_spacing: RowSpacing,
    // colors the cached geometry was painted with
    painted_colors: Cell<Option<TablatureColors>>,
    has_tempo_label: bool,
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
        let beats = &measure.voices[0].beats;
        let beat_widths: Vec<f32> = beats
            .iter()
            .enumerate()
            .map(|(i, beat)| beat_natural_width(beat, beats.get(i + 1)))
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
        // the tempo is shown on the first measure and whenever it changes
        let has_tempo_label = measure_id.checked_sub(1).is_none_or(|previous| {
            measure_header.tempo != song.measure_headers[previous].tempo
        });
        let row_needs = RowSpacing::for_measure(measure, measure_header, has_tempo_label);
        let vertical_measure_height = measure_height(row_needs, string_count);
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
            row_needs,
            row_spacing: row_needs,
            painted_colors: Cell::new(None),
            has_tempo_label,
        }
    }

    /// The annotation rows this measure needs, for the line to merge.
    pub const fn row_needs(&self) -> RowSpacing {
        self.row_needs
    }

    /// Apply the rows granted to this measure's line.
    pub fn set_row_spacing(&mut self, row_spacing: RowSpacing) {
        if self.row_spacing != row_spacing {
            self.row_spacing = row_spacing;
            let string_count = self.song.tracks[self.track_id].strings.len();
            self.vertical_measure_height = measure_height(row_spacing, string_count);
            self.canvas_cache.clear();
        }
    }

    pub const fn set_first_on_line(&mut self, value: bool) {
        self.is_first_on_line = value;
    }

    pub fn view(&self) -> Element<'_, Message> {
        Canvas::new(self)
            .height(self.vertical_measure_height)
            .width(Length::Fixed(self.total_measure_len))
            .into()
    }

    /// View stretched to fill its row, weighted by the natural measure width.
    pub fn view_fill(&self) -> Element<'_, Message> {
        let portion = (self.total_measure_len.round() as u16).max(1);
        Canvas::new(self)
            .height(self.vertical_measure_height)
            .width(Length::FillPortion(portion))
            .into()
    }

    /// The fixed overhead width (padding, time signature, repeats) that doesn't scale with beats.
    fn overhead_width(&self) -> f32 {
        self.total_measure_len - self.measure_len
    }

    pub fn toggle_focused(&mut self) {
        self.is_focused = !self.is_focused;
        self.focused_beat = 0;
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
            return Some(Action::publish(Message::FocusMeasure(
                self.measure_id,
                beat_id,
            )));
        }
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let colors = TablatureColors::of(theme);
        // the cached geometry holds the colors it was painted with, so a
        // change of theme has to repaint it
        if self.painted_colors.get() != Some(colors) {
            self.painted_colors.set(Some(colors));
            self.canvas_cache.clear();
        }
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
            let rows = self.row_spacing;
            let measure_start_y = rows.first_string_y();

            // draw focused box
            if self.is_focused {
                draw_focused_box(
                    frame,
                    colors,
                    actual_width,
                    vertical_measure_height,
                    measure_start_x,
                    measure_start_y,
                    rows,
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
                let stroke = Stroke::default().with_width(0.8).with_color(colors.string_line);
                frame.stroke(&line, stroke);
            }

            // measure headers
            let measure_header = &self.song.measure_headers[self.measure_id];
            let next_measure_header = &self.song.measure_headers.get(self.measure_id + 1);

            // display open measure bar
            if measure_header.repeat_open {
                draw_open_repeat(
                    frame,
                    colors,
                    measure_start_x,
                    measure_start_y,
                    vertical_measure_height,
                );
            } else if self.measure_id == 0 {
                draw_open_section(
                    frame,
                    colors,
                    measure_start_x,
                    measure_start_y,
                    vertical_measure_height,
                );
            } else if self.is_first_on_line {
                // only the first measure on a row draws its opening line,
                // otherwise it doubles with the previous measure's end line
                draw_measure_vertical_line(
                    frame,
                    colors,
                    vertical_measure_height,
                    measure_start_x,
                    measure_start_y,
                );
            }

            // display time signature (if first measure OR if it changed)
            if self.has_time_signature {
                draw_time_signature(
                    frame,
                    colors,
                    &measure_header.time_signature,
                    measure_start_x,
                    measure_start_y,
                    string_count,
                    measure_header.repeat_open, // need to offset if repeat dots present
                );
            }

            // capture tempo label len to adjust next annotations
            let mut tempo_label_len = 0;
            // display measure tempo (if first measure OR if it changed)
            if self.has_tempo_label {
                let tempo_label = format!("{TEMPO_SIGN} = {}", measure_header.tempo.value);
                tempo_label_len = tempo_label.chars().count() * 10;
                let tempo_text = Text {
                    shaping: Auto,
                    content: tempo_label,
                    color: colors.foreground,
                    size: 11.0.into(),
                    position: Point::new(measure_start_x, rows.marker_y()),
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
                    color: colors.accent,
                    size: 10.0.into(),
                    position: Point::new(
                        measure_start_x + MEASURE_NOTES_PADDING + tempo_label_len as f32,
                        rows.marker_y(),
                    ),
                    ..Text::default()
                };
                frame.fill_text(marker_text);
            }

            // measure count label
            let measure_count_text = Text {
                shaping: Auto,
                content: format!("{}", self.measure_id + 1),
                color: colors.accent,
                size: 10.0.into(),
                position: Point::new(measure_start_x, measure_start_y - 15.0),
                ..Text::default()
            };
            frame.fill_text(measure_count_text);

            // alternative ending bracket (e.g., "1.", "2.", "1.2.")
            if measure_header.repeat_alternative > 0 {
                draw_alternative_ending(
                    frame,
                    colors,
                    measure_header.repeat_alternative,
                    measure_start_x,
                    actual_width,
                    RowSpacing::ALT_ENDING_Y,
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
            let mut beat_positions = Vec::with_capacity(beats_len);
            for (b_id, beat) in beats.iter().enumerate() {
                // pick color if beat under focus
                let beat_color = if self.is_focused && b_id == self.focused_beat {
                    colors.accent
                } else {
                    colors.foreground
                };
                let beat_width = self.beat_widths[b_id] * width_scale;
                // the inline effect glyphs may not reach into the space the
                // next beat's grace note occupies
                let next_grace = beats.get(b_id + 1).map_or(0.0, grace_gap_width);
                // draw beat
                draw_beat(
                    frame,
                    colors,
                    beat_position_x,
                    beat_width - next_grace,
                    width_scale,
                    measure_start_y,
                    vertical_measure_height,
                    rows,
                    beat,
                    beat_color,
                );
                beat_positions.push(beat_position_x);
                beat_position_x += beat_width;
            }

            // tuplet brackets span consecutive beats of the same division
            if rows.tuplet > 0.0 {
                for (enters, x1, x2) in tuplet_runs(beats, &beat_positions) {
                    draw_tuplet_bracket(frame, colors, enters, x1, x2, rows.tuplet_y());
                }
            }

            // draw close measure
            if measure_header.repeat_close > 0 {
                draw_close_repeat(
                    frame,
                    colors,
                    measure_start_x + actual_width,
                    measure_start_y,
                    vertical_measure_height,
                    measure_header.repeat_close,
                );
            } else if next_measure_header.is_none() {
                draw_end_section(
                    frame,
                    colors,
                    measure_start_x + actual_width,
                    measure_start_y,
                    vertical_measure_height,
                );
            } else {
                // vertical measure end
                draw_measure_vertical_line(
                    frame,
                    colors,
                    vertical_measure_height,
                    measure_start_x + actual_width, // end of measure
                    measure_start_y,
                );
            }
        });

        vec![tab]
    }
}

fn draw_focused_box(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    total_measure_len: f32,
    vertical_measure_height: f32,
    measure_start_x: f32,
    measure_start_y: f32,
    rows: RowSpacing,
) {
    const BOX_SIDE: f32 = 8.0;
    // keep the line off the canvas edge
    const CANVAS_MARGIN: f32 = 2.0;

    // ride the header and footer boundaries so the box encloses everything
    // drawn around the staff - measure number, bend labels, staccato dots
    // above, tremolo picking slashes below - without crossing any of it
    let top = rows.staff_header;
    let bottom = rows.staff_footer - CANVAS_MARGIN;
    let focused_box = Rectangle {
        x: measure_start_x + BOX_SIDE,
        y: measure_start_y - top,
        width: total_measure_len - BOX_SIDE * 2.0,
        height: vertical_measure_height + top + bottom,
    };

    let Rectangle {
        x,
        y,
        width,
        height,
    } = focused_box;

    let top_left = Point::new(x, y);
    let rectangle_size = Size::new(width, height);
    let stroke = Stroke::default().with_width(1.0).with_color(colors.foreground);
    frame.stroke_rectangle(top_left, rectangle_size, stroke);
}

fn draw_measure_vertical_line(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    vertical_measure_height: f32,
    measure_start_x: f32,
    measure_start_y: f32,
) {
    let start_point = Point::new(measure_start_x, measure_start_y);
    let end_point = Point::new(measure_start_x, measure_start_y + vertical_measure_height);
    let vertical_line = Path::line(start_point, end_point);
    let stroke = Stroke::default().with_width(1.5).with_color(colors.foreground);
    frame.stroke(&vertical_line, stroke);
}

/// Total canvas height: annotation rows, the staff, and the footer.
fn measure_height(rows: RowSpacing, string_count: usize) -> f32 {
    rows.first_string_y() + STRING_LINE_HEIGHT * (string_count - 1) as f32 + rows.staff_footer
}

/// Effect annotations stacked above a beat, deduplicated across its notes.
/// Shared by the row sizing and the drawing so both agree on the height.
fn beat_annotations(beat: &Beat) -> Vec<&'static str> {
    let mut annotations: Vec<&'static str> = beat
        .notes
        .iter()
        .flat_map(|note| above_note_effect_annotation(&note.effect))
        .collect();
    annotations.sort_unstable();
    annotations.dedup();
    annotations
}

/// Natural width of a beat: base length, room for bend arrows (like
/// TuxGuitar's `getEffectWidth`), and room for the grace note that the
/// next beat draws in the gap before it.
fn beat_natural_width(beat: &Beat, next_beat: Option<&Beat>) -> f32 {
    let bend_extra = beat
        .notes
        .iter()
        .filter_map(|n| n.effect.bend.as_ref())
        .map(|b| b.movements().len() as f32 * BEND_ARROW_WIDTH)
        .fold(0.0, f32::max);
    let grace_extra = next_beat.map_or(0.0, grace_gap_width);
    BEAT_LENGTH + bend_extra + grace_extra
}

/// Label of a grace note: its fret, or a cross when it is dead.
fn grace_label(grace: &GraceEffect) -> String {
    if grace.is_dead {
        "x".to_string()
    } else {
        grace.fret.to_string()
    }
}

/// Room the grace notes of a beat need in the gap before it.
fn grace_gap_width(beat: &Beat) -> f32 {
    beat.notes
        .iter()
        .filter_map(|note| note.effect.grace.as_ref())
        .map(|grace| {
            grace_label(grace).chars().count() as f32 * GRACE_DIGIT_WIDTH + GRACE_GAP * 2.0
        })
        .fold(0.0, f32::max)
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
    colors: TablatureColors,
    beat_position_x: f32,
    // span usable for glyphs drawn after the note, up to the next beat's grace
    beat_span: f32,
    width_scale: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
    rows: RowSpacing,
    beat: &Beat,
    beat_color: Color,
) {
    // Annotate chord effect
    if let Some(chord) = &beat.effect.chord {
        let note_effect_text = Text {
            shaping: Auto,
            content: chord.name.clone(),
            color: colors.foreground,
            size: 8.0.into(),
            position: Point::new(beat_position_x + 3.0, rows.chord_y()),
            ..Text::default()
        };
        frame.fill_text(note_effect_text);
    }
    if !beat.effect.stroke.is_empty() && !beat.notes.is_empty() {
        draw_stroke_arrow(frame, colors, beat, beat_position_x, measure_start_y);
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
        draw_staccato_dot(frame, colors, beat, beat_position_x, measure_start_y);
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
            measure_start_y + vertical_measure_height,
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
            measure_start_y,
            beat_position_x,
            beat_span,
            width_scale,
            note,
            beat_color,
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

#[allow(clippy::too_many_arguments)]
fn draw_note(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
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
    let note_label_len = note_label.chars().count();
    let local_beat_position_y = (f32::from(note.string) - 1.0) * STRING_LINE_HEIGHT;
    // center the notes with more than one char
    let note_position_x = beat_position_x + 3.0 - note_label_len as f32 / 2.0;
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
            color: colors.foreground,
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
    colors: TablatureColors,
    movements: &[i32],
    note_position_x: f32,
    note_position_y: f32,
    beat_position_x: f32,
    width_per_beat: f32,
    width_scale: f32,
    measure_start_y: f32,
    show_amplitude: bool,
) {
    let stroke = Stroke::default().with_width(0.8).with_color(colors.foreground);
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
                    color: colors.foreground,
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
    colors: TablatureColors,
    measure_start_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    let position_x = measure_start_x;

    // draw first thick one
    let start_point = Point::new(position_x, measure_start_y);
    let end_point = Point::new(position_x, measure_start_y + vertical_measure_height);
    let tick_vertical_line = Path::line(start_point, end_point);
    let stroke = Stroke::default().with_width(4.0).with_color(colors.foreground);
    frame.stroke(&tick_vertical_line, stroke);

    // then thin one
    draw_measure_vertical_line(
        frame,
        colors,
        vertical_measure_height,
        measure_start_x + 6.0,
        measure_start_y,
    );
}

fn draw_open_repeat(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    measure_start_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    draw_open_section(
        frame,
        colors,
        measure_start_x,
        measure_start_y,
        vertical_measure_height,
    );
    // draw repeat dots
    draw_repeat_dots(
        frame,
        colors,
        measure_start_x + HALF_BEAT_LENGTH,
        measure_start_y,
        vertical_measure_height,
    );
}

fn draw_close_repeat(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    measure_end_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
    repeat_count: i8,
) {
    draw_end_section(
        frame,
        colors,
        measure_end_x,
        measure_start_y,
        vertical_measure_height,
    );
    // draw repeat dots
    draw_repeat_dots(
        frame,
        colors,
        measure_end_x - HALF_BEAT_LENGTH,
        measure_start_y,
        vertical_measure_height,
    );
    // add repeat count text
    let repeat_count_text = Text {
        shaping: Auto,
        content: format!("x{repeat_count}"),
        color: colors.foreground,
        size: 9.0.into(),
        position: Point::new(measure_end_x - 12.0, measure_start_y - 15.0),
        ..Text::default()
    };
    frame.fill_text(repeat_count_text);
}

/// Picking direction above the staff, like TuxGuitar's `paintPickStroke`:
/// an up stroke is a `∨`, a down stroke the bracket-shaped `∏`.
fn draw_pick_stroke(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    direction: &BeatStrokeDirection,
    beat_position_x: f32,
    y: f32,
) {
    let stroke = Stroke::default().with_width(0.8).with_color(colors.foreground);
    let x = beat_position_x + 3.5;
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
            frame.stroke(
                &top_bar,
                Stroke::default().with_width(2.0).with_color(colors.foreground),
            );
        }
        BeatStrokeDirection::None => {}
    }
}

/// Staccato dot above the top note of the beat (TuxGuitar only draws it in
/// score mode; the dot above the fret number is the tab equivalent).
fn draw_staccato_dot(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    beat: &Beat,
    beat_position_x: f32,
    measure_start_y: f32,
) {
    let min_string = beat.notes.iter().map(|n| n.string).min().unwrap_or(1);
    let top_note_y = measure_start_y + (f32::from(min_string) - 1.0) * STRING_LINE_HEIGHT - 5.0;
    let center = Point::new(beat_position_x + 3.5, top_note_y - 3.0);
    frame.fill(&Path::circle(center, 1.3), colors.foreground);
}

/// Tremolo picking slashes below the tab, one per duration halving from an
/// eighth note, like TuxGuitar's tablature-only rendering.
fn draw_tremolo_picking(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    tremolo_picking: &TremoloPickingEffect,
    beat_position_x: f32,
    tab_bottom_y: f32,
) {
    let slashes = match tremolo_picking.duration.value {
        v if v >= 32 => 3,
        v if v >= 16 => 2,
        _ => 1,
    };
    let stroke = Stroke::default().with_width(1.2).with_color(colors.foreground);
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
    colors: TablatureColors,
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

    let stroke = Stroke::default().with_width(0.8).with_color(colors.foreground);

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

/// Group consecutive beats sharing a tuplet division, like TuxGuitar's
/// `paintDivisionTypes`. A run ends when the division changes or when its
/// accumulated duration completes a whole group.
///
/// Returns `(group size, first beat x, last beat x)` per run.
fn tuplet_runs(beats: &[Beat], beat_positions: &[f32]) -> Vec<(u8, f32, f32)> {
    let mut runs = Vec::new();
    let mut run: Option<TupletRun> = None;
    for (beat, &x) in beats.iter().zip(beat_positions) {
        let duration = &beat.duration;
        if let Some(current) = &run {
            let whole_group = current.shortest > 0
                && current
                    .accumulated
                    .is_multiple_of(u32::from(current.enters) * current.shortest);
            if !duration.same_tuplet_division(current.division) || whole_group {
                runs.push(current.bounds());
                run = None;
            }
        }
        if duration.is_tuplet() {
            match &mut run {
                Some(current) => current.x2 = x,
                None => run = Some(TupletRun::new(duration, x)),
            }
        }
        if let Some(current) = &mut run {
            current.extend(duration.time());
        }
    }
    if let Some(current) = &run {
        runs.push(current.bounds());
    }
    runs
}

/// A run of consecutive beats sharing one tuplet division.
struct TupletRun<'a> {
    division: &'a Duration,
    enters: u8,
    x1: f32,
    x2: f32,
    accumulated: u32,
    shortest: u32,
}

impl<'a> TupletRun<'a> {
    const fn new(division: &'a Duration, x: f32) -> Self {
        Self {
            division,
            enters: division.tuplet_enters,
            x1: x,
            x2: x,
            accumulated: 0,
            shortest: 0,
        }
    }

    const fn extend(&mut self, time: u32) {
        self.accumulated += time;
        if self.shortest == 0 || time < self.shortest {
            self.shortest = time;
        }
    }

    const fn bounds(&self) -> (u8, f32, f32) {
        (self.enters, self.x1, self.x2)
    }
}

/// A tuplet bracket: a horizontal line broken by the group size, with a
/// tick at each end pointing down towards the notes. A run covering a
/// single beat has no span to bracket, so only its label is drawn.
fn draw_tuplet_bracket(frame: &mut Frame<Renderer>,
    colors: TablatureColors, enters: u8, x1: f32, x2: f32, y: f32) {
    const TICK: f32 = 4.0;
    const LABEL_SIZE: f32 = 8.0;
    let has_span = x2 > x1;
    // the notes are centred a few pixels right of their beat position
    let left = x1 + 1.0;
    let right = x2 + 6.0;
    let center = left + (right - left) / 2.0;
    let label = enters.to_string();
    let label_half = label.chars().count() as f32 * LABEL_SIZE / 4.0;

    if has_span {
        let stroke = Stroke::default().with_width(0.8).with_color(colors.foreground);
        frame.stroke(
            &Path::line(Point::new(left, y + TICK), Point::new(left, y)),
            stroke,
        );
        frame.stroke(
            &Path::line(Point::new(right, y + TICK), Point::new(right, y)),
            stroke,
        );
        // the arms stop short of the label, and are skipped when the label
        // already fills the span
        let arm_left_end = center - label_half - 1.0;
        let arm_right_start = center + label_half + 1.0;
        if arm_left_end > left {
            frame.stroke(
                &Path::line(Point::new(left, y), Point::new(arm_left_end, y)),
                stroke,
            );
        }
        if right > arm_right_start {
            frame.stroke(
                &Path::line(Point::new(arm_right_start, y), Point::new(right, y)),
                stroke,
            );
        }
    }

    let label_text = Text {
        shaping: Auto,
        content: label,
        color: colors.foreground,
        size: LABEL_SIZE.into(),
        position: Point::new(center, y - LABEL_SIZE / 2.0),
        align_x: Alignment::Center,
        ..Text::default()
    };
    frame.fill_text(label_text);
}

fn draw_alternative_ending(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    repeat_alternative: u8,
    measure_start_x: f32,
    measure_width: f32,
    bracket_y: f32,
) {
    let bracket_height = 10.0;
    let bracket_start = measure_start_x + 2.0;
    let bracket_end = measure_start_x + measure_width;

    let stroke = Stroke::default().with_width(1.0).with_color(colors.foreground);

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
        color: colors.foreground,
        size: 9.0.into(),
        position: Point::new(bracket_start + 3.0, bracket_y),
        ..Text::default()
    };
    frame.fill_text(label_text);
}

fn draw_repeat_dots(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
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
        Stroke::default().with_width(2.0).with_color(colors.foreground),
    );

    // bottom dot
    let bottom_position_y = start_y + (vertical_measure_height / 3.0) * 2.0;
    let center = Point::new(start_x, bottom_position_y);
    let circle = Path::circle(center, 1.0);

    frame.stroke(
        &circle,
        Stroke::default().with_width(2.0).with_color(colors.foreground),
    );
}

fn draw_end_section(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    measure_end_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    // draw first thin one
    draw_measure_vertical_line(
        frame,
        colors,
        vertical_measure_height,
        measure_end_x - 8.0,
        measure_start_y,
    );

    // then thick one
    let position_x = measure_end_x - 2.0;
    let start_point = Point::new(position_x, measure_start_y);
    let end_point = Point::new(position_x, measure_start_y + vertical_measure_height);
    let thick_vertical_line = Path::line(start_point, end_point);
    let stroke = Stroke::default().with_width(4.0).with_color(colors.foreground);
    frame.stroke(&thick_vertical_line, stroke);
}

fn draw_time_signature(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    time_signature: &TimeSignature,
    measure_start_x: f32,
    measure_start_y: f32,
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
        color: colors.foreground,
        size: 17.into(),
        position: Point::new(
            measure_start_x + position_x,
            (measure_start_y - 1.0) + position_y,
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
        annotation.push(HAMMER_ON);
    }
    if let Some(slide) = &note_effect.slide {
        annotation.push(match slide {
            SlideType::IntoFromAbove | SlideType::IntoFromBelow | SlideType::OutDownwards => {
                HORIZONTAL_BAR
            }
            SlideType::ShiftSlideTo => SHIFT_SLIDE,
            SlideType::LegatoSlideTo | SlideType::OutUpWards => LEGATO_SLIDE,
        });
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
        NoteType::Tie => TIE.into(),
        NoteType::Dead => "x".to_string(),
        NoteType::Unknown(i) => {
            log::warn!("NoteType Unknown({i})");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::song_parser::QUARTER;

    fn beat(value: u16, enters: u8, times: u8) -> Beat {
        Beat {
            duration: Duration {
                value,
                dotted: false,
                double_dotted: false,
                tuplet_enters: enters,
                tuplet_times: times,
            },
            ..Beat::default()
        }
    }

    #[test]
    fn no_run_without_tuplets() {
        let beats = vec![beat(QUARTER, 1, 1), beat(QUARTER, 1, 1)];
        assert!(tuplet_runs(&beats, &[0.0, 10.0]).is_empty());
    }

    #[test]
    fn triplet_of_eighths_is_one_run() {
        // three eighth notes in the time of two: one bracket labelled 3
        let beats = vec![beat(8, 3, 2), beat(8, 3, 2), beat(8, 3, 2)];
        let runs = tuplet_runs(&beats, &[0.0, 10.0, 20.0]);
        assert_eq!(runs, vec![(3, 0.0, 20.0)]);
    }

    #[test]
    fn consecutive_triplets_are_separate_runs() {
        // a complete group closes the bracket, so six eighths make two
        let beats: Vec<Beat> = (0..6).map(|_| beat(8, 3, 2)).collect();
        let positions: Vec<f32> = (0..6).map(|i| i as f32 * 10.0).collect();
        let runs = tuplet_runs(&beats, &positions);
        assert_eq!(runs, vec![(3, 0.0, 20.0), (3, 30.0, 50.0)]);
    }

    #[test]
    fn a_normal_beat_closes_the_run() {
        let beats = vec![
            beat(8, 3, 2),
            beat(8, 3, 2),
            beat(QUARTER, 1, 1),
            beat(8, 3, 2),
        ];
        let runs = tuplet_runs(&beats, &[0.0, 10.0, 20.0, 30.0]);
        assert_eq!(runs, vec![(3, 0.0, 10.0), (3, 30.0, 30.0)]);
    }

    #[test]
    fn changing_division_closes_the_run() {
        let beats = vec![beat(8, 3, 2), beat(16, 5, 4), beat(16, 5, 4)];
        let runs = tuplet_runs(&beats, &[0.0, 10.0, 20.0]);
        assert_eq!(runs, vec![(3, 0.0, 0.0), (5, 10.0, 20.0)]);
    }

    #[test]
    fn quintuplet_of_sixteenths_is_one_run() {
        let beats: Vec<Beat> = (0..5).map(|_| beat(16, 5, 4)).collect();
        let positions: Vec<f32> = (0..5).map(|i| i as f32 * 10.0).collect();
        assert_eq!(tuplet_runs(&beats, &positions), vec![(5, 0.0, 40.0)]);
    }
}
