//! Note and beat effects drawn on the staff: slides, arcs, bends, strokes and tremolo.

use super::beat::BeatSlot;
use super::layout::{BEND_ARROW_WIDTH, STRING_LINE_HEIGHT, Staff};
use crate::parser::song_parser::{Beat, BeatStrokeDirection, TremoloPickingEffect};
use crate::ui::utils::TablatureColors;
use iced::advanced::text::Shaping::Auto;
use iced::widget::canvas::{Frame, LineDash, Path, Stroke, Text};
use iced::{Color, Point, Renderer};

/// The sloped line of a slide, rising or falling towards the note it lands
/// on. Without a target it is a short level stub, like TuxGuitar's.
pub(super) fn draw_slide(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    note_position_x: f32,
    string_y: f32,
    next_note: Option<(f32, i16)>,
    value: i16,
) {
    const Y_MOVE: f32 = STRING_LINE_HEIGHT / 3.5;
    let stroke = Stroke::default()
        .with_width(0.9)
        .with_color(colors.foreground);
    let start_x = note_position_x + 5.0;
    let Some((next_x, next_value)) = next_note else {
        // nothing to reach: a level stub leaving the note
        frame.stroke(
            &Path::line(
                Point::new(start_x, string_y - Y_MOVE),
                Point::new(start_x + 13.0, string_y - Y_MOVE),
            ),
            stroke,
        );
        return;
    };
    let end_x = (next_x - 5.0).max(start_x + 2.0);
    // slope towards the landing fret; equal frets slide level above the string
    let (start_y, end_y) = match next_value.cmp(&value) {
        std::cmp::Ordering::Less => (string_y - Y_MOVE, string_y + Y_MOVE),
        std::cmp::Ordering::Greater => (string_y + Y_MOVE, string_y - Y_MOVE),
        std::cmp::Ordering::Equal => (string_y - Y_MOVE, string_y - Y_MOVE),
    };
    frame.stroke(
        &Path::line(Point::new(start_x, start_y), Point::new(end_x, end_y)),
        stroke,
    );
}

/// The slur of a hammer-on or pull-off, arcing over to the next note.
pub(super) fn draw_hammer_arc(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    note_position_x: f32,
    note_position_y: f32,
    next_note_x: Option<f32>,
) {
    let x = note_position_x + 5.0;
    let width = next_note_x.map_or(10.0, |next| (next - 5.0 - x).max(8.0));
    let y = note_position_y;
    let height = STRING_LINE_HEIGHT / 2.0;
    let arc = Path::new(|p| {
        p.move_to(Point::new(x, y));
        p.bezier_curve_to(
            Point::new(x, y - height),
            Point::new(x + width, y - height),
            Point::new(x + width, y),
        );
    });
    frame.stroke(
        &arc,
        Stroke::default()
            .with_width(0.9)
            .with_color(colors.foreground),
    );
}

/// The arc joining a tied note back to the one it continues.
pub(super) fn draw_tie_arc(
    frame: &mut Frame<Renderer>,
    color: Color,
    from_x: f32,
    to_x: f32,
    string_y: f32,
) {
    let y = string_y + STRING_LINE_HEIGHT / 3.0;
    let height = STRING_LINE_HEIGHT / 3.0;
    let from_x = from_x.min(to_x - 4.0);
    let arc = Path::new(|p| {
        p.move_to(Point::new(from_x, y));
        p.bezier_curve_to(
            Point::new(from_x, y + height),
            Point::new(to_x, y + height),
            Point::new(to_x, y),
        );
    });
    frame.stroke(&arc, Stroke::default().with_width(0.9).with_color(color));
}

// Amplitude labels in tones, indexed by bend value (half-semitone units)
const BEND_AMPLITUDES: [&str; 13] = [
    "", "¼", "½", "¾", "1", "1¼", "1½", "1¾", "2", "2¼", "2½", "2¾", "3",
];

/// Draw a bend like TuxGuitar's `paintBend`: one curved arrow per movement
/// rising to (or releasing from) a band above the staff, with the reached
/// amplitude in tones next to the arrow tip. A bend without movements is a
/// held bend, drawn as a dashed line at band height across the beat.
/// The bend arrows of a note, from its fret number up above the staff,
/// and the amplitude each one reaches when `show_amplitude`.
pub(super) fn draw_bend(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    staff: Staff,
    slot: &BeatSlot,
    note_position: Point,
    movements: &[i32],
    show_amplitude: bool,
) {
    let Point {
        x: note_position_x,
        y: note_position_y,
    } = note_position;
    let measure_start_y = staff.top;
    let stroke = Stroke::default()
        .with_width(0.8)
        .with_color(colors.foreground);
    let arrow_size = 2.5;
    // shrink the arrows with the beat when the measure is compressed, so
    // they stay within the width reserved by beat_natural_width
    let arrow_width = BEND_ARROW_WIDTH * slot.width_scale.min(1.0);
    // vertical extent: from the note's fret number up to a band above the staff
    let y_low = note_position_y + 4.0;
    let y_high = measure_start_y - 8.0;
    let amplitude_y = measure_start_y - 18.0;

    if movements.is_empty() {
        // held bend: dashed line at band height until the end of the beat
        let x_start = note_position_x - 4.0;
        let x_end = (slot.x() + slot.span - 6.0).max(x_start + 5.0);
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

/// Picking direction above the staff, like TuxGuitar's `paintPickStroke`:
/// an up stroke is a `∨`, a down stroke the bracket-shaped `∏`.
pub(super) fn draw_pick_stroke(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    direction: &BeatStrokeDirection,
    beat_position_x: f32,
    y: f32,
) {
    let stroke = Stroke::default()
        .with_width(0.8)
        .with_color(colors.foreground);
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
                Stroke::default()
                    .with_width(2.0)
                    .with_color(colors.foreground),
            );
        }
        BeatStrokeDirection::None => {}
    }
}

/// Staccato dot above the top note of the beat (TuxGuitar only draws it in
/// score mode; the dot above the fret number is the tab equivalent).
pub(super) fn draw_staccato_dot(
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
pub(super) fn draw_tremolo_picking(
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
    let stroke = Stroke::default()
        .with_width(1.2)
        .with_color(colors.foreground);
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

pub(super) fn draw_stroke_arrow(
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

    let stroke = Stroke::default()
        .with_width(0.8)
        .with_color(colors.foreground);

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
