//! Staff furniture: bar lines, repeats, endings, time signature and focus boxes.

use super::layout::{BEAT_LENGTH, HALF_BEAT_LENGTH, RowSpacing, STRING_LINE_HEIGHT};
use crate::parser::song_parser::{MeasureHeader, TimeSignature};
use crate::ui::utils::TablatureColors;
use iced::advanced::text::Shaping::Auto;
use iced::widget::canvas::{Frame, Path, Stroke, Text};
use iced::{Point, Rectangle, Renderer, Size};

/// The bar line a measure opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BarStart {
    /// A repeat sign: thick and thin lines, then dots.
    Repeat,
    /// The opening of the song: thick and thin lines.
    Section,
    /// A plain line, for the first measure of a staff line.
    Line,
    /// No line: the previous measure's closing line is shared.
    None,
}

impl BarStart {
    pub(super) const fn of(header: &MeasureHeader, measure_id: usize, first_on_line: bool) -> Self {
        if header.repeat_open {
            Self::Repeat
        } else if measure_id == 0 {
            Self::Section
        } else if first_on_line {
            Self::Line
        } else {
            Self::None
        }
    }
}

/// The bar line a measure closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BarEnd {
    /// A repeat sign, with how many times the section repeats.
    Repeat(i8),
    /// The end of the song: thin and thick lines.
    Section,
    /// A plain line.
    Line,
}

impl BarEnd {
    pub(super) const fn of(header: &MeasureHeader, is_last: bool) -> Self {
        if header.repeat_close > 0 {
            Self::Repeat(header.repeat_close)
        } else if is_last {
            Self::Section
        } else {
            Self::Line
        }
    }
}

pub(super) fn draw_bar_start(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    bar: BarStart,
    measure_start_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    match bar {
        BarStart::Repeat => draw_open_repeat(
            frame,
            colors,
            measure_start_x,
            measure_start_y,
            vertical_measure_height,
        ),
        BarStart::Section => draw_open_section(
            frame,
            colors,
            measure_start_x,
            measure_start_y,
            vertical_measure_height,
        ),
        BarStart::Line => draw_measure_vertical_line(
            frame,
            colors,
            vertical_measure_height,
            measure_start_x,
            measure_start_y,
        ),
        BarStart::None => {}
    }
}

pub(super) fn draw_bar_end(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    bar: BarEnd,
    measure_end_x: f32,
    measure_start_y: f32,
    vertical_measure_height: f32,
) {
    match bar {
        BarEnd::Repeat(count) => draw_close_repeat(
            frame,
            colors,
            measure_end_x,
            measure_start_y,
            vertical_measure_height,
            count,
        ),
        BarEnd::Section => draw_end_section(
            frame,
            colors,
            measure_end_x,
            measure_start_y,
            vertical_measure_height,
        ),
        BarEnd::Line => draw_measure_vertical_line(
            frame,
            colors,
            vertical_measure_height,
            measure_end_x,
            measure_start_y,
        ),
    }
}

/// Keep drawn lines off the canvas edge, where they would be clipped in half.
pub(super) const CANVAS_MARGIN: f32 = 2.0;

/// How far above the staff the focus box sits.
///
/// It rides the header boundary, so the box encloses everything drawn around
/// the staff - measure number, bend labels, staccato dots - without crossing
/// any of it. On a line with no annotation rows the header alone reaches the
/// canvas edge, so the box is held below it.
pub(super) fn focus_box_top(rows: RowSpacing, measure_start_y: f32) -> f32 {
    rows.staff_header
        .min(measure_start_y - CANVAS_MARGIN)
        .max(0.0)
}

pub(super) fn draw_selection_fill(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    total_measure_len: f32,
    vertical_measure_height: f32,
    measure_start_x: f32,
    measure_start_y: f32,
    rows: RowSpacing,
) {
    const BOX_SIDE: f32 = 8.0;
    let top = focus_box_top(rows, measure_start_y);
    let bottom = rows.staff_footer - CANVAS_MARGIN;
    let top_left = Point::new(measure_start_x + BOX_SIDE, measure_start_y - top);
    let rectangle_size = Size::new(
        total_measure_len - BOX_SIDE * 2.0,
        vertical_measure_height + top + bottom,
    );
    frame.fill_rectangle(top_left, rectangle_size, colors.selection);
}

pub(super) fn draw_focused_box(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    total_measure_len: f32,
    vertical_measure_height: f32,
    measure_start_x: f32,
    measure_start_y: f32,
    rows: RowSpacing,
) {
    const BOX_SIDE: f32 = 8.0;

    let top = focus_box_top(rows, measure_start_y);
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
    let stroke = Stroke::default()
        .with_width(1.0)
        .with_color(colors.foreground);
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
    let stroke = Stroke::default()
        .with_width(1.5)
        .with_color(colors.foreground);
    frame.stroke(&vertical_line, stroke);
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
    let stroke = Stroke::default()
        .with_width(4.0)
        .with_color(colors.foreground);
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

pub(super) fn draw_alternative_ending(
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

    let stroke = Stroke::default()
        .with_width(1.0)
        .with_color(colors.foreground);

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
        Stroke::default()
            .with_width(2.0)
            .with_color(colors.foreground),
    );

    // bottom dot
    let bottom_position_y = start_y + (vertical_measure_height / 3.0) * 2.0;
    let center = Point::new(start_x, bottom_position_y);
    let circle = Path::circle(center, 1.0);

    frame.stroke(
        &circle,
        Stroke::default()
            .with_width(2.0)
            .with_color(colors.foreground),
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
    let stroke = Stroke::default()
        .with_width(4.0)
        .with_color(colors.foreground);
    frame.stroke(&thick_vertical_line, stroke);
}

pub(super) fn draw_time_signature(
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
