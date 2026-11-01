//! Rhythm under the staff: stems, beams, rests, dots and tuplet brackets.

use crate::parser::song_parser::{Beat, Duration, MeasureHeader, QUARTER_TIME};
use crate::ui::utils::TablatureColors;
use iced::advanced::text::Shaping::Auto;
use iced::widget::canvas::{Frame, Path, Stroke, Text};
use iced::widget::text::Alignment;
use iced::{Point, Renderer, Size};

// A stem hangs under the staff, its beams stacked up from the far end.
const STEM_TOP: f32 = 5.0;
const STEM_LENGTH: f32 = 21.0;
const BEAM_SPACING: f32 = 5.0;
const BEAM_THICKNESS: f32 = 1.8;
const FLAG_WIDTH: f32 = 6.0;
// A sixty-fourth note carries four beams, the deepest written music asks
// for. A malformed duration may claim more than the stem could hold.
pub(super) const MAX_BEAMS: usize = 5;

/// How a beat's rhythm is drawn under the tab: `None` when it carries no
/// stem, otherwise the number of beams the stem needs.
///
/// A whole note has no stem; a half note gets a short one; a quarter a full
/// one; and every halving below that adds a beam, as TuxGuitar does.
pub(super) fn stem_beams(beat: &Beat) -> Option<usize> {
    if beat.notes.is_empty() || beat.duration.value <= 1 {
        // a silence is drawn as a rest on the staff instead
        return None;
    }
    let halvings = f32::from(beat.duration.value).log2().round() as usize;
    Some(halvings.saturating_sub(2).min(MAX_BEAMS))
}

/// Whether a beat is a silence: it takes its time without sounding.
///
/// A beat flagged empty carries no time of its own, so it is not a rest.
pub(super) const fn is_rest(beat: &Beat) -> bool {
    beat.notes.is_empty() && !beat.empty
}

/// The rest of a silent beat, centred on the staff, as TuxGuitar places it.
///
/// TuxGuitar draws engraved glyphs; these are plain shapes carrying the same
/// reading: a bar hung under the line for a whole, sitting on it for a half,
/// a zigzag for a quarter, and a stroke with one hook per halving below that.
pub(super) fn draw_rest(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    beat: &Beat,
    x: f32,
    staff_top_y: f32,
    staff_height: f32,
) {
    let center_y = staff_top_y + staff_height / 2.0;
    let stroke = Stroke::default()
        .with_width(1.4)
        .with_color(colors.foreground);
    match beat.duration.value {
        // a whole rest hangs below its line, a half rest sits on it
        1 | 2 => {
            let width = 7.0;
            let height = 3.0;
            let top = if beat.duration.value == 1 {
                center_y
            } else {
                center_y - height
            };
            frame.fill_rectangle(
                Point::new(x, top),
                Size::new(width, height),
                colors.foreground,
            );
        }
        4 => {
            // the zigzag of a quarter rest, folded down the staff
            let top = center_y - 7.0;
            let path = Path::new(|p| {
                p.move_to(Point::new(x, top));
                p.line_to(Point::new(x + 5.0, top + 4.5));
                p.line_to(Point::new(x + 1.0, top + 8.0));
                p.line_to(Point::new(x + 6.0, top + 13.0));
            });
            frame.stroke(&path, stroke);
        }
        value => {
            // a leaning stroke, with a hook per halving below the quarter,
            // held to the same depth as the beams on a stem
            let hooks = (f32::from(value.max(8)).log2().round() as usize - 2).min(MAX_BEAMS);
            let top = center_y - 6.0;
            let bottom = top + 5.0 + hooks as f32 * 3.5;
            frame.stroke(
                &Path::line(Point::new(x + 5.0, top), Point::new(x + 1.0, bottom)),
                stroke,
            );
            for hook in 0..hooks {
                let hook_y = top + hook as f32 * 3.5;
                frame.fill(
                    &Path::circle(Point::new(x + 4.5 - hook as f32 * 0.8, hook_y), 1.4),
                    colors.foreground,
                );
            }
        }
    }
    // just clear of the glyph, which is about seven wide
    draw_duration_dot(frame, colors, beat, x + 9.0, center_y);
}

/// How long a run of short notes may be before it breaks, from TuxGuitar's
/// `getDivisionLength`: a quarter note, or a dotted one in compound time.
pub(super) const fn division_length(header: &MeasureHeader) -> u32 {
    let signature = &header.time_signature;
    if signature.denominator.value == 8 && signature.numerator.is_multiple_of(3) {
        QUARTER_TIME + QUARTER_TIME / 2
    } else {
        QUARTER_TIME
    }
}

/// Runs of beats beamed together: consecutive sounding notes of an eighth or
/// shorter falling in one division of the measure, like TuxGuitar's `canJoin`.
///
/// Each run is the inclusive index range of the beats it joins.
pub(super) fn beam_runs(beats: &[Beat], measure_start: u32, division: u32) -> Vec<(usize, usize)> {
    let joinable = |beat: &Beat| stem_beams(beat).is_some_and(|beams| beams > 0);
    let division_of = |beat: &Beat| beat.start.saturating_sub(measure_start) / division.max(1);

    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (index, beat) in beats.iter().enumerate() {
        if !joinable(beat) {
            continue;
        }
        match runs.last_mut() {
            // a run carries on while the beats stay adjacent and in the same
            // division of the measure
            Some(run) if run.1 + 1 == index && division_of(&beats[run.1]) == division_of(beat) => {
                run.1 = index;
            }
            _ => runs.push((index, index)),
        }
    }
    runs
}

/// The rhythm under the staff: a stem per note, beams over the runs that
/// join, and a flag where a note stands alone.
pub(super) fn draw_rhythm(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    beats: &[Beat],
    beat_positions: &[f32],
    tab_bottom_y: f32,
    header: &MeasureHeader,
) {
    let stem_x = |index: usize| beat_positions.get(index).copied().unwrap_or(0.0) + 3.0;
    let top = tab_bottom_y + STEM_TOP;
    let bottom = top + STEM_LENGTH;
    let stroke = Stroke::default()
        .with_width(1.0)
        .with_color(colors.foreground);

    for (index, beat) in beats.iter().enumerate() {
        // rests carry their own dot; empty beats carry nothing at all
        if beat.notes.is_empty() {
            continue;
        }
        let x = stem_x(index);
        if let Some(beams) = stem_beams(beat) {
            // a half note is stemmed only half way, so it reads as the
            // longer note
            let stem_top = if beat.duration.value == 2 {
                top + STEM_LENGTH / 2.0
            } else {
                top
            };
            frame.stroke(
                &Path::line(Point::new(x, stem_top), Point::new(x, bottom)),
                stroke,
            );
            // the dot clears the flag and the beams stacked up the stem,
            // but never leaves the stem itself
            let dot_y = (bottom - beams as f32 * BEAM_SPACING).max(stem_top);
            draw_duration_dot(frame, colors, beat, x + FLAG_WIDTH + 2.0, dot_y);
        } else {
            // a dotted whole note has no stem, yet keeps its dot, as
            // TuxGuitar draws it
            draw_duration_dot(frame, colors, beat, x, bottom);
        }
    }

    let beam_stroke = Stroke::default()
        .with_width(BEAM_THICKNESS)
        .with_color(colors.foreground);
    for (first, last) in beam_runs(beats, header.start, division_length(header)) {
        let deepest = (first..=last)
            .filter_map(|i| stem_beams(&beats[i]))
            .max()
            .unwrap_or(0);
        // one bar per level, broken wherever the notes under it are longer
        for level in 1..=deepest {
            let y = bottom - (level - 1) as f32 * BEAM_SPACING;
            let mut bar: Option<(usize, usize)> = None;
            for (index, beat) in beats.iter().enumerate().take(last + 1).skip(first) {
                let deep_enough = stem_beams(beat).is_some_and(|beams| beams >= level);
                match (&mut bar, deep_enough) {
                    (Some(open), true) => open.1 = index,
                    (None, true) => bar = Some((index, index)),
                    (Some(open), false) => {
                        draw_beam(frame, beam_stroke, *open, first, last, &stem_x, y);
                        bar = None;
                    }
                    (None, false) => {}
                }
            }
            if let Some(open) = bar {
                draw_beam(frame, beam_stroke, open, first, last, &stem_x, y);
            }
        }
    }
}

/// Which way a stub over a single note points: forward for a note standing
/// alone or opening its run, back towards the note it follows otherwise.
pub(super) const fn beam_leans_right(from: usize, run_first: usize, run_last: usize) -> bool {
    run_first == run_last || from == run_first
}

/// One bar of a beam. A bar over a single note is a stub, leaning towards
/// the run it belongs to.
fn draw_beam(
    frame: &mut Frame<Renderer>,
    stroke: Stroke,
    bar: (usize, usize),
    run_first: usize,
    run_last: usize,
    stem_x: &impl Fn(usize) -> f32,
    y: f32,
) {
    let (from, to) = bar;
    let (x1, x2) = if from == to {
        let x = stem_x(from);
        if beam_leans_right(from, run_first, run_last) {
            (x, x + FLAG_WIDTH)
        } else {
            (x - FLAG_WIDTH, x)
        }
    } else {
        (stem_x(from), stem_x(to))
    };
    frame.stroke(&Path::line(Point::new(x1, y), Point::new(x2, y)), stroke);
}

/// The dot of a dotted duration, at the position its caller sets aside.
fn draw_duration_dot(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    beat: &Beat,
    dot_x: f32,
    y: f32,
) {
    if !beat.duration.dotted && !beat.duration.double_dotted {
        return;
    }
    frame.fill(&Path::circle(Point::new(dot_x, y), 1.2), colors.foreground);
    if beat.duration.double_dotted {
        frame.fill(
            &Path::circle(Point::new(dot_x + 3.5, y), 1.2),
            colors.foreground,
        );
    }
}

/// Group consecutive beats sharing a tuplet division, like TuxGuitar's
/// `paintDivisionTypes`. A run ends when the division changes or when its
/// accumulated duration completes a whole group.
///
/// Returns `(group size, first beat x, last beat x)` per run.
pub(super) fn tuplet_runs(beats: &[Beat], beat_positions: &[f32]) -> Vec<(u8, f32, f32)> {
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
pub(super) fn draw_tuplet_bracket(
    frame: &mut Frame<Renderer>,
    colors: TablatureColors,
    enters: u8,
    x1: f32,
    x2: f32,
    y: f32,
) {
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
        let stroke = Stroke::default()
            .with_width(0.8)
            .with_color(colors.foreground);
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
