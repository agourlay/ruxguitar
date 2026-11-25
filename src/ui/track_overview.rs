//! The multitrack overview under the tablature: a row per track, a square
//! per measure, to see where each instrument plays and jump there. Like
//! TuxGuitar's track table, each row also solos and mutes its track.
//!
//! Squares follow the tablature's colors: a neutral tone where a track plays,
//! the accent on the current track, a faint one where the measure only rests.
//! A track that will not be heard, muted or left out of a solo, fades. The
//! measure being played gets an inner marker, and a click moves to that
//! track and measure.
//!
//! The band of measure numbers stays on top while the rows scroll under it,
//! and names a measure's marker when hovered.

use crate::parser::song_parser::{Measure, Song};
use crate::ui::application::Message;
use crate::ui::selections::TrackSelection;
use crate::ui::utils::{TablatureColors, mix, toggle_style};
use iced::advanced::mouse;
use iced::advanced::text::Shaping::Auto;
use iced::mouse::Cursor;
use iced::widget::canvas::{Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::text::Wrapping;
use iced::widget::{
    Action, Canvas, Id, button, canvas, column, container, row, scrollable, space, text, tooltip,
};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use std::cell::{Cell, RefCell};

/// The side of a measure's square.
const CELL: f32 = 14.0;
/// From one square to the next, gap included.
const STEP: f32 = CELL + 1.0;
/// The band above the squares holding measure numbers and markers.
const HEADER: f32 = 14.0;
/// The track names with their solo and mute toggles.
const LABEL_WIDTH: f32 = 210.0;
const LABEL_SIZE: f32 = 11.0;
const TOGGLE_WIDTH: f32 = 18.0;
/// Rows always shown before the overview scrolls vertically.
const MIN_VISIBLE_ROWS: usize = 6;
/// The share of the window's height the overview may grow to.
const WINDOW_SHARE: f32 = 0.3;
const TOOLTIP_SIZE: f32 = 11.0;
/// The horizontal scrollbar, drawn over the bottom of the squares.
const SCROLLBAR_WIDTH: f32 = 10.0;
/// Measures kept visible past the played one while following playback.
pub const PLAYING_MARGIN: usize = 3;

pub struct TrackOverview {
    /// Per track, per measure, whether the measure only rests.
    rests: Vec<Vec<bool>>,
    labels: Vec<String>,
    /// Per track, its full name and tuning, told when its label is hovered;
    /// empty when it has neither.
    details: Vec<String>,
    /// Per measure, the title of its marker, which can be empty.
    markers: Vec<Option<String>>,
    /// Rows shown before the overview scrolls vertically.
    max_rows: usize,
    cells_cache: Cache,
    header_cache: Cache,
    /// The colors and offset the cached header was painted with.
    painted_header: Cell<Option<(TablatureColors, f32)>>,
    /// What the cached squares were painted with.
    painted: RefCell<Option<Painted>>,
    squares_id: Id,
    labels_id: Id,
    /// Where the squares are scrolled and how wide they show, once known.
    squares: Option<SquaresView>,
    /// Where the labels are scrolled, which follows the squares.
    labels_y: f32,
    /// The measure the scroll last followed.
    followed: Option<usize>,
    /// The track whose row to bring into view once the view is known.
    track_to_follow: Option<usize>,
}

impl TrackOverview {
    /// The overview of `song`, as high as a window of `window_height` allows.
    pub fn new(song: &Song, window_height: f32) -> Self {
        Self {
            rests: song
                .tracks
                .iter()
                .map(|track| track.measures.iter().map(is_rest_measure).collect())
                .collect(),
            labels: song
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| format!("{} {}", index + 1, track.name))
                .collect(),
            details: TrackSelection::all_of(song)
                .iter()
                .map(TrackSelection::detail)
                .collect(),
            markers: song
                .measure_headers
                .iter()
                .map(|header| {
                    header
                        .marker
                        .as_ref()
                        .map(|marker| marker.title.trim().to_string())
                })
                .collect(),
            max_rows: rows_for_window(window_height),
            cells_cache: Cache::default(),
            header_cache: Cache::default(),
            painted_header: Cell::new(None),
            painted: RefCell::new(None),
            squares_id: Id::unique(),
            labels_id: Id::unique(),
            squares: None,
            labels_y: 0.0,
            followed: None,
            track_to_follow: None,
        }
    }

    fn measure_count(&self) -> usize {
        self.markers.len()
    }

    /// The title to tell of a measure's marker: none without a marker, or
    /// for one left untitled, which only shows its tick.
    fn marker_title(&self, measure: usize) -> Option<&str> {
        let title = self.markers.get(measure)?.as_deref()?;
        (!title.is_empty()).then_some(title)
    }

    /// Rows shown at once: all of them when the window leaves the room.
    fn visible_rows(&self) -> usize {
        self.labels.len().min(self.max_rows)
    }

    /// The window was resized: the overview grows or shrinks with it.
    pub fn window_resized(&mut self, window_height: f32) {
        self.max_rows = rows_for_window(window_height);
    }

    /// How far down the rows are scrolled, under the header.
    fn scroll_y(&self) -> f32 {
        // no further than the rows go: squares that stop scrolling, as when
        // the window grows to show them all, do not report it
        let furthest = (self.labels.len() - self.visible_rows()) as f32 * STEP;
        self.squares.map_or(0.0, |squares| squares.y.min(furthest))
    }

    /// Forget where the squares stand, for when they show again and
    /// start from the left and top, with `track` to bring back into view.
    pub fn reset_view(&mut self, track: usize) {
        self.squares = None;
        self.labels_y = 0.0;
        self.followed = None;
        self.track_to_follow = Some(track);
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
        if let Some(track) = self.track_to_follow.take()
            && let Some(follow) = self.follow_track(track)
        {
            return Some(follow);
        }
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

    /// The squares' offset bringing `track`'s row into view, when hidden;
    /// the labels follow the squares. Before the view is known, the track
    /// waits for it.
    pub fn follow_track(&mut self, track: usize) -> Option<(Id, AbsoluteOffset)> {
        let Some(squares) = self.squares.as_mut() else {
            self.track_to_follow = Some(track);
            return None;
        };
        let visible = self.labels.len().min(self.max_rows);
        // the squares hold the offset a measure follow scrolls with too
        let y = follow_row(track, squares.y, visible)?;
        squares.y = y;
        Some((self.squares_id.clone(), AbsoluteOffset { x: squares.x, y }))
    }

    pub fn view(
        &self,
        track: usize,
        focused: usize,
        selection: Option<(usize, usize)>,
        sounds: Vec<TrackSound>,
    ) -> Element<'_, Message> {
        let labels = self
            .labels
            .iter()
            .zip(&self.details)
            .zip(&sounds)
            .enumerate()
            .map(|(index, ((label, detail), sound))| {
                let is_current = index == track;
                let name = button(
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
                        // the accent, like the current track's squares
                        let accent = TablatureColors::of(theme).accent;
                        let background = match status {
                            button::Status::Hovered => mix(accent, Color::WHITE, 0.15),
                            _ => accent,
                        };
                        button::primary(theme, status).with_background(background)
                    } else {
                        button::text(theme, status)
                    }
                });
                // the label clips a long name, and has no room for the tuning
                let name: Element<'_, Message> = if detail.is_empty() {
                    name.into()
                } else {
                    tooltip(
                        name,
                        text(detail).size(LABEL_SIZE).shaping(Auto),
                        tooltip::Position::FollowCursor,
                    )
                    .style(container::rounded_box)
                    .into()
                };
                row![
                    name,
                    toggle("S", "Solo", sound.solo, Message::ToggleTrackSolo(index)),
                    toggle("M", "Mute", sound.muted, Message::ToggleTrackMute(index)),
                ]
                .into()
            });
        let rows = self.labels.len();
        let visible_height = self.visible_rows() as f32 * STEP;
        let scrollbar = Scrollbar::default()
            .width(SCROLLBAR_WIDTH)
            .scroller_width(SCROLLBAR_WIDTH)
            .spacing(0.0);
        // the vertical scrollbar sits by the labels: beside the squares it
        // would leave a corner with the horizontal one, showing a hidden row
        let labels = scrollable(column(labels))
            .id(self.labels_id.clone())
            .direction(Direction::Vertical(scrollbar))
            .on_scroll(Message::TrackOverviewLabelsScrolled)
            .height(visible_height);
        // the labels scroll under the header's height, like the squares
        // under their header
        let labels = column![space().height(HEADER), labels].width(LABEL_WIDTH);

        let squares = Canvas::new(OverviewCanvas {
            overview: self,
            track,
            focused,
            selection,
            audible: sounds.iter().map(|sound| sound.audible).collect(),
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
            .height(HEADER + visible_height + SCROLLBAR_WIDTH)
            .into()
    }
}

/// A track's solo and mute, and whether it will be heard.
#[derive(Debug, Clone, Copy)]
pub struct TrackSound {
    pub solo: bool,
    pub muted: bool,
    pub audible: bool,
}

impl Default for TrackSound {
    fn default() -> Self {
        Self {
            solo: false,
            muted: false,
            audible: true,
        }
    }
}

/// A row's solo or mute toggle: a dim letter while off, so the names stand
/// out, inverted while on like the toolbar's.
fn toggle<'a>(letter: &'a str, label: &'a str, on: bool, message: Message) -> Element<'a, Message> {
    let toggle = button(text(letter).size(LABEL_SIZE).center())
        .padding(0)
        .width(TOGGLE_WIDTH)
        .height(STEP)
        .on_press(message)
        .style(move |theme: &Theme, status| {
            if on {
                toggle_style(true)(theme, status)
            } else {
                let style = button::text(theme, status);
                if status == button::Status::Hovered {
                    style
                } else {
                    // off: a dim letter, so the names stand out
                    button::Style {
                        text_color: TablatureColors::of(theme).string_line,
                        ..style
                    }
                }
            }
        });
    tooltip(toggle, label, tooltip::Position::FollowCursor)
        .style(container::rounded_box)
        .into()
}

/// What the cached squares were painted with.
#[derive(Debug, Clone, PartialEq)]
struct Painted {
    colors: TablatureColors,
    track: usize,
    audible: Vec<bool>,
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

/// The colors of the squares, from the tablature's.
#[derive(Debug, Clone, Copy)]
struct SquareColors {
    plays: Color,
    current: Color,
    /// As `plays` and `current`, for a track that will not be heard.
    silent_plays: Color,
    silent_current: Color,
    rests: Color,
    /// The marker cut into a square with notes.
    marker: Color,
    /// The marker on a resting square, which the background would not show on.
    resting_marker: Color,
}

impl SquareColors {
    fn of(colors: TablatureColors, background: Color) -> Self {
        Self {
            plays: mix(background, colors.foreground, 0.45),
            current: colors.accent,
            silent_plays: mix(background, colors.foreground, 0.25),
            silent_current: mix(background, colors.accent, 0.45),
            rests: mix(background, colors.foreground, 0.12),
            marker: background,
            resting_marker: colors.foreground,
        }
    }

    fn square(&self, rests: bool, current: bool, audible: bool) -> Color {
        match (rests, current, audible) {
            (true, _, _) => self.rests,
            (false, true, true) => self.current,
            (false, false, true) => self.plays,
            (false, true, false) => self.silent_current,
            (false, false, false) => self.silent_plays,
        }
    }

    fn marker(&self, rests: bool) -> Color {
        if rests {
            self.resting_marker
        } else {
            self.marker
        }
    }
}

/// Rows shown before the overview scrolls, in a window `window_height` high.
fn rows_for_window(window_height: f32) -> usize {
    let room = window_height * WINDOW_SHARE - HEADER - SCROLLBAR_WIDTH;
    ((room / STEP) as usize).max(MIN_VISIBLE_ROWS)
}

/// The `(track, measure)` of the square under a point of the canvas, whose
/// header stands `scroll_y` down, over the rows scrolled under it.
fn cell_at(point: Point, tracks: usize, measures: usize, scroll_y: f32) -> Option<(usize, usize)> {
    if point.x < 0.0 || point.y < scroll_y + HEADER {
        return None;
    }
    let measure = (point.x / STEP) as usize;
    let track = ((point.y - HEADER) / STEP) as usize;
    (track < tracks && measure < measures).then_some((track, measure))
}

/// The measure whose part of the header, standing `scroll_y` down the
/// canvas, is under a point.
fn header_measure_at(point: Point, scroll_y: f32) -> Option<usize> {
    let in_header = point.x >= 0.0 && (scroll_y..scroll_y + HEADER).contains(&point.y);
    in_header.then(|| (point.x / STEP) as usize)
}

/// The vertical offset bringing `track`'s row into a view at `offset`
/// showing `visible` rows, or `None` when it shows already.
fn follow_row(track: usize, offset: f32, visible: usize) -> Option<f32> {
    // the header stays on top: the view shows `visible` rows from `offset`
    let top = track as f32 * STEP;
    if top < offset {
        Some(top)
    } else if top + STEP > offset + visible as f32 * STEP {
        Some((track + 1).saturating_sub(visible) as f32 * STEP)
    } else {
        None
    }
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
    /// Per track, whether it will be heard.
    audible: Vec<bool>,
}

impl OverviewCanvas<'_> {
    /// The squares, which only change with the theme, the current track
    /// and which tracks are heard.
    fn paint_cells(&self, frame: &mut Frame, squares: SquareColors) {
        let overview = self.overview;
        for (track, rests) in overview.rests.iter().enumerate() {
            let y = HEADER + track as f32 * STEP;
            for (measure, rests) in rests.iter().enumerate() {
                let audible = self.audible.get(track).copied().unwrap_or(true);
                let color = squares.square(*rests, track == self.track, audible);
                frame.fill_rectangle(
                    Point::new(measure as f32 * STEP, y),
                    Size::new(CELL, CELL),
                    color,
                );
            }
        }
    }

    /// The measure numbers and marker ticks, `y` down the canvas to stay on
    /// top of the rows scrolled under them.
    fn paint_header(&self, frame: &mut Frame, colors: TablatureColors, background: Color, y: f32) {
        frame.fill_rectangle(
            Point::new(0.0, y),
            Size::new(frame.width(), HEADER),
            background,
        );
        for (measure, marker) in self.overview.markers.iter().enumerate() {
            let x = measure as f32 * STEP;
            if measure == 0 || (measure + 1) % 5 == 0 {
                frame.fill_text(Text {
                    shaping: Auto,
                    content: (measure + 1).to_string(),
                    color: colors.foreground,
                    size: 9.0.into(),
                    position: Point::new(x, y),
                    ..Text::default()
                });
            }
            if marker.is_some() {
                frame.fill_rectangle(
                    Point::new(x, y + HEADER - 3.0),
                    Size::new(CELL, 2.0),
                    colors.accent,
                );
            }
        }
    }

    /// The titled marker under the cursor, when it is over the header.
    fn hovered_marker(&self, bounds: Rectangle, cursor: Cursor) -> Option<(usize, Point)> {
        let point = cursor.position_in(bounds)?;
        let measure = header_measure_at(point, self.overview.scroll_y())?;
        self.overview.marker_title(measure)?;
        Some((measure, point))
    }

    /// The title of the hovered marker, in a box by the cursor.
    fn paint_tooltip(&self, frame: &mut Frame, theme: &Theme, title: &str, cursor: Point) {
        let palette = theme.palette();
        // the canvas only shows a part of itself: stay within it
        let (left, right) = self
            .overview
            .squares
            .map_or((0.0, frame.width()), |view| (view.x, view.x + view.width));
        // monospace: a character is about 0.6 of the size wide
        let width = title.chars().count() as f32 * TOOLTIP_SIZE * 0.6 + 12.0;
        let x = (cursor.x + 10.0).min(right - width).max(left);
        let y = self.overview.scroll_y() + HEADER;
        let bubble =
            Path::rounded_rectangle(Point::new(x, y), Size::new(width, STEP + 3.0), 3.0.into());
        frame.fill(&bubble, palette.text);
        frame.fill_text(Text {
            shaping: Auto,
            content: title.to_string(),
            color: palette.background,
            size: TOOLTIP_SIZE.into(),
            position: Point::new(x + 6.0, y + 2.0),
            ..Text::default()
        });
    }

    /// The selection, the focused measure and its marker.
    fn paint_focus(
        &self,
        frame: &mut Frame,
        theme: &Theme,
        colors: TablatureColors,
        squares: SquareColors,
    ) {
        let overview = self.overview;
        let rows_height = overview.rests.len() as f32 * STEP;
        if let Some((start, end)) = self.selection {
            frame.fill_rectangle(
                Point::new(start as f32 * STEP, HEADER),
                Size::new((end + 1 - start) as f32 * STEP - 1.0, rows_height - 1.0),
                // a faint tint, like the tablature's: the squares show through
                theme.palette().primary.scale_alpha(0.2),
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
        let marker = Path::rounded_rectangle(
            Point::new(x + 3.0, HEADER + self.track as f32 * STEP + 3.0),
            Size::new(CELL - 6.0, CELL - 6.0),
            2.0.into(),
        );
        let resting = overview
            .rests
            .get(self.track)
            .and_then(|rests| rests.get(self.focused))
            .copied()
            .unwrap_or(true);
        frame.fill(&marker, squares.marker(resting));
    }
}

impl canvas::Program<Message> for OverviewCanvas<'_> {
    /// The measure whose marker is hovered, to repaint when it changes.
    type State = Option<usize>;

    fn update(
        &self,
        hovered: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Option<Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let point = cursor.position_in(bounds)?;
                let (track, measure) = cell_at(
                    point,
                    self.overview.rests.len(),
                    self.overview.measure_count(),
                    self.overview.scroll_y(),
                )?;
                Some(Action::publish(Message::TrackOverviewMeasure(
                    track, measure,
                )))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let marker = self
                    .hovered_marker(bounds, cursor)
                    .map(|(measure, _)| measure);
                // the tooltip follows the cursor along its marker
                (marker.is_some() || *hovered != marker).then(|| {
                    *hovered = marker;
                    Action::request_redraw()
                })
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _hovered: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Vec<Geometry> {
        let colors = TablatureColors::of(theme);
        let squares = SquareColors::of(colors, theme.palette().background);
        let painted = Painted {
            colors,
            track: self.track,
            audible: self.audible.clone(),
        };
        let mut cached = self.overview.painted.borrow_mut();
        if cached.as_ref() != Some(&painted) {
            *cached = Some(painted);
            self.overview.cells_cache.clear();
        }
        drop(cached);
        let cells = self
            .overview
            .cells_cache
            .draw(renderer, bounds.size(), |frame| {
                self.paint_cells(frame, squares);
            });
        let mut focus = Frame::new(renderer, bounds.size());
        self.paint_focus(&mut focus, theme, colors, squares);

        let background = theme.palette().background;
        let scroll_y = self.overview.scroll_y();
        let painted_header = Some((colors, scroll_y));
        if self.overview.painted_header.get() != painted_header {
            self.overview.painted_header.set(painted_header);
            self.overview.header_cache.clear();
        }
        let header = self
            .overview
            .header_cache
            .draw(renderer, bounds.size(), |frame| {
                self.paint_header(frame, colors, background, scroll_y);
            });
        let mut layers = vec![cells, focus.into_geometry(), header];

        if let Some((measure, point)) = self.hovered_marker(bounds, cursor)
            && let Some(title) = self.overview.marker_title(measure)
        {
            let mut tooltip = Frame::new(renderer, bounds.size());
            self.paint_tooltip(&mut tooltip, theme, title, point);
            layers.push(tooltip.into_geometry());
        }
        layers
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
                self.overview.scroll_y(),
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
        let at = |x: f32, y: f32| cell_at(Point::new(x, y), 3, 10, 0.0);
        assert_eq!(at(1.0, HEADER + 1.0), Some((0, 0)));
        assert_eq!(at(STEP * 4.5, HEADER + STEP * 2.5), Some((2, 4)));
        // the header, and past the last track or measure
        assert_eq!(at(1.0, HEADER - 1.0), None);
        assert_eq!(at(1.0, HEADER + STEP * 3.0), None);
        assert_eq!(at(STEP * 10.0, HEADER + 1.0), None);
    }

    #[test]
    fn the_header_stays_over_the_rows_scrolled_under_it() {
        // scrolled two rows down: the header covers where row 2 starts
        let scroll_y = 2.0 * STEP;
        let at = |y: f32| cell_at(Point::new(1.0, y), 10, 10, scroll_y);
        assert_eq!(at(scroll_y + 1.0), None);
        assert_eq!(at(scroll_y + HEADER + 1.0), Some((2, 0)));
        let header = |y: f32| header_measure_at(Point::new(STEP * 4.5, y), scroll_y);
        assert_eq!(header(scroll_y + 1.0), Some(4));
        assert_eq!(header(scroll_y + HEADER + 1.0), None);
        assert_eq!(header(1.0), None);
    }

    #[test]
    fn only_a_titled_marker_has_a_title_to_tell() {
        use crate::parser::song_parser::{Marker, MeasureHeader};
        let header = |title: Option<&str>| MeasureHeader {
            marker: title.map(|title| Marker {
                title: title.to_string(),
                color: 0,
            }),
            ..Default::default()
        };
        let song = Song {
            measure_headers: vec![header(None), header(Some(" Intro ")), header(Some("  "))],
            ..Default::default()
        };
        let overview = TrackOverview::new(&song, 400.0);
        assert_eq!(overview.marker_title(0), None);
        assert_eq!(overview.marker_title(1), Some("Intro"));
        // untitled: it keeps its tick, without a tooltip
        assert_eq!(overview.markers[2], Some(String::new()));
        assert_eq!(overview.marker_title(2), None);
        assert_eq!(overview.marker_title(9), None);
    }

    #[test]
    fn a_taller_window_shows_more_rows() {
        assert_eq!(rows_for_window(400.0), MIN_VISIBLE_ROWS);
        assert_eq!(rows_for_window(768.0), 13);
        assert!(rows_for_window(1440.0) > rows_for_window(768.0));
        let song = Song {
            tracks: (0..9).map(|_| Default::default()).collect(),
            ..Default::default()
        };
        let mut overview = TrackOverview::new(&song, 400.0);
        assert_eq!(overview.visible_rows(), 6);
        // scrolled down in the small window, which then grows to show all
        // the rows: the header is back on top without a report
        overview.squares_at(view(0.0, 3.0 * STEP));
        assert_eq!(overview.scroll_y(), 3.0 * STEP);
        overview.window_resized(768.0);
        assert_eq!(overview.visible_rows(), 9);
        assert_eq!(overview.scroll_y(), 0.0);
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
        TrackOverview::new(&song, 400.0)
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
    fn the_current_track_plays_in_the_accent() {
        let colors = TablatureColors::of(&Theme::Dark);
        let squares = SquareColors::of(colors, Theme::Dark.palette().background);
        assert_eq!(squares.square(false, true, true), colors.accent);
        assert_ne!(squares.square(false, false, true), colors.accent);
        // a rest looks the same on every track
        assert_eq!(
            squares.square(true, true, true),
            squares.square(true, false, false)
        );
    }

    #[test]
    fn a_track_not_heard_fades() {
        let theme = Theme::Dark;
        let background = theme.palette().background;
        let squares = SquareColors::of(TablatureColors::of(&theme), background);
        for current in [true, false] {
            let heard = squares.square(false, current, true);
            let silent = squares.square(false, current, false);
            assert_ne!(heard, silent);
            // closer to the background, still apart from a rest
            assert!(distance(silent, background) < distance(heard, background));
            assert_ne!(silent, squares.square(true, current, false));
        }
    }

    fn distance(a: Color, b: Color) -> f32 {
        (a.r - b.r).abs() + (a.g - b.g).abs() + (a.b - b.b).abs()
    }

    #[test]
    fn a_hidden_row_scrolls_into_view() {
        // rows 2 to 7 show
        let offset = 2.0 * STEP;
        assert_eq!(follow_row(2, offset, 6), None);
        assert_eq!(follow_row(7, offset, 6), None);
        // above: it becomes the first row, below: the last
        assert_eq!(follow_row(0, offset, 6), Some(0.0));
        assert_eq!(follow_row(9, offset, 6), Some(4.0 * STEP));
    }

    #[test]
    fn following_a_track_moves_the_squares_and_the_labels_after_them() {
        let song = Song {
            tracks: (0..10).map(|_| Default::default()).collect(),
            ..Default::default()
        };
        let mut overview = TrackOverview::new(&song, 400.0);
        // the view is not known yet: the track waits for it
        assert_eq!(overview.follow_track(8), None);
        let (id, offset) = overview.squares_at(view(30.0, 0.0)).unwrap();
        assert_eq!(id, overview.squares_id);
        assert_eq!((offset.x, offset.y), (30.0, 3.0 * STEP));
        // a measure followed meanwhile keeps the row
        assert_eq!(overview.follow(50, 0).unwrap().1.y, 3.0 * STEP);
        // the squares report it: the labels follow, and their echo is quiet
        let (id, offset) = overview.squares_at(view(50.0 * STEP, 3.0 * STEP)).unwrap();
        assert_eq!((id, offset.y), (overview.labels_id.clone(), 3.0 * STEP));
        assert_eq!(overview.labels_at(3.0 * STEP), None);
        // shows already
        assert_eq!(overview.follow_track(5), None);
        assert_eq!(overview.follow_track(0).unwrap().1.y, 0.0);
    }

    #[test]
    fn the_marker_shows_on_a_resting_square() {
        for theme in [Theme::Light, Theme::Dark] {
            let background = theme.palette().background;
            let squares = SquareColors::of(TablatureColors::of(&theme), background);
            assert_eq!(squares.marker(false), background);
            assert_ne!(squares.marker(true), background);
        }
    }
}
