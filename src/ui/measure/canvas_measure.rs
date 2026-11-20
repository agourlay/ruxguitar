//! The canvas widget drawing one measure of a track, and its click handling.

use super::beat::{BeatSlot, draw_annotations, draw_beat};
use super::columns::{
    Column, column_annotations, column_at_tick, measure_columns, separated_drum_rows, voice_beats,
};
use super::layout::{
    BEAT_LENGTH, HALF_BEAT_LENGTH, MEASURE_NOTES_PADDING, MIN_BEAT_WIDTH, MIN_MEASURE_WIDTH,
    RowSpacing, SECOND_RHYTHM_BAND, Staff, column_natural_width, lead_in_width, measure_height,
    spacing_for_quarter,
};
use super::rhythm::{draw_rhythm, draw_tuplet_bracket, tuplet_runs};
use super::staff::{
    BarEnd, BarStart, draw_alternative_ending, draw_bar_end, draw_bar_start, draw_focused_box,
    draw_selection_fill, draw_string_lines, draw_time_signature,
};
use crate::parser::song_parser::{KeySignature, MeasureHeader, Song, TripletFeel};
use crate::ui::application::Message;
use crate::ui::utils::TablatureColors;
use iced::advanced::mouse;
use iced::advanced::text::Shaping::Auto;
use iced::mouse::Cursor;
use iced::widget::canvas::{Cache, Event, Frame, Geometry, Text};
use iced::widget::{Action, Canvas, canvas};
use iced::{Element, Length, Point, Rectangle, Renderer, Theme};
use std::cell::Cell;
use std::collections::HashMap;
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
    /// The focused column: the beats of every voice starting together.
    focused_column: usize,
    canvas_cache: Cache,
    measure_len: f32,
    /// The moments the beats of the measure start, shared by its voices.
    columns: Vec<Column>,
    /// On a drum track, the rows of the other voices' beats with a note
    /// moved off a row already used in its column, by (voice, beat).
    drum_rows: HashMap<(usize, usize), Vec<i8>>,
    // natural width of each column and their sum (immutable song data,
    // computed once instead of on every redraw)
    column_widths: Vec<f32>,
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
        let columns = measure_columns(measure);
        // the measure is spaced by its shortest note, of any voice drawn. A
        // beat flagged empty carries no time, so it has no say, as
        // TuxGuitar's own skips it. The fallback is for a measure with
        // nothing to space by: a floor here would widen the long notes it is
        // meant to protect
        let quarter_spacing = columns
            .iter()
            .flat_map(|column| &column.beats)
            .map(|&(voice, beat)| &measure.voices[voice].beats[beat])
            .filter(|beat| !beat.empty)
            .map(|beat| spacing_for_quarter(&beat.duration))
            .fold(0.0_f32, f32::max);
        let quarter_spacing = if quarter_spacing > 0.0 {
            quarter_spacing
        } else {
            MIN_BEAT_WIDTH
        };
        let is_drums = song
            .midi_channels
            .iter()
            .any(|c| c.channel_id == track.channel_id && c.is_percussion());
        let drum_rows = if is_drums {
            separated_drum_rows(measure, &columns, track.strings.len())
        } else {
            HashMap::new()
        };
        let column_widths: Vec<f32> = (0..columns.len())
            .map(|index| column_natural_width(measure, &columns, index, quarter_spacing, &lyrics))
            .collect();
        let natural_beats_len: f32 = column_widths.iter().sum();
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
            focused_column: 0,
            canvas_cache: Cache::default(),
            measure_len,
            columns,
            drum_rows,
            column_widths,
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

    #[cfg(test)]
    pub const fn is_selected(&self) -> bool {
        self.is_selected
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
        self.focused_column = 0;
        self.canvas_cache.clear();
    }

    /// The focused column, the beat index the rest of the app speaks of.
    pub const fn focused_beat(&self) -> usize {
        self.focused_column
    }

    pub fn focus_beat(&mut self, column: usize) {
        if self.focused_column != column {
            self.focused_column = column;
            self.canvas_cache.clear();
        }
    }

    /// The column playing at `tick`, an absolute tick of the measure.
    pub fn column_at_tick(&self, tick: u32) -> usize {
        column_at_tick(&self.columns, tick)
    }

    /// The column's start, as an absolute tick of the measure.
    pub fn column_start(&self, column: usize) -> Option<u32> {
        self.columns.get(column).map(|column| column.start)
    }

    pub fn clear_canvas_cache(&self) {
        self.canvas_cache.clear();
    }

    /// The column under the given x position, mirroring the layout used by
    /// `draw` (clicks in the leading padding select the first column, clicks
    /// past the last column the last one).
    pub(super) fn beat_at_x(&self, x: f32, actual_width: f32) -> usize {
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
        for (column, width) in self.column_widths.iter().enumerate() {
            beat_x += width * width_scale;
            if x < beat_x {
                return column;
            }
        }
        // past the last column: the nearest visible one, never a zero-width
        // column of empty beats that may close the measure
        let measure = &self.song.tracks[self.track_id].measures[self.measure_id];
        self.columns
            .iter()
            .rposition(|column| {
                column
                    .beats
                    .iter()
                    .any(|&(voice, beat)| !measure.voices[voice].beats[beat].empty)
            })
            .unwrap_or_else(|| self.column_widths.len().saturating_sub(1))
    }
}

impl CanvasMeasure {
    /// Paint the measure: its staff, labels, beats and bar lines, across
    /// the width it was actually given.
    fn paint(&self, frame: &mut Frame<Renderer>, colors: TablatureColors) {
        log::debug!("Re-drawing measure {}", self.measure_id);
        let string_count = self.song.tracks[self.track_id].strings.len();
        let header = &self.song.measure_headers[self.measure_id];
        let is_last = self.measure_id + 1 == self.song.measure_headers.len();
        // may be wider than total_measure_len, when the measure fills its line
        let width = frame.width();
        let staff = Staff::new(self.row_spacing, string_count);

        if self.is_selected {
            draw_selection_fill(frame, colors, staff, width);
        }
        if self.is_focused {
            draw_focused_box(frame, colors, staff, width);
        }
        draw_string_lines(frame, colors, staff, string_count, width);
        let bar_start = BarStart::of(header, self.measure_id, self.is_first_on_line);
        draw_bar_start(frame, colors, bar_start, 0.0, staff.top, staff.height);
        if self.has_time_signature {
            draw_time_signature(
                frame,
                colors,
                &header.time_signature,
                0.0,
                staff.top,
                string_count,
                header.repeat_open, // need to offset if repeat dots present
            );
        }
        self.draw_labels(frame, colors, staff, header);
        // alternative ending bracket (e.g., "1.", "2.", "1.2.")
        if header.repeat_alternative > 0 {
            draw_alternative_ending(
                frame,
                colors,
                header.repeat_alternative,
                0.0,
                width,
                RowSpacing::ALT_ENDING_Y,
            );
        }
        self.draw_beats(frame, colors, staff, header, width);
        let bar_end = BarEnd::of(header, is_last);
        draw_bar_end(frame, colors, bar_end, width, staff.top, staff.height);
    }

    /// The tempo, marker, key and feel above the staff, and the measure number.
    fn draw_labels(
        &self,
        frame: &mut Frame<Renderer>,
        colors: TablatureColors,
        staff: Staff,
        header: &MeasureHeader,
    ) {
        let rows = staff.rows;
        // the marker follows the tempo label on the same row
        let mut tempo_label_len = 0;
        if self.has_tempo_label {
            let tempo_label = format!("{TEMPO_SIGN} = {}", header.tempo.value);
            tempo_label_len = tempo_label.chars().count() * 10;
            frame.fill_text(Text {
                shaping: Auto,
                content: tempo_label,
                color: colors.foreground,
                size: 11.0.into(),
                position: Point::new(0.0, rows.marker_y()),
                ..Text::default()
            });
        }
        if let Some(marker) = &header.marker {
            frame.fill_text(Text {
                shaping: Auto,
                content: marker.title.clone(),
                color: colors.accent,
                size: 10.0.into(),
                position: Point::new(
                    MEASURE_NOTES_PADDING + tempo_label_len as f32,
                    rows.marker_y(),
                ),
                ..Text::default()
            });
        }
        // key signature and feel, at the measure that establishes them
        if self.has_key_signature || self.has_triplet_feel {
            let mut labels = Vec::new();
            if self.has_key_signature {
                labels.push(header.key_signature.to_string());
            }
            if self.has_triplet_feel {
                labels.push(triplet_feel_label(header.triplet_feel).to_string());
            }
            frame.fill_text(Text {
                shaping: Auto,
                content: labels.join("  "),
                color: colors.foreground,
                size: 9.0.into(),
                position: Point::new(2.0, rows.signature_y()),
                ..Text::default()
            });
        }
        frame.fill_text(Text {
            shaping: Auto,
            content: format!("{}", self.measure_id + 1),
            color: colors.accent,
            size: 10.0.into(),
            position: Point::new(0.0, staff.top - 15.0),
            ..Text::default()
        });
    }

    /// The beats of every voice with their lyrics, then each voice's rhythm
    /// and tuplets under the staff.
    fn draw_beats(
        &self,
        frame: &mut Frame<Renderer>,
        colors: TablatureColors,
        staff: Staff,
        header: &MeasureHeader,
        width: f32,
    ) {
        let measure = &self.song.tracks[self.track_id].measures[self.measure_id];
        log::debug!("{} columns", self.columns.len());
        let mut beat_start = 0.0;
        if self.has_time_signature {
            beat_start += BEAT_LENGTH;
        }
        if header.repeat_open {
            beat_start += BEAT_LENGTH;
        }
        // the extra width the measure was given goes to its beats
        let width_scale = if self.natural_beats_len > 0.0 {
            (width - self.overhead_width()) / self.natural_beats_len
        } else {
            1.0
        };
        // every position is known before any beat is drawn: slides and
        // hammer-ons reach for the next note on their string
        let column_x = beat_x_positions(
            beat_start + MEASURE_NOTES_PADDING,
            &self.column_widths,
            width_scale,
        );
        // the room after a column, up to the grace notes and slide-ins the
        // next one draws before itself
        let column_span = |column: usize| {
            let beat_width = self.column_widths[column] * width_scale;
            let next_lead_in = self.columns.get(column + 1).map_or(0.0, |next| {
                next.beats
                    .iter()
                    .map(|&(voice, beat)| lead_in_width(&measure.voices[voice].beats[beat]))
                    .fold(0.0, f32::max)
            });
            beat_width - next_lead_in
        };

        for (voice_index, voice) in measure.voices.iter().enumerate() {
            let positions = self.voice_positions(voice_index, &column_x);
            for (index, _, column) in voice_beats(measure, &self.columns, voice_index) {
                let slot = BeatSlot {
                    beats: &voice.beats,
                    positions: &positions,
                    index,
                    span: column_span(column),
                    width_scale,
                    color: if self.is_focused && column == self.focused_column {
                        colors.accent
                    } else {
                        colors.foreground
                    },
                    rows: self
                        .drum_rows
                        .get(&(voice_index, index))
                        .map_or(&[], Vec::as_slice),
                };
                draw_beat(frame, colors, staff, &slot);
                // the syllable sung on this beat, under the staff
                if voice_index == 0
                    && let Some(syllable) = self.lyrics.get(index)
                    && !syllable.is_empty()
                {
                    frame.fill_text(Text {
                        shaping: Auto,
                        content: syllable.clone(),
                        color: colors.foreground,
                        size: 8.0.into(),
                        position: Point::new(
                            positions[index],
                            staff.bottom() + staff.rows.staff_footer + staff.rows.second_rhythm,
                        ),
                        ..Text::default()
                    });
                }
            }
            // the rhythm under the staff, once every beat position is known:
            // the first voice's, then the others' in their band below it
            let rhythm_top = if voice_index == 0 {
                staff.bottom()
            } else {
                staff.bottom() + staff.rows.staff_footer
            };
            draw_rhythm(frame, colors, &voice.beats, &positions, rhythm_top, header);
            // tuplet brackets span consecutive beats of the same division:
            // above the staff for the first voice, under its band for others
            let tuplet_y = if voice_index == 0 {
                (staff.rows.tuplet > 0.0).then(|| staff.rows.tuplet_y())
            } else {
                (staff.rows.second_rhythm > SECOND_RHYTHM_BAND)
                    .then_some(rhythm_top + SECOND_RHYTHM_BAND)
            };
            if let Some(y) = tuplet_y {
                for (enters, x1, x2) in tuplet_runs(&voice.beats, &positions) {
                    draw_tuplet_bracket(frame, colors, enters, x1, x2, y);
                }
            }
        }

        // the effects above the staff, once per column for all its voices
        for (column_index, column) in self.columns.iter().enumerate() {
            let annotations = column_annotations(measure, column);
            if !annotations.is_empty() {
                draw_annotations(
                    frame,
                    colors,
                    staff,
                    column_x[column_index],
                    column_span(column_index),
                    &annotations,
                );
            }
        }
    }

    /// Where each beat of a voice starts: at its column, or for a beat not
    /// drawn (a rest of a voice past the first) where its tick falls between
    /// the columns around it, so neighbouring notes still find their way.
    fn voice_positions(&self, voice: usize, column_x: &[f32]) -> Vec<f32> {
        let measure = &self.song.tracks[self.track_id].measures[self.measure_id];
        measure.voices[voice]
            .beats
            .iter()
            .enumerate()
            .map(|(index, beat)| {
                if let Some(column) = self
                    .columns
                    .iter()
                    .position(|column| column.beats.contains(&(voice, index)))
                {
                    return column_x[column];
                }
                let at = column_at_tick(&self.columns, beat.start);
                let (Some(column), Some(&x)) = (self.columns.get(at), column_x.get(at)) else {
                    return column_x.first().copied().unwrap_or(0.0);
                };
                match (self.columns.get(at + 1), column_x.get(at + 1)) {
                    (Some(next), Some(&next_x)) if next.start > column.start => {
                        let progress = beat.start.saturating_sub(column.start) as f32
                            / (next.start - column.start) as f32;
                        x + (next_x - x) * progress.min(1.0)
                    }
                    _ => x,
                }
            })
            .collect()
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
        let tab = self
            .canvas_cache
            .draw(renderer, bounds.size(), |frame| self.paint(frame, colors));
        vec![tab]
    }
}

/// Where each beat starts: one after the other from `start`, each as wide
/// as its natural width scaled to the room the measure was given.
pub(super) fn beat_x_positions(start: f32, natural_widths: &[f32], width_scale: f32) -> Vec<f32> {
    natural_widths
        .iter()
        .scan(start, |x, width| {
            let position = *x;
            *x += width * width_scale;
            Some(position)
        })
        .collect()
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
