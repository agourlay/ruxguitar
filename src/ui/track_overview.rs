//! The multitrack overview under the tablature: a row per track, a square
//! per measure, to see where each instrument plays and jump there.
//!
//! Like TuxGuitar's track table (`TGTableViewer`), a square takes its track's
//! color, or a muted one when the measure only rests; the measure being
//! played gets an inner marker, and a click moves to that track and measure.

use crate::parser::song_parser::{Measure, Song};
use crate::ui::application::Message;
use crate::ui::utils::{COLOR_DARK_RED, TablatureColors, mix};
use iced::advanced::mouse;
use iced::advanced::text::Shaping::Auto;
use iced::mouse::Cursor;
use iced::widget::canvas::{Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::text::Wrapping;
use iced::widget::{
    Action, Canvas, Id, button, canvas, column, container, row, scrollable, space, text,
};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use std::cell::Cell;

/// The side of a measure's square.
const CELL: f32 = 14.0;
/// From one square to the next, gap included.
const STEP: f32 = CELL + 1.0;
/// The band above the squares holding measure numbers and markers.
const HEADER: f32 = 14.0;
const LABEL_WIDTH: f32 = 170.0;
const LABEL_SIZE: f32 = 11.0;
/// Rows shown before the overview scrolls vertically.
const MAX_VISIBLE_ROWS: usize = 6;
/// The horizontal scrollbar, drawn over the bottom of the squares.
const SCROLLBAR_WIDTH: f32 = 10.0;
/// Measures kept visible past the played one while following playback.
pub const PLAYING_MARGIN: usize = 3;

pub struct TrackOverview {
    /// Per track, per measure, whether the measure only rests.
    rests: Vec<Vec<bool>>,
    colors: Vec<Color>,
    labels: Vec<String>,
    /// Per measure, whether it has a marker.
    markers: Vec<bool>,
    cells_cache: Cache,
    painted_colors: Cell<Option<TablatureColors>>,
    squares_id: Id,
    labels_id: Id,
    /// Where the squares are scrolled and how wide they show, once known.
    squares: Option<SquaresView>,
    /// Where the labels are scrolled, which follows the squares.
    labels_y: f32,
    /// The measure the scroll last followed.
    followed: Option<usize>,
}

impl TrackOverview {
    pub fn new(song: &Song) -> Self {
        Self {
            rests: song
                .tracks
                .iter()
                .map(|track| track.measures.iter().map(is_rest_measure).collect())
                .collect(),
            colors: song.tracks.iter().map(|t| track_color(t.color)).collect(),
            labels: song
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| format!("{} {}", index + 1, track.name))
                .collect(),
            markers: song
                .measure_headers
                .iter()
                .map(|header| header.marker.is_some())
                .collect(),
            cells_cache: Cache::default(),
            painted_colors: Cell::new(None),
            squares_id: Id::unique(),
            labels_id: Id::unique(),
            squares: None,
            labels_y: 0.0,
            followed: None,
        }
    }

    fn measure_count(&self) -> usize {
        self.markers.len()
    }

    /// Forget where the squares stand, for when they show again and
    /// start from the left.
    pub fn reset_view(&mut self) {
        self.squares = None;
        self.labels_y = 0.0;
        self.followed = None;
    }

    /// The squares were scrolled: keep where, and the labels' offset to
    /// line them up with the rows, when it moved vertically.
    pub fn squares_scrolled(&mut self, viewport: Viewport) -> Option<(Id, AbsoluteOffset)> {
        let offset = viewport.absolute_offset();
        let width = viewport.bounds().width;
        self.squares_at(SquaresView {
            x: offset.x,
            y: offset.y,
            width,
            max_x: (viewport.content_bounds().width - width).max(0.0),
        })
    }

    fn squares_at(&mut self, view: SquaresView) -> Option<(Id, AbsoluteOffset)> {
        self.squares = Some(view);
        ((view.y - self.labels_y).abs() > 0.5).then(|| {
            self.labels_y = view.y;
            (self.labels_id.clone(), AbsoluteOffset { x: 0.0, y: view.y })
        })
    }

    /// The labels were scrolled: the squares' offset to line them up with
    /// the labels, when it differs.
    pub fn labels_scrolled(&mut self, viewport: Viewport) -> Option<(Id, AbsoluteOffset)> {
        self.labels_at(viewport.absolute_offset().y)
    }

    fn labels_at(&mut self, y: f32) -> Option<(Id, AbsoluteOffset)> {
        self.labels_y = y;
        let squares = self.squares.as_mut()?;
        ((squares.y - y).abs() > 0.5).then(|| {
            squares.y = y;
            (self.squares_id.clone(), AbsoluteOffset { x: squares.x, y })
        })
    }

    /// The squares' offset bringing `measure` into view, once the view is
    /// known and the measure is not the one already followed.
    pub fn follow(&mut self, measure: usize, margin: usize) -> Option<(Id, AbsoluteOffset)> {
        let squares = self.squares.as_mut()?;
        if self.followed == Some(measure) {
            return None;
        }
        self.followed = Some(measure);
        // the scrollable stops at its end, and reports nothing when that is
        // where it already stands
        let x = follow_offset(measure, squares.x, squares.width, margin)?.min(squares.max_x);
        squares.x = x;
        Some((self.squares_id.clone(), AbsoluteOffset { x, y: squares.y }))
    }

    pub fn view(
        &self,
        track: usize,
        focused: usize,
        selection: Option<(usize, usize)>,
    ) -> Element<'_, Message> {
        let labels = self.labels.iter().enumerate().map(|(index, label)| {
            let is_current = index == track;
            button(
                text(label)
                    .size(LABEL_SIZE)
                    .shaping(Auto)
                    .wrapping(Wrapping::None),
            )
            .padding([0, 6])
            .width(Length::Fill)
            .height(STEP)
            .clip(true)
            .on_press(Message::TrackOverviewTrack(index))
            .style(move |theme: &Theme, status| {
                if is_current {
                    button::primary(theme, status)
                } else {
                    button::text(theme, status)
                }
            })
            .into()
        });
        let rows = self.labels.len();
        let visible_rows = rows.min(MAX_VISIBLE_ROWS) as f32;
        // the labels show as high as the squares above their scrollbar
        let rows_height = HEADER + visible_rows * STEP;
        let scrollbar = Scrollbar::default()
            .width(SCROLLBAR_WIDTH)
            .scroller_width(SCROLLBAR_WIDTH)
            .spacing(0.0);
        // the vertical scrollbar sits by the labels: beside the squares it
        // would leave a corner with the horizontal one, showing a hidden row
        let labels = scrollable(column![space().height(HEADER), column(labels)])
            .id(self.labels_id.clone())
            .direction(Direction::Vertical(scrollbar))
            .on_scroll(Message::TrackOverviewLabelsScrolled)
            .width(LABEL_WIDTH)
            .height(rows_height);

        let squares = Canvas::new(OverviewCanvas {
            overview: self,
            track,
            focused,
            selection,
        })
        .width(self.measure_count() as f32 * STEP)
        // room under the last row to scroll it above the scrollbar, so the
        // squares scroll as far as the labels
        .height(HEADER + rows as f32 * STEP + SCROLLBAR_WIDTH);
        let squares = scrollable(squares)
            .id(self.squares_id.clone())
            .direction(Direction::Both {
                vertical: Scrollbar::hidden(),
                horizontal: scrollbar,
            })
            .on_scroll(Message::TrackOverviewScrolled)
            .width(Length::Fill);

        container(row![labels, squares].spacing(6))
            .height(rows_height + SCROLLBAR_WIDTH)
            .into()
    }
}

/// Where the squares are scrolled, and how wide they show.
#[derive(Debug, Clone, Copy)]
struct SquaresView {
    x: f32,
    y: f32,
    width: f32,
    /// The furthest the squares scroll.
    max_x: f32,
}

/// Whether no voice of the measure plays a note.
fn is_rest_measure(measure: &Measure) -> bool {
    measure
        .voices
        .iter()
        .all(|voice| voice.beats.iter().all(|beat| beat.notes.is_empty()))
}

/// A track's color from its 0xRRGGBB value; files without one (GP1, GP2,
/// some GPX) store 0, shown in the tablature's accent rather than black.
fn track_color(rgb: i32) -> Color {
    if rgb == 0 {
        return COLOR_DARK_RED;
    }
    let [_, r, g, b] = rgb.to_be_bytes();
    Color::from_rgb8(r, g, b)
}

/// A track color as shown on `background`: one too close to it, like white
/// on the light theme or black on the dark one, is blended with the text.
fn visible_color(track: Color, background: Color, text: Color) -> Color {
    // by channel: a saturated color stands out even as dark as the background
    let distance = (track.r - background.r)
        .abs()
        .max((track.g - background.g).abs())
        .max((track.b - background.b).abs());
    if distance < 0.15 {
        mix(track, text, 0.5)
    } else {
        track
    }
}

/// The marker color showing on a square: dark on a bright one, light on a
/// dark one, with TuxGuitar's threshold.
fn marker_color(square: Color) -> Color {
    let [r, g, b, _] = square.into_rgba8();
    if u32::from(r) + u32::from(g) + u32::from(b) > 3 * 0x50 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// The `(track, measure)` of the square under a point of the canvas.
fn cell_at(point: Point, tracks: usize, measures: usize) -> Option<(usize, usize)> {
    if point.x < 0.0 || point.y < HEADER {
        return None;
    }
    let measure = (point.x / STEP) as usize;
    let track = ((point.y - HEADER) / STEP) as usize;
    (track < tracks && measure < measures).then_some((track, measure))
}

/// The horizontal offset bringing `measure` into a view at `offset` and
/// `width` wide, keeping `margin` measures visible after it, or `None` when
/// it is visible already. Ported from TuxGuitar's `followHorizontalScroll`.
fn follow_offset(measure: usize, offset: f32, width: f32, margin: usize) -> Option<f32> {
    let position = measure as f32 * STEP;
    // a narrow view keeps a smaller margin, so it does not jump back and forth
    let mut margin = margin;
    while margin > 0 && width < 4.0 * margin as f32 * STEP {
        margin -= 1;
    }
    if position < offset {
        Some(position)
    } else if position + (1 + margin) as f32 * STEP > offset + width {
        Some((position - margin as f32 * STEP).max(0.0))
    } else {
        None
    }
}

/// The overview's squares, with what changes as the song plays.
struct OverviewCanvas<'a> {
    overview: &'a TrackOverview,
    track: usize,
    focused: usize,
    selection: Option<(usize, usize)>,
}

impl OverviewCanvas<'_> {
    /// The header and the squares, which only change with the theme.
    fn paint_cells(&self, frame: &mut Frame, colors: TablatureColors, background: Color) {
        let overview = self.overview;
        for (measure, has_marker) in overview.markers.iter().enumerate() {
            let x = measure as f32 * STEP;
            if measure == 0 || (measure + 1) % 5 == 0 {
                frame.fill_text(Text {
                    shaping: Auto,
                    content: (measure + 1).to_string(),
                    color: colors.foreground,
                    size: 9.0.into(),
                    position: Point::new(x, 0.0),
                    ..Text::default()
                });
            }
            if *has_marker {
                frame.fill_rectangle(
                    Point::new(x, HEADER - 3.0),
                    Size::new(CELL, 2.0),
                    colors.accent,
                );
            }
        }
        for (track, rests) in overview.rests.iter().enumerate() {
            let y = HEADER + track as f32 * STEP;
            for (measure, rests) in rests.iter().enumerate() {
                let color = if *rests {
                    colors.string_line
                } else {
                    visible_color(overview.colors[track], background, colors.foreground)
                };
                frame.fill_rectangle(
                    Point::new(measure as f32 * STEP, y),
                    Size::new(CELL, CELL),
                    color,
                );
            }
        }
    }

    /// The selection, the focused measure and its marker.
    fn paint_focus(&self, frame: &mut Frame, theme: &Theme, colors: TablatureColors) {
        let overview = self.overview;
        let rows_height = overview.rests.len() as f32 * STEP;
        if let Some((start, end)) = self.selection {
            frame.fill_rectangle(
                Point::new(start as f32 * STEP, HEADER),
                Size::new((end + 1 - start) as f32 * STEP - 1.0, rows_height - 1.0),
                theme.palette().primary.scale_alpha(0.4),
            );
        }
        let x = self.focused as f32 * STEP;
        frame.stroke_rectangle(
            Point::new(x - 0.5, HEADER - 0.5),
            Size::new(STEP, rows_height),
            Stroke::default()
                .with_color(colors.foreground)
                .with_width(1.0),
        );
        let resting = overview
            .rests
            .get(self.track)
            .and_then(|rests| rests.get(self.focused))
            .copied()
            .unwrap_or(true);
        let square = if resting {
            colors.string_line
        } else {
            let background = theme.palette().background;
            visible_color(overview.colors[self.track], background, colors.foreground)
        };
        let marker = Path::rounded_rectangle(
            Point::new(x + 3.0, HEADER + self.track as f32 * STEP + 3.0),
            Size::new(CELL - 6.0, CELL - 6.0),
            2.0.into(),
        );
        frame.fill(&marker, marker_color(square));
    }
}

impl canvas::Program<Message> for OverviewCanvas<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Option<Action<Message>> {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let point = cursor.position_in(bounds)?;
            let (track, measure) = cell_at(
                point,
                self.overview.rests.len(),
                self.overview.measure_count(),
            )?;
            return Some(Action::publish(Message::TrackOverviewMeasure(
                track, measure,
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
        // the cached squares hold the colors they were painted with
        if self.overview.painted_colors.get() != Some(colors) {
            self.overview.painted_colors.set(Some(colors));
            self.overview.cells_cache.clear();
        }
        let cells = self
            .overview
            .cells_cache
            .draw(renderer, bounds.size(), |frame| {
                self.paint_cells(frame, colors, theme.palette().background);
            });
        let mut focus = Frame::new(renderer, bounds.size());
        self.paint_focus(&mut focus, theme, colors);
        vec![cells, focus.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> mouse::Interaction {
        let over_square = cursor.position_in(bounds).is_some_and(|point| {
            cell_at(
                point,
                self.overview.rests.len(),
                self.overview.measure_count(),
            )
            .is_some()
        });
        if over_square {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::song_parser::{Beat, Note, NoteEffect, Voice};

    #[test]
    fn a_point_finds_its_square() {
        let at = |x: f32, y: f32| cell_at(Point::new(x, y), 3, 10);
        assert_eq!(at(1.0, HEADER + 1.0), Some((0, 0)));
        assert_eq!(at(STEP * 4.5, HEADER + STEP * 2.5), Some((2, 4)));
        // the header, and past the last track or measure
        assert_eq!(at(1.0, HEADER - 1.0), None);
        assert_eq!(at(1.0, HEADER + STEP * 3.0), None);
        assert_eq!(at(STEP * 10.0, HEADER + 1.0), None);
    }

    #[test]
    fn a_visible_measure_keeps_the_scroll() {
        // ten measures wide from measure 5
        let (offset, width) = (5.0 * STEP, 10.0 * STEP);
        assert_eq!(follow_offset(5, offset, width, 0), None);
        assert_eq!(follow_offset(14, offset, width, 0), None);
    }

    #[test]
    fn a_measure_out_of_view_scrolls_it_in() {
        let (offset, width) = (5.0 * STEP, 20.0 * STEP);
        // before the view: it becomes the first one shown
        assert_eq!(follow_offset(2, offset, width, 3), Some(2.0 * STEP));
        // after it: shown with the margin after it
        assert_eq!(follow_offset(23, offset, width, 3), Some(20.0 * STEP));
        assert_eq!(follow_offset(25, offset, width, 0), Some(25.0 * STEP));
    }

    #[test]
    fn a_narrow_view_follows_with_a_smaller_margin() {
        // four measures wide: a margin of three would keep jumping, it
        // shrinks to one
        let width = 4.0 * STEP;
        assert_eq!(follow_offset(4, 0.0, width, 3), Some(3.0 * STEP));
    }

    fn overview() -> TrackOverview {
        let song = Song {
            measure_headers: (0..100).map(|_| Default::default()).collect(),
            ..Default::default()
        };
        TrackOverview::new(&song)
    }

    fn view(x: f32, y: f32) -> SquaresView {
        SquaresView {
            x,
            y,
            width: 10.0 * STEP,
            max_x: 90.0 * STEP,
        }
    }

    #[test]
    fn following_waits_for_the_view_and_moves_once_per_measure() {
        let mut overview = overview();
        assert_eq!(overview.follow(50, 0), None);
        overview.squares_at(view(0.0, 0.0));
        let (id, offset) = overview.follow(50, 0).unwrap();
        assert_eq!(id, overview.squares_id);
        assert_eq!(offset.x, 50.0 * STEP);
        // scrolled away by hand: the same measure does not pull it back
        overview.squares_at(view(0.0, 0.0));
        assert_eq!(overview.follow(50, 0), None);
    }

    #[test]
    fn following_stops_at_the_end_of_the_squares() {
        let mut overview = overview();
        overview.squares_at(view(0.0, 0.0));
        let (_, offset) = overview.follow(99, 0).unwrap();
        assert_eq!(offset.x, 90.0 * STEP);
        // the end is visible: nothing more to scroll
        assert_eq!(overview.follow(95, 0), None);
    }

    #[test]
    fn labels_and_squares_scroll_together_vertically() {
        let mut overview = overview();
        // sideways: the labels stay
        assert_eq!(overview.squares_at(view(30.0, 0.0)), None);
        // down: the labels follow
        let (id, offset) = overview.squares_at(view(30.0, 45.0)).unwrap();
        assert_eq!((id, offset.y), (overview.labels_id.clone(), 45.0));
        // their echo moves nothing back
        assert_eq!(overview.labels_at(45.0), None);
        // the labels scrolled: the squares follow, where they stand sideways
        let (id, offset) = overview.labels_at(15.0).unwrap();
        assert_eq!(id, overview.squares_id);
        assert_eq!((offset.x, offset.y), (30.0, 15.0));
        assert_eq!(overview.squares_at(view(30.0, 15.0)), None);
    }

    fn measure(beats: Vec<Beat>) -> Measure {
        Measure {
            voices: vec![Voice {
                measure_index: 0,
                beats,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_measure_without_notes_rests() {
        assert!(is_rest_measure(&measure(vec![Beat::default()])));
        assert!(is_rest_measure(&measure(vec![])));
        let note = Beat {
            notes: vec![Note::new(NoteEffect::default())],
            ..Default::default()
        };
        assert!(!is_rest_measure(&measure(vec![Beat::default(), note])));
    }

    #[test]
    fn a_track_without_color_takes_the_accent() {
        assert_eq!(track_color(0xFF_00_00), Color::from_rgb8(255, 0, 0));
        assert_eq!(track_color(0x00_80_FF), Color::from_rgb8(0, 128, 255));
        assert_eq!(track_color(0), COLOR_DARK_RED);
    }

    #[test]
    fn a_track_color_close_to_the_background_is_lifted() {
        let (white, black) = (Color::WHITE, Color::BLACK);
        let red = Color::from_rgb8(255, 0, 0);
        assert_eq!(visible_color(red, white, black), red);
        // as dark as the dark theme, but saturated
        let dark = Color::from_rgb8(43, 45, 49);
        assert_eq!(visible_color(red, dark, white), red);
        assert_ne!(
            visible_color(Color::from_rgb8(250, 250, 250), white, black),
            Color::from_rgb8(250, 250, 250)
        );
        assert_ne!(
            visible_color(Color::from_rgb8(10, 10, 10), black, white),
            Color::from_rgb8(10, 10, 10)
        );
    }

    #[test]
    fn the_marker_contrasts_with_its_square() {
        assert_eq!(marker_color(Color::from_rgb8(255, 255, 0)), Color::BLACK);
        assert_eq!(marker_color(Color::from_rgb8(40, 40, 120)), Color::WHITE);
    }
}
