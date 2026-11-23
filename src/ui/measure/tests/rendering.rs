use super::{one_measure_song, rest_at, run_of, two_voice_song};
use crate::parser::song_parser::{BendEffect, BendPoint, Chord, QUARTER_TIME, Song};
use crate::ui::measure::CanvasMeasure;
use crate::ui::measure::chord::CHORD_STRING_SPACING;
use crate::ui::measure::layout::{MEASURE_NOTES_PADDING, Staff};
use iced::advanced::graphics::geometry::Renderer as _;
use iced::advanced::renderer::Headless;
use iced::mouse::Cursor;
use iced::widget::canvas::Program;
use iced::{Color, Font, Rectangle, Renderer, Size, Theme};
use std::ops::Range;
use std::rc::Rc;

struct RenderedMeasure {
    pixels: Vec<u8>,
    width: usize,
    staff: Staff,
}

impl RenderedMeasure {
    fn new(song: Rc<Song>) -> Self {
        let strings = song.tracks[0].strings.len();
        let measure = CanvasMeasure::new(0, 0, song, false, false, vec![]);
        let staff = Staff::new(measure.row_needs(), strings);
        let size = Size::new(
            measure.total_measure_len.ceil(),
            measure.vertical_measure_height.ceil(),
        );
        let mut renderer = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(Renderer::new(
                Font::MONOSPACE,
                16.0.into(),
                Some("tiny-skia"),
            ))
            .expect("tiny-skia headless renderer");
        let geometry = measure.draw(
            &Default::default(),
            &renderer,
            &Theme::Light,
            Rectangle::with_size(size),
            Cursor::Unavailable,
        );
        for layer in geometry {
            renderer.draw_geometry(layer);
        }
        let pixels = renderer.screenshot(
            Size::new(size.width as u32, size.height as u32),
            1.0,
            Color::WHITE,
        );
        Self {
            pixels,
            width: size.width as usize,
            staff,
        }
    }

    // Look for dark ink in a region, allowing for antialiasing and faint grid lines.
    fn has_ink(&self, x: Range<usize>, mut y: Range<usize>) -> bool {
        y.any(|row| {
            x.clone()
                .any(|col| self.pixels[4 * (row * self.width + col)] < 128)
        })
    }
}

#[test]
fn a_pre_bend_marks_the_initial_pitch_jump_before_the_release() {
    for (from, to) in [(0, 4), (4, 0)] {
        let mut beats = run_of(&[4]);
        let note = &mut beats[0].notes[0];
        note.string = 4;
        note.effect.bend = Some(BendEffect {
            points: vec![
                BendPoint {
                    position: 0,
                    value: from,
                },
                BendPoint {
                    position: 12,
                    value: to,
                },
            ],
        });
        let score = RenderedMeasure::new(one_measure_song(beats));
        // Between the top two strings both curves cross on the right.
        // Only the pre-bend also crosses on the left, directly after the fret.
        let top = score.staff.top as usize;
        let x = MEASURE_NOTES_PADDING as usize;
        assert_eq!(score.has_ink(x + 8..x + 12, top + 4..top + 9), from > to);
        assert!(score.has_ink(x + 12..x + 22, top + 4..top + 9));
    }
}

#[test]
fn second_voice_eighths_beam_in_pairs_below_the_first_voice() {
    let score = RenderedMeasure::new(two_voice_song(run_of(&[2]), run_of(&[8; 4])));
    let bottom = score.staff.bottom() as usize;
    let lower_top = bottom + score.staff.rows.staff_footer as usize;
    let lower_band = lower_top..lower_top + score.staff.rows.second_rhythm as usize;
    let upper_band = bottom + 1..lower_band.start;
    // Probe halfway between stems. Only the two pairs in the lower voice
    // are joined; the quarter-note boundary between the pairs stays clear.
    let start = MEASURE_NOTES_PADDING as usize;
    let spacing = (score.width - 2 * start) / 4;
    for (gap, beamed) in [true, false, true].into_iter().enumerate() {
        let x = start + spacing * gap + spacing / 2;
        assert_eq!(
            score.has_ink(x..x + 3, lower_band.clone()),
            beamed,
            "gap {gap}"
        );
        assert!(!score.has_ink(x..x + 3, upper_band.clone()));
    }
}

#[test]
fn whole_and_half_rests_sit_on_opposite_sides_of_the_midline() {
    let x = MEASURE_NOTES_PADDING as usize + 3;
    for (duration, hangs_below) in [(1, true), (2, false)] {
        let score = RenderedMeasure::new(one_measure_song(vec![rest_at(duration, QUARTER_TIME)]));
        let mid = (score.staff.top + score.staff.height / 2.0) as usize;
        assert_eq!(score.has_ink(x..x + 4, mid + 1..mid + 3), hangs_below);
        assert_eq!(score.has_ink(x..x + 4, mid - 2..mid), !hangs_below);
    }
}

#[test]
fn chord_diagram_places_the_high_string_on_the_right_and_offsets_the_first_fret() {
    let mut beats = run_of(&[4]);
    beats[0].effect.chord = Some(Chord {
        name: "Am".into(),
        first_fret: Some(5),
        strings: vec![5, 0, -1, -1, 0, 7],
        ..Default::default()
    });
    let score = RenderedMeasure::new(one_measure_song(beats));
    let left = MEASURE_NOTES_PADDING as usize + 3;
    let right = left + 5 * CHORD_STRING_SPACING as usize;
    let chord_top = score.staff.rows.chord_y() as usize;
    // Fifth and seventh frets occupy the first and third spaces of this diagram.
    let first = chord_top + 6..chord_top + 9;
    let third = chord_top + 16..chord_top + 19;
    assert!(score.has_ink(right - 1..right + 1, first.clone()));
    assert!(!score.has_ink(left - 1..left + 1, first));
    assert!(score.has_ink(left - 1..left + 1, third.clone()));
    assert!(!score.has_ink(right - 1..right + 1, third));
}
