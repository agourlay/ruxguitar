//! Chord diagrams drawn above the staff.

use crate::parser::song_parser::Chord;
use crate::ui::utils::TablatureColors;
use iced::advanced::text::Shaping::Auto;
use iced::widget::canvas::{Frame, Path, Stroke, Text};
use iced::{Point, Renderer};

// A chord diagram: markers above the nut, then the fret grid, then the name.
pub(super) const CHORD_STRING_SPACING: f32 = 4.0;
const CHORD_FRET_SPACING: f32 = 5.0;
const CHORD_FRETS: usize = 5;
const CHORD_MARKER_HEIGHT: f32 = 5.0;
// Room on the left of the grid for the first-fret number.
pub(super) const CHORD_FIRST_FRET_SPACE: f32 = 6.0;
pub(super) const CHORD_DIAGRAM_HEIGHT: f32 =
    CHORD_MARKER_HEIGHT + CHORD_FRET_SPACING * CHORD_FRETS as f32;

/// Whether a chord carries a fingering worth drawing as a grid.
pub(super) fn has_diagram(chord: &Chord) -> bool {
    chord.strings.iter().any(|&fret| fret >= 0)
}

/// Width a chord diagram occupies.
pub(super) fn chord_diagram_width(chord: &Chord) -> f32 {
    (chord.strings.len().max(1) - 1) as f32 * CHORD_STRING_SPACING
}

/// A chord fingering, like TuxGuitar's `paintDiagram`: a grid of strings and
/// frets, dots for the fingered frets, and above the nut a circle for an open
/// string or a cross for a muted one.
pub(super) fn draw_chord_diagram(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    chord: &Chord,
    x: f32,
    y: f32,
) {
    let string_count = chord.strings.len();
    if string_count == 0 {
        return;
    }
    let stroke = Stroke::default()
        .with_width(0.6)
        .with_color(colors.string_line);
    let width = chord_diagram_width(chord);
    let grid_top = y + CHORD_MARKER_HEIGHT;
    let grid_bottom = grid_top + CHORD_FRET_SPACING * CHORD_FRETS as f32;

    // strings run down the grid, frets across it
    for i in 0..string_count {
        let string_x = x + i as f32 * CHORD_STRING_SPACING;
        frame.stroke(
            &Path::line(
                Point::new(string_x, grid_top),
                Point::new(string_x, grid_bottom),
            ),
            stroke,
        );
    }
    for fret in 0..=CHORD_FRETS {
        let fret_y = grid_top + fret as f32 * CHORD_FRET_SPACING;
        frame.stroke(
            &Path::line(Point::new(x, fret_y), Point::new(x + width, fret_y)),
            stroke,
        );
    }

    // frets are absolute, so the grid starts at the chord's first fret
    let first_fret = chord.first_fret.unwrap_or(1).max(1);
    if first_fret > 1 {
        let first_fret_text = Text {
            shaping: Auto,
            content: first_fret.to_string(),
            color: colors.foreground,
            size: 6.0.into(),
            position: Point::new(x - CHORD_FIRST_FRET_SPACE + 1.0, grid_top - 1.0),
            ..Text::default()
        };
        frame.fill_text(first_fret_text);
    }

    // string 1 is the rightmost line of the grid
    for (index, &fret) in chord.strings.iter().enumerate() {
        let note_x = x + width - index as f32 * CHORD_STRING_SPACING;
        if fret < 0 {
            let cross = CHORD_MARKER_HEIGHT / 2.0 - 0.5;
            let center_y = y + CHORD_MARKER_HEIGHT / 2.0;
            frame.stroke(
                &Path::line(
                    Point::new(note_x - cross, center_y - cross),
                    Point::new(note_x + cross, center_y + cross),
                ),
                stroke,
            );
            frame.stroke(
                &Path::line(
                    Point::new(note_x + cross, center_y - cross),
                    Point::new(note_x - cross, center_y + cross),
                ),
                stroke,
            );
        } else if fret == 0 {
            frame.stroke(
                &Path::circle(Point::new(note_x, y + CHORD_MARKER_HEIGHT / 2.0), 1.3),
                stroke,
            );
        } else {
            let position = f32::from(fret) - first_fret as f32 + 1.0;
            if position >= 1.0 && position <= CHORD_FRETS as f32 {
                let dot_y = grid_top + (position - 0.5) * CHORD_FRET_SPACING;
                frame.fill(
                    &Path::circle(Point::new(note_x, dot_y), 1.5),
                    colors.foreground,
                );
            }
        }
    }
}
