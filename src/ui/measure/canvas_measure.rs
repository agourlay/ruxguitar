//! The canvas widget drawing one measure of a track, and its click handling.

use super::beat::draw_beat;
use super::layout::{
    BEAT_LENGTH, HALF_BEAT_LENGTH, MEASURE_NOTES_PADDING, MIN_BEAT_WIDTH, MIN_MEASURE_WIDTH,
    RowSpacing, STRING_LINE_HEIGHT, beat_base_width, beat_natural_width, grace_gap_width,
    lyric_extra_width, measure_height, spacing_for_quarter,
};
use super::rhythm::{draw_rhythm, draw_tuplet_bracket, tuplet_runs};
use super::staff::{
    BarEnd, BarStart, draw_alternative_ending, draw_bar_end, draw_bar_start, draw_focused_box,
    draw_selection_fill, draw_time_signature,
};
use crate::parser::song_parser::{KeySignature, MeasureHeader, Song, TripletFeel};
use crate::ui::application::Message;
use crate::ui::utils::TablatureColors;
use iced::advanced::mouse;
use iced::advanced::text::Shaping::Auto;
use iced::mouse::Cursor;
use iced::widget::canvas::{Cache, Event, Geometry, Path, Stroke, Text};
use iced::widget::{Action, Canvas, canvas};
use iced::{Element, Length, Point, Rectangle, Renderer, Theme};
use std::cell::Cell;
use std::rc::Rc;

// Unicode symbols for musical notation
const TEMPO_SIGN: char = '\u{1D15F}'; // 𝅗𝅥 https://unicodeplus.com/U+1D15F

#[derive(Debug)]
pub struct CanvasMeasure {
    pub measure_id: usize,
    track_id: usize,
    song: Rc<Song>,
    is_focused: bool,
    is_selected: bool,
    pointer_drag_active: bool,
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
    has_key_signature: bool,
    has_triplet_feel: bool,
    /// One syllable per beat, empty where the beat carries none.
    lyrics: Vec<String>,
}

impl CanvasMeasure {
    pub fn new(
        measure_id: usize,
        track_id: usize,
        song: Rc<Song>,
        focused: bool,
        has_time_signature: bool,
        lyrics: Vec<String>,
    ) -> Self {
        let track = &song.tracks[track_id];
        let measure = &track.measures[measure_id];
        let measure_header = &song.measure_headers[measure_id];
        let beats = &measure.voices[0].beats;
        // the measure is spaced by its shortest note. A beat flagged empty
        // carries no time, so it has no say, as TuxGuitar's own skips it.
        // The fallback is for a measure with nothing to space by: a floor
        // here would widen the long notes it is meant to protect
        let quarter_spacing = beats
            .iter()
            .filter(|beat| !beat.empty)
            .map(|beat| spacing_for_quarter(&beat.duration))
            .fold(0.0_f32, f32::max);
        let quarter_spacing = if quarter_spacing > 0.0 {
            quarter_spacing
        } else {
            MIN_BEAT_WIDTH
        };
        let beat_widths: Vec<f32> = beats
            .iter()
            .enumerate()
            .map(|(i, beat)| {
                beat_natural_width(beat, beats.get(i + 1), quarter_spacing)
                    + lyric_extra_width(&lyrics, i, beat_base_width(beat, quarter_spacing))
            })
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
        // the tempo, key and feel are shown on the first measure and again
        // wherever they change
        let previous_header = measure_id.checked_sub(1).map(|p| &song.measure_headers[p]);
        let has_tempo_label =
            previous_header.is_none_or(|previous| measure_header.tempo != previous.tempo);
        let has_key_signature = shows_key_signature(measure_header, previous_header);
        let has_triplet_feel = previous_header.map_or(
            measure_header.triplet_feel != TripletFeel::None,
            |previous| measure_header.triplet_feel != previous.triplet_feel,
        );
        let has_lyrics = lyrics.iter().any(|syllable| !syllable.is_empty());
        let row_needs = RowSpacing::for_measure(
            measure,
            measure_header,
            has_tempo_label,
            has_key_signature || has_triplet_feel,
            has_lyrics,
        );
        let vertical_measure_height = measure_height(row_needs, string_count);
        Self {
            measure_id,
            track_id,
            song,
            is_focused: focused,
            is_selected: false,
            pointer_drag_active: false,
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
            has_key_signature,
            has_triplet_feel,
            lyrics,
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

    pub fn set_selected(&mut self, selected: bool) {
        if self.is_selected != selected {
            self.is_selected = selected;
            self.canvas_cache.clear();
        }
    }

    pub fn set_pointer_drag_active(&mut self, active: bool) {
        self.pointer_drag_active = active;
    }

    pub fn toggle_focused(&mut self) {
        self.is_focused = !self.is_focused;
        self.focused_beat = 0;
        self.canvas_cache.clear();
    }

    pub const fn focused_beat(&self) -> usize {
        self.focused_beat
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
        let beats = &self.song.tracks[self.track_id].measures[self.measure_id].voices[0].beats;
        for (beat_id, width) in self.beat_widths.iter().enumerate() {
            beat_x += width * width_scale;
            if x < beat_x {
                return beat_id;
            }
        }
        // past the last beat: the nearest visible one, never the zero-width
        // empty beat that may close the voice
        beats
            .iter()
            .rposition(|beat| !beat.empty)
            .unwrap_or_else(|| self.beat_widths.len().saturating_sub(1))
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
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.position_in(bounds).is_some() =>
            {
                let cursor_position = cursor.position_in(bounds).unwrap();
                let beat_id = self.beat_at_x(cursor_position.x, bounds.width);
                let press_position = cursor.position().expect("cursor inside measure bounds");
                *state = MeasureInteraction::Clicked;
                return Some(Action::publish(Message::SelectionDragStart(
                    self.measure_id,
                    beat_id,
                    press_position,
                )));
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
                if self.pointer_drag_active
                    && cursor.position_in(bounds).is_some()
                    && let Some(cursor_position) = cursor.position() =>
            {
                return Some(Action::publish(Message::SelectionDragTo(
                    self.measure_id,
                    cursor_position,
                )));
            }
            _ => {}
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

            if self.is_selected {
                draw_selection_fill(
                    frame,
                    colors,
                    actual_width,
                    vertical_measure_height,
                    measure_start_x,
                    measure_start_y,
                    rows,
                );
            }

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
                let stroke = Stroke::default()
                    .with_width(0.8)
                    .with_color(colors.string_line);
                frame.stroke(&line, stroke);
            }

            // measure headers
            let measure_header = &self.song.measure_headers[self.measure_id];
            let next_measure_header = self.song.measure_headers.get(self.measure_id + 1);

            let bar_start = BarStart::of(measure_header, self.measure_id, self.is_first_on_line);
            draw_bar_start(
                frame,
                colors,
                bar_start,
                measure_start_x,
                measure_start_y,
                vertical_measure_height,
            );

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

            // key signature and feel, at the measure that establishes them
            if self.has_key_signature || self.has_triplet_feel {
                let mut labels = Vec::new();
                if self.has_key_signature {
                    labels.push(measure_header.key_signature.to_string());
                }
                if self.has_triplet_feel {
                    labels.push(triplet_feel_label(measure_header.triplet_feel).to_string());
                }
                let signature_text = Text {
                    shaping: Auto,
                    content: labels.join("  "),
                    color: colors.foreground,
                    size: 9.0.into(),
                    position: Point::new(measure_start_x + 2.0, rows.signature_y()),
                    ..Text::default()
                };
                frame.fill_text(signature_text);
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
                    beats,
                    &beat_positions,
                    b_id,
                );
                // the syllable sung on this beat, under the staff
                if let Some(syllable) = self.lyrics.get(b_id)
                    && !syllable.is_empty()
                {
                    let lyric_text = Text {
                        shaping: Auto,
                        content: syllable.clone(),
                        color: colors.foreground,
                        size: 8.0.into(),
                        position: Point::new(
                            beat_position_x,
                            measure_start_y + vertical_measure_height + rows.staff_footer,
                        ),
                        ..Text::default()
                    };
                    frame.fill_text(lyric_text);
                }
                beat_positions.push(beat_position_x);
                beat_position_x += beat_width;
            }

            // the rhythm under the staff, once every beat position is known
            draw_rhythm(
                frame,
                colors,
                beats,
                &beat_positions,
                measure_start_y + vertical_measure_height,
                measure_header,
            );

            // tuplet brackets span consecutive beats of the same division
            if rows.tuplet > 0.0 {
                for (enters, x1, x2) in tuplet_runs(beats, &beat_positions) {
                    draw_tuplet_bracket(frame, colors, enters, x1, x2, rows.tuplet_y());
                }
            }

            let bar_end = BarEnd::of(measure_header, next_measure_header.is_none());
            draw_bar_end(
                frame,
                colors,
                bar_end,
                measure_start_x + actual_width,
                measure_start_y,
                vertical_measure_height,
            );
        });

        vec![tab]
    }
}

/// Whether a measure states its key: when it changes, or when a song opens
/// in something other than the default key everything starts in.
pub(super) fn shows_key_signature(
    header: &MeasureHeader,
    previous: Option<&MeasureHeader>,
) -> bool {
    previous.map_or(
        header.key_signature != KeySignature::new(0, false),
        |previous| header.key_signature != previous.key_signature,
    )
}

/// How a change of feel reads: TuxGuitar draws note pictograms, which the
/// tab spells out instead.
const fn triplet_feel_label(feel: TripletFeel) -> &'static str {
    match feel {
        TripletFeel::None => "straight",
        TripletFeel::Eighth => "swing 8th",
        TripletFeel::Sixteenth => "swing 16th",
    }
}
