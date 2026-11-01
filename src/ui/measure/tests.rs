use super::beat::{
    beat_annotations, multiple_bend_conflicts, next_note_on_string, note_value, tied_from_note_x,
};
use super::canvas_measure::CanvasMeasure;
use super::canvas_measure::shows_key_signature;
use super::chord::has_diagram;
use super::layout::{
    BEAT_LENGTH, DURATION_WIDTHS, LABEL_CHAR_WIDTH, LABEL_GAP, LABEL_MAX_EXTRA,
    MEASURE_NOTES_PADDING, MIN_BEAT_WIDTH, ROW_CHORD, ROW_LYRIC, ROW_MARKER, RowSpacing,
    beat_natural_width, grace_gap_width, label_extra_width, lyric_extra_width, spacing_for_quarter,
};
use super::rhythm::{
    MAX_BEAMS, beam_leans_right, beam_runs, division_length, is_rest, stem_beams, tuplet_runs,
};
use super::staff::{BarEnd, BarStart, CANVAS_MARGIN, focus_box_top};
use crate::parser::song_parser::{
    Beat, Chord, Duration, KeySignature, MeasureHeader, Note, NoteEffect, QUARTER_TIME,
};
use crate::parser::song_parser::{
    BeatEffects, BendEffect, BendPoint, GraceEffect, Measure, NoteType, QUARTER, Song, Track, Voice,
};
use std::rc::Rc;

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

fn note_on(string: i8, value: i16) -> Note {
    let mut note = Note::new(NoteEffect::default());
    note.string = string;
    note.value = value;
    note
}

fn beat_with(notes: Vec<Note>) -> Beat {
    Beat {
        notes,
        ..Beat::default()
    }
}

fn chord_named(name: &str) -> Chord {
    Chord {
        name: name.to_string(),
        ..Default::default()
    }
}

fn beat_naming(name: &str) -> Beat {
    Beat {
        effect: BeatEffects {
            chord: Some(chord_named(name)),
            ..Default::default()
        },
        ..Beat::default()
    }
}

fn beat_of(value: u16) -> Beat {
    Beat {
        duration: Duration {
            value,
            ..Default::default()
        },
        ..Beat::default()
    }
}

fn sounding(value: u16, dotted: bool) -> Beat {
    let mut beat = Beat {
        duration: Duration {
            value,
            dotted,
            ..Default::default()
        },
        ..Beat::default()
    };
    beat.notes.push(Note::new(NoteEffect::default()));
    beat
}

fn run_of(durations: &[u16]) -> Vec<Beat> {
    let mut start = QUARTER_TIME;
    durations
        .iter()
        .map(|&value| {
            let mut beat = sounding(value, false);
            beat.start = start;
            start += beat.duration.time();
            beat
        })
        .collect()
}

#[test]
fn a_lone_flag_points_forward() {
    // a note standing alone flags forward, the way a written eighth does
    assert!(beam_leans_right(0, 0, 0));
    // opening a run, a partial beam also points into the run
    assert!(beam_leans_right(0, 0, 3));
    // but inside or closing one it points back at what it follows
    assert!(!beam_leans_right(2, 0, 3));
    assert!(!beam_leans_right(3, 0, 3));
}

#[test]
fn a_silence_is_a_rest_but_an_empty_beat_is_not() {
    // no notes and time of its own: a rest
    assert!(is_rest(&beat_of(QUARTER)));
    // a note sounding: not a rest
    assert!(!is_rest(&sounding(QUARTER, false)));
    // flagged empty, so it carries no time and needs no glyph
    let empty = Beat {
        empty: true,
        ..beat_of(QUARTER)
    };
    assert!(!is_rest(&empty));
}

#[test]
fn short_notes_beam_within_a_division() {
    // eight eighths in 4/4 beam in pairs, one pair per quarter
    let beats = run_of(&[8; 8]);
    assert_eq!(
        beam_runs(&beats, QUARTER_TIME, QUARTER_TIME),
        vec![(0, 1), (2, 3), (4, 5), (6, 7)]
    );
}

#[test]
fn a_long_note_breaks_the_run() {
    // a quarter in the middle is not beamed, and parts what surrounds it
    let beats = run_of(&[8, 8, QUARTER, 8, 8]);
    assert_eq!(
        beam_runs(&beats, QUARTER_TIME, QUARTER_TIME),
        vec![(0, 1), (3, 4)]
    );
}

#[test]
fn a_run_breaks_at_a_division_boundary() {
    // an eighth landing off the beat carries its run into the next
    // quarter, where it starts afresh rather than beaming across
    let beats = run_of(&[QUARTER, 8, 8, 8]);
    let runs = beam_runs(&beats, QUARTER_TIME, QUARTER_TIME);
    assert_eq!(runs, vec![(1, 2), (3, 3)]);
}

#[test]
fn a_rest_breaks_the_run() {
    let mut beats = run_of(&[8; 4]);
    beats[1].notes.clear();
    let runs = beam_runs(&beats, QUARTER_TIME, QUARTER_TIME);
    assert!(
        runs.iter()
            .all(|&(first, last)| !(first..=last).contains(&1))
    );
}

#[test]
fn compound_time_beams_in_threes() {
    // 6/8 groups by the dotted quarter, so six eighths make two runs
    let beats = run_of(&[8; 6]);
    let compound = QUARTER_TIME + QUARTER_TIME / 2;
    assert_eq!(
        beam_runs(&beats, QUARTER_TIME, compound),
        vec![(0, 2), (3, 5)]
    );
}

#[test]
fn the_division_follows_the_time_signature() {
    let mut header = MeasureHeader::default();
    assert_eq!(division_length(&header), QUARTER_TIME);
    header.time_signature.numerator = 6;
    header.time_signature.denominator.value = 8;
    assert_eq!(division_length(&header), QUARTER_TIME + QUARTER_TIME / 2);
    // 7/8 is not a compound meter, so it groups by the quarter
    header.time_signature.numerator = 7;
    assert_eq!(division_length(&header), QUARTER_TIME);
}

#[test]
fn a_malformed_duration_cannot_overrun_the_stem() {
    // a corrupt file can name a duration far shorter than music uses;
    // its beams would be drawn past the stem and over the staff
    assert_eq!(stem_beams(&sounding(16384, false)), Some(MAX_BEAMS));
    // the deepest real duration stays under the limit
    assert!(stem_beams(&sounding(64, false)).unwrap() < MAX_BEAMS);
}

#[test]
fn a_stem_carries_one_beam_per_halving() {
    // a whole note stands alone, everything shorter grows a stem
    assert_eq!(stem_beams(&sounding(1, false)), None);
    assert_eq!(stem_beams(&sounding(2, false)), Some(0));
    assert_eq!(stem_beams(&sounding(QUARTER, false)), Some(0));
    assert_eq!(stem_beams(&sounding(8, false)), Some(1));
    assert_eq!(stem_beams(&sounding(16, false)), Some(2));
    assert_eq!(stem_beams(&sounding(32, false)), Some(3));
    assert_eq!(stem_beams(&sounding(64, false)), Some(4));
}

#[test]
fn a_silent_beat_carries_no_stem() {
    // rests are drawn on the staff, not stemmed under it
    assert_eq!(stem_beams(&beat_of(QUARTER)), None);
}

#[test]
fn a_note_is_as_wide_as_it_is_long() {
    // spaced by a measure of quarters, a half note takes twice the room
    let spacing = spacing_for_quarter(&beat_of(QUARTER).duration);
    let quarter = beat_natural_width(&beat_of(QUARTER), None, spacing);
    let half = beat_natural_width(&beat_of(2), None, spacing);
    let whole = beat_natural_width(&beat_of(1), None, spacing);
    assert!((half - quarter * 2.0).abs() < 0.01);
    assert!((whole - quarter * 4.0).abs() < 0.01);
}

#[test]
fn short_notes_open_the_measure_out() {
    // a measure of sixteenths is spaced more widely per quarter than a
    // measure of quarters, so the sixteenths stay readable
    let by_quarter = spacing_for_quarter(&beat_of(QUARTER).duration);
    let by_sixteenth = spacing_for_quarter(&beat_of(16).duration);
    assert!(by_sixteenth > by_quarter * 3.0);

    // yet a single sixteenth stays narrower than a single quarter
    let sixteenth = beat_natural_width(&beat_of(16), None, by_sixteenth);
    let quarter = beat_natural_width(&beat_of(QUARTER), None, by_quarter);
    assert!(sixteenth < quarter);
}

#[test]
fn long_notes_keep_their_proportions() {
    // a measure of whole notes is spaced by the whole note, and one must
    // not be widened by a floor meant to protect short notes
    let spacing = spacing_for_quarter(&beat_of(1).duration);
    let whole = beat_natural_width(&beat_of(1), None, spacing);
    assert!(
        (whole - DURATION_WIDTHS[0]).abs() < 0.01,
        "whole note {whole}"
    );

    // the same holds a step down
    let spacing = spacing_for_quarter(&beat_of(2).duration);
    let half = beat_natural_width(&beat_of(2), None, spacing);
    assert!((half - DURATION_WIDTHS[1]).abs() < 0.01, "half note {half}");
}

#[test]
fn an_empty_beat_takes_no_room() {
    // it carries no time, so it may not take space from what does
    let empty = Beat {
        empty: true,
        ..sounding(QUARTER, false)
    };
    assert!(beat_natural_width(&empty, None, BEAT_LENGTH) < f32::EPSILON);
}

#[test]
fn a_beat_never_narrows_below_what_it_carries() {
    // a whole note in a measure of sixteenths is wide; a sixteenth in a
    // measure of quarters would be hair-thin, so a floor holds it
    let by_quarter = spacing_for_quarter(&beat_of(QUARTER).duration);
    let squeezed = beat_natural_width(&beat_of(64), None, by_quarter);
    assert!(squeezed >= MIN_BEAT_WIDTH);
}

#[test]
fn a_long_chord_name_widens_its_beat() {
    // a short name fits beside the next one
    let short = beat_naming("C");
    assert!(
        (beat_natural_width(&short, Some(&beat_naming("G")), BEAT_LENGTH) - BEAT_LENGTH).abs()
            < f32::EPSILON
    );

    // a long one pushes the next chord away
    let long = beat_naming("Bbsus4add9");
    assert!(beat_natural_width(&long, Some(&beat_naming("G")), BEAT_LENGTH) > BEAT_LENGTH);

    // but claims nothing where the next beat names no chord to run into
    assert!(
        (beat_natural_width(&long, Some(&Beat::default()), BEAT_LENGTH) - BEAT_LENGTH).abs()
            < f32::EPSILON
    );
    assert!((beat_natural_width(&long, None, BEAT_LENGTH) - BEAT_LENGTH).abs() < f32::EPSILON);
}

#[test]
fn beat_text_widens_its_beat_too() {
    let verse = Beat {
        text: "Verse".to_string(),
        ..Beat::default()
    };
    let next = Beat {
        text: "Chorus".to_string(),
        ..Beat::default()
    };
    assert!(beat_natural_width(&verse, Some(&next), BEAT_LENGTH) > BEAT_LENGTH);
    assert!(
        (beat_natural_width(&verse, Some(&Beat::default()), BEAT_LENGTH) - BEAT_LENGTH).abs()
            < f32::EPSILON
    );
}

#[test]
fn a_label_on_a_short_note_still_gets_its_room() {
    // a sixteenth's base is narrower than a quarter's, so its label
    // needs more extra, not the same: reserving against a fixed width
    // left labels on short notes a few pixels shy
    let label = "spe-cial";
    let short_base = MIN_BEAT_WIDTH;
    let needed = label.chars().count() as f32 * LABEL_CHAR_WIDTH + LABEL_GAP;
    assert!((short_base + label_extra_width(label, short_base) - needed).abs() < f32::EPSILON);
    // and a base already wide enough claims nothing
    assert!(label_extra_width(label, needed) < f32::EPSILON);
}

#[test]
fn a_long_syllable_widens_its_beat() {
    let lyrics =
        |words: &[&str]| -> Vec<String> { words.iter().map(|w| (*w).to_string()).collect() };
    // a short word fits the beat it is sung on
    assert!(lyric_extra_width(&lyrics(&["How", "ma"]), 0, BEAT_LENGTH) < f32::EPSILON);
    // a long one pushes the next word away
    assert!(lyric_extra_width(&lyrics(&["spe-cial", "peo"]), 0, BEAT_LENGTH) > 0.0);
    // but never past the limit, however long the word
    let very_long = lyrics(&["supercalifragilistic", "next"]);
    assert!((lyric_extra_width(&very_long, 0, BEAT_LENGTH) - LABEL_MAX_EXTRA).abs() < f32::EPSILON);
    // nothing is claimed when the next beat is silent: the word may lean
    // into the space after it
    assert!(lyric_extra_width(&lyrics(&["spe-cial", ""]), 0, BEAT_LENGTH) < f32::EPSILON);
    assert!(lyric_extra_width(&lyrics(&["spe-cial"]), 0, BEAT_LENGTH) < f32::EPSILON);
    // and none at all where nothing is sung
    assert!(lyric_extra_width(&lyrics(&["", "next"]), 0, BEAT_LENGTH) < f32::EPSILON);
}

#[test]
fn the_default_key_is_not_announced() {
    let c_major = KeySignature::new(0, false);
    let header = MeasureHeader {
        key_signature: c_major,
        ..Default::default()
    };
    // most songs open in C major: saying so on every first measure is noise
    assert!(!shows_key_signature(&header, None));
    // anything else is worth stating
    let e_flat = MeasureHeader {
        key_signature: KeySignature::new(-3, false),
        ..Default::default()
    };
    assert!(shows_key_signature(&e_flat, None));
    // and so is a change, back to the default included
    assert!(shows_key_signature(&header, Some(&e_flat)));
    assert!(!shows_key_signature(&header, Some(&header)));
}

#[test]
fn a_chord_without_a_fingering_shows_only_its_name() {
    let named = Chord {
        name: "C5".to_string(),
        strings: vec![-1; 6],
        ..Default::default()
    };
    assert!(!has_diagram(&named));

    let fingered = Chord {
        name: "C5".to_string(),
        strings: vec![-1, -1, 5, 5, 3, -1],
        ..Default::default()
    };
    assert!(has_diagram(&fingered));
}

#[test]
fn a_diagram_claims_the_room_it_needs() {
    let fingered = Chord {
        strings: vec![-1, -1, 5, 5, 3, -1],
        ..Default::default()
    };
    let beat = Beat {
        effect: BeatEffects {
            chord: Some(fingered),
            ..Default::default()
        },
        ..Beat::default()
    };
    // the grid is wider than a bare beat, so the beat grows for it
    assert!(beat_natural_width(&beat, None, BEAT_LENGTH) > BEAT_LENGTH);
    assert!(
        (beat_natural_width(&Beat::default(), None, BEAT_LENGTH) - BEAT_LENGTH).abs()
            < f32::EPSILON
    );
}

#[test]
fn focus_box_stays_off_the_canvas_edge() {
    // a line with no annotations at all: the header alone sits against
    // the top of the canvas, where the box line would be clipped
    let bare = RowSpacing::default();
    let staff_y = bare.first_string_y();
    assert!(staff_y - focus_box_top(bare, staff_y) >= CANVAS_MARGIN);

    // with annotation rows there is room, so the box rides the boundary
    let annotated = RowSpacing {
        marker: ROW_MARKER,
        ..Default::default()
    };
    let staff_y = annotated.first_string_y();
    assert!((focus_box_top(annotated, staff_y) - annotated.staff_header).abs() < f32::EPSILON);
}

#[test]
fn slide_reaches_the_next_note_on_its_string() {
    let beats = vec![
        beat_with(vec![note_on(1, 5)]),
        // an intervening beat that does not touch string 1
        beat_with(vec![note_on(2, 7)]),
        beat_with(vec![note_on(1, 9)]),
    ];
    let positions = [0.0, 10.0, 20.0];
    assert_eq!(
        next_note_on_string(&beats, &positions, 0, 1),
        Some((20.0, 9))
    );
    // nothing follows the last note on the string
    assert_eq!(next_note_on_string(&beats, &positions, 2, 1), None);
}

#[test]
fn tie_hangs_from_the_closest_earlier_note() {
    let beats = vec![
        beat_with(vec![note_on(1, 5)]),
        beat_with(vec![note_on(2, 7)]),
        beat_with(vec![note_on(1, 5)]),
    ];
    let positions = [0.0, 10.0, 20.0];
    assert_eq!(tied_from_note_x(&beats, &positions, 2, 1), Some(0.0));
    // nothing precedes the first beat
    assert_eq!(tied_from_note_x(&beats, &positions, 0, 1), None);
}

#[test]
fn a_rest_breaks_the_tie() {
    let beats = vec![
        beat_with(vec![note_on(1, 5)]),
        // a rest: no notes at all
        beat_with(vec![]),
        beat_with(vec![note_on(1, 5)]),
    ];
    let positions = [0.0, 10.0, 20.0];
    assert_eq!(tied_from_note_x(&beats, &positions, 2, 1), None);
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

#[test]
fn the_song_opens_with_a_section_line() {
    let header = MeasureHeader::default();
    assert_eq!(BarStart::of(&header, 0, true), BarStart::Section);
    // a later measure only draws its opening line at the start of a staff line
    assert_eq!(BarStart::of(&header, 3, true), BarStart::Line);
    assert_eq!(BarStart::of(&header, 3, false), BarStart::None);
}

#[test]
fn a_repeat_sign_wins_over_the_other_opening_lines() {
    let header = MeasureHeader {
        repeat_open: true,
        ..Default::default()
    };
    assert_eq!(BarStart::of(&header, 0, true), BarStart::Repeat);
    assert_eq!(BarStart::of(&header, 3, false), BarStart::Repeat);
}

#[test]
fn the_last_measure_closes_the_song_unless_it_repeats() {
    let plain = MeasureHeader::default();
    assert_eq!(BarEnd::of(&plain, false), BarEnd::Line);
    assert_eq!(BarEnd::of(&plain, true), BarEnd::Section);
    let repeated = MeasureHeader {
        repeat_close: 2,
        ..Default::default()
    };
    assert_eq!(BarEnd::of(&repeated, true), BarEnd::Repeat(2));
}

/// A song of one six-string track with a single measure of these beats.
fn one_measure_song(beats: Vec<Beat>) -> Rc<Song> {
    Rc::new(Song {
        tracks: vec![Track {
            strings: (1..=6).map(|string| (string, 40)).collect(),
            measures: vec![Measure {
                voices: vec![Voice {
                    measure_index: 0,
                    beats,
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        measure_headers: vec![MeasureHeader::default()],
        ..Default::default()
    })
}

fn four_quarters() -> Vec<Beat> {
    (0..4).map(|_| beat_with(vec![note_on(1, 0)])).collect()
}

#[test]
fn a_click_selects_the_beat_under_it() {
    let measure = CanvasMeasure::new(
        0,
        0,
        one_measure_song(four_quarters()),
        false,
        false,
        vec![],
    );
    let width = measure.total_measure_len;
    let beat_width = (width - MEASURE_NOTES_PADDING * 2.0) / 4.0;
    // the leading padding belongs to the first beat
    assert_eq!(measure.beat_at_x(0.0, width), 0);
    assert_eq!(
        measure.beat_at_x(MEASURE_NOTES_PADDING + 1.5 * beat_width, width),
        1
    );
    assert_eq!(measure.beat_at_x(width, width), 3);
}

#[test]
fn a_click_follows_a_stretched_measure() {
    let measure = CanvasMeasure::new(
        0,
        0,
        one_measure_song(four_quarters()),
        false,
        false,
        vec![],
    );
    let width = measure.total_measure_len;
    let beat_width = (width - MEASURE_NOTES_PADDING * 2.0) / 4.0;
    // twice as wide: the beats take the extra room, the padding does not
    let stretched = width * 2.0;
    let x = MEASURE_NOTES_PADDING + 2.5 * (beat_width + width / 4.0);
    assert_eq!(measure.beat_at_x(x, stretched), 2);
}

#[test]
fn a_click_past_the_end_skips_the_closing_empty_beat() {
    let mut beats = four_quarters();
    beats.push(Beat {
        empty: true,
        ..Beat::default()
    });
    let measure = CanvasMeasure::new(0, 0, one_measure_song(beats), false, false, vec![]);
    let width = measure.total_measure_len;
    assert_eq!(measure.beat_at_x(width * 2.0, width), 3);
}

#[test]
fn a_plain_measure_needs_no_row_above_the_staff() {
    let song = one_measure_song(four_quarters());
    let header = &song.measure_headers[0];
    let measure = &song.tracks[0].measures[0];
    let rows = RowSpacing::for_measure(measure, header, false, false, false);
    assert_eq!(
        rows.first_string_y(),
        RowSpacing::default().first_string_y()
    );
    // the stems hang below it
    assert!(rows.staff_footer > RowSpacing::default().staff_footer);
}

#[test]
fn each_annotation_claims_its_row() {
    let mut beats = four_quarters();
    beats[1].effect.chord = Some(chord_named("Am"));
    let song = one_measure_song(beats);
    let header = &song.measure_headers[0];
    let measure = &song.tracks[0].measures[0];
    let rows = RowSpacing::for_measure(measure, header, true, false, true);
    assert_eq!(rows.marker, ROW_MARKER);
    assert_eq!(rows.chord, ROW_CHORD);
    assert_eq!(rows.lyric, ROW_LYRIC);
    assert_eq!(rows.signature, 0.0);
    // the staff starts below the rows above it
    let plain = RowSpacing::default();
    assert_eq!(
        rows.first_string_y(),
        plain.first_string_y() + ROW_MARKER + ROW_CHORD
    );
}

#[test]
fn a_line_keeps_the_tallest_of_each_row() {
    let mut line = RowSpacing {
        marker: ROW_MARKER,
        ..RowSpacing::default()
    };
    line.merge(RowSpacing {
        chord: ROW_CHORD,
        ..RowSpacing::default()
    });
    assert_eq!(line.marker, ROW_MARKER);
    assert_eq!(line.chord, ROW_CHORD);
}

#[test]
fn shared_annotations_are_listed_once_in_order() {
    let mut muted = note_on(1, 0);
    muted.effect.palm_mute = true;
    let mut ringing = note_on(2, 0);
    ringing.effect.palm_mute = true;
    ringing.effect.let_ring = true;
    assert_eq!(
        beat_annotations(&beat_with(vec![muted, ringing])),
        ["L.R", "P.M"]
    );
}

#[test]
fn a_note_reads_as_its_fret_and_kind() {
    let mut note = note_on(1, 3);
    note.kind = NoteType::Normal;
    assert_eq!(note_value(&note), "3");
    note.effect.ghost_note = true;
    assert_eq!(note_value(&note), "(3)");
    note.kind = NoteType::Dead;
    assert_eq!(note_value(&note), "x");
    // a tie draws an arc instead
    note.kind = NoteType::Tie;
    assert_eq!(note_value(&note), "");
}

#[test]
fn a_dead_grace_note_takes_less_room_than_a_two_digit_one() {
    assert_eq!(grace_gap_width(&beat_with(vec![note_on(1, 0)])), 0.0);
    let graced = |fret: i8, is_dead: bool| {
        let mut note = note_on(1, 0);
        note.effect.grace = Some(GraceEffect {
            fret,
            is_dead,
            ..Default::default()
        });
        beat_with(vec![note])
    };
    assert!(grace_gap_width(&graced(0, true)) < grace_gap_width(&graced(12, false)));
}

fn bent(string: i8, from: i8, to: i8) -> Note {
    let mut note = note_on(string, 5);
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
    note
}

#[test]
fn only_the_highest_string_labels_bends_moving_together() {
    let beat = beat_with(vec![bent(1, 0, 4), bent(2, 0, 4)]);
    let movements = [4];
    assert!(!multiple_bend_conflicts(&beat, &beat.notes[0], &movements));
    assert!(multiple_bend_conflicts(&beat, &beat.notes[1], &movements));
}

#[test]
fn bends_moving_apart_label_none() {
    let beat = beat_with(vec![bent(1, 0, 4), bent(2, 4, 0)]);
    assert!(multiple_bend_conflicts(&beat, &beat.notes[0], &[4]));
}
