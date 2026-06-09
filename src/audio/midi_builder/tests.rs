use super::MidiBuilder;
use super::effects::{apply_triplet_feel, compute_stroke_offsets};
use crate::audio::midi_event::{MidiEvent, MidiEventType};
use crate::audio::playback_order::compute_playback_order;
use crate::parser::song_parser::{
    Beat, BeatStrokeDirection, DURATION_EIGHTH, DURATION_SIXTEENTH, Note, NoteEffect, NoteType,
    TripletFeel,
};
use crate::parser::song_parser_tests::parse_gp_file;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[test]
fn test_midi_events_for_all_files() {
    let test_dir = Path::new("test-files");
    let gold_dir = Path::new("test-files/gold-generated-midi");
    for entry in std::fs::read_dir(test_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let extension = path.extension().unwrap();
        if extension != "gp5"
            && extension != "gp4"
            && extension != "gp3"
            && extension != "gpx"
            && extension != "gp"
        {
            continue;
        }
        let file_name = path.file_name().unwrap().to_str().unwrap();
        eprintln!("Parsing file: {file_name}");
        let file_path = path.to_str().unwrap();
        let song = parse_gp_file(file_path)
            .unwrap_or_else(|err| panic!("Failed to parse file: {file_name}\n{err}"));
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);
        assert!(!events.is_empty(), "No events found for {file_name}");

        // assert sorted by tick
        assert!(events.windows(2).all(|w| w[0].tick <= w[1].tick));
        assert_eq!(events[0].tick, 1);

        // check against golden file
        let gold_file_path = gold_dir.join(format!("{file_name}.txt"));
        if !gold_file_path.exists() {
            // create gold file
            let mut file = std::fs::File::create(&gold_file_path).unwrap();
            for event in &events {
                writeln!(file, "{}", print_event(event)).unwrap();
            }
        }

        // verify against gold file
        validate_gold_rendered_result(&events, gold_file_path);
    }
}

fn print_event(event: &MidiEvent) -> String {
    format!("{:?} {:?} {:?}", event.tick, event.event, event.track)
}

fn validate_gold_rendered_result(events: &[MidiEvent], gold_path: PathBuf) {
    let gold = std::fs::read_to_string(&gold_path).expect("gold file not found!");
    let mut expected_lines = events.iter().map(print_event);
    for (i1, l1) in gold.lines().enumerate() {
        let l2 = expected_lines.next().unwrap();
        if l1.trim_end() != l2.trim_end() {
            println!("## GOLD line {} ##", i1 + 1);
            println!("{}", l1.trim_end());
            println!("## ACTUAL ##");
            println!("{}", l2.trim_end());
            println!("#####");
            assert_eq!(l1, l2, "line {i1} failed for {gold_path:?}");
        }
    }
}

#[test]
fn test_midi_events_for_demo_song() {
    const FILE_PATH: &str = "test-files/Demo v5.gp5";
    let song = parse_gp_file(FILE_PATH).unwrap();
    let song = Rc::new(song);
    let builder = MidiBuilder::new();
    let events = builder.build_for_song(&song);

    assert_eq!(events.len(), 4682);
    assert_eq!(events[0].tick, 1);

    // assert number of tracks
    let track_count = song.tracks.len();
    let unique_tracks: HashSet<_> = events.iter().map(|event| event.track).collect();
    assert_eq!(unique_tracks.len(), track_count + 1); // plus None for info events

    // skip MIDI program messages
    let rhythm_track_events: Vec<_> = events
        .iter()
        .filter(|e| e.track == Some(0))
        .skip(11)
        .collect();

    // print 20 first for debugging
    // for (i, event) in rhythm_track_events.iter().enumerate().take(20) {
    //     eprintln!("{} {:?}", i, event);
    // }

    // C5 ON
    let event = &rhythm_track_events[0];
    assert_eq!(event.tick, 960);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 60, 95)));

    let event = &rhythm_track_events[1];
    assert_eq!(event.tick, 960);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 55, 95)));

    let event = &rhythm_track_events[2];
    assert_eq!(event.tick, 960);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 127)));

    // C5 OFF
    let event = &rhythm_track_events[3];
    assert_eq!(event.tick, 1440);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 60)));

    let event = &rhythm_track_events[4];
    assert_eq!(event.tick, 1440);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 55)));

    let event = &rhythm_track_events[5];
    assert_eq!(event.tick, 1440);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 48)));

    // single note `3` on string `1` (E2)
    let event = &rhythm_track_events[6];
    assert_eq!(event.tick, 1440);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 95)));

    // single note OFF (palm mute)
    let event = &rhythm_track_events[7];
    assert_eq!(event.tick, 1605);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 48)));

    // single note `3` on string `1` (E2)
    let event = &rhythm_track_events[8];
    assert_eq!(event.tick, 1920);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 95)));

    // single note OFF (palm mute)
    let event = &rhythm_track_events[9];
    assert_eq!(event.tick, 2085);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 48)));

    // C5 ON
    let event = &rhythm_track_events[10];
    assert_eq!(event.tick, 2400);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 60, 95)));

    let event = &rhythm_track_events[11];
    assert_eq!(event.tick, 2400);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 55, 95)));

    let event = &rhythm_track_events[12];
    assert_eq!(event.tick, 2400);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 48, 127)));

    // skip MIDI program messages
    let solo_track_events: Vec<_> = events
        .iter()
        .filter(|e| e.track == Some(1))
        .skip(11)
        .collect();

    //print 100 first for debugging
    for (i, event) in solo_track_events.iter().enumerate().take(100) {
        eprintln!("{i} {event:?}");
    }

    // trill ON
    let event = &solo_track_events[0];
    assert_eq!(event.tick, 12480);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOn(2, 72, 95)));

    // trill OFF
    let event = &solo_track_events[1];
    assert_eq!(event.tick, 12720);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOff(2, 72)));

    // trill ON
    let event = &solo_track_events[2];
    assert_eq!(event.tick, 12720);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOn(2, 69, 95)));

    // trill OFF
    let event = &solo_track_events[3];
    assert_eq!(event.tick, 12960);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOff(2, 69)));

    // trill ON
    let event = &solo_track_events[4];
    assert_eq!(event.tick, 12960);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOn(2, 72, 95)));

    // trill OFF
    let event = &solo_track_events[5];
    assert_eq!(event.tick, 13200);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOff(2, 72)));

    // pass some trill notes...

    // trill ON
    let event = &solo_track_events[30];
    assert_eq!(event.tick, 16080);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOn(2, 69, 95)));

    // trill OFF
    let event = &solo_track_events[31];
    assert_eq!(event.tick, 16319);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOff(2, 69)));

    // tremolo ON (repeated section)
    let event = &solo_track_events[32];
    assert_eq!(event.tick, 27840);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOn(2, 60, 95)));

    // tremolo OFF (now a sixteenth note: 240 ticks, matching the reference)
    let event = &solo_track_events[33];
    assert_eq!(event.tick, 28080);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOff(2, 60)));

    // note ON (after all tremolo and repeated sections)
    let event = &solo_track_events[48];
    assert_eq!(event.tick, 77760);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOn(2, 63, 95)));

    // note OFF
    let event = &solo_track_events[49];
    assert_eq!(event.tick, 78240);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOff(2, 63)));

    // note ON hammer
    let event = &solo_track_events[50];
    assert_eq!(event.tick, 78240);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOn(2, 65, 70)));

    // note OFF hammer
    let event = &solo_track_events[51];
    assert_eq!(event.tick, 78720);
    assert_eq!(event.track, Some(1));
    assert!(matches!(event.event, MidiEventType::NoteOff(2, 65)));
}

#[test]
fn test_midi_events_for_bleed() {
    const FILE_PATH: &str = "test-files/Meshuggah - Bleed.gp5";
    let song = parse_gp_file(FILE_PATH).unwrap();
    let song = Rc::new(song);
    let builder = MidiBuilder::new();
    let events = builder.build_for_song(&song);

    assert_eq!(events.len(), 44449);
    assert_eq!(events[0].tick, 1);

    // assert number of tracks
    let track_count = song.tracks.len();
    let unique_tracks: HashSet<_> = events.iter().map(|event| event.track).collect();
    assert_eq!(unique_tracks.len(), track_count);

    // skip MIDI program messages
    let rhythm_track_events: Vec<_> = events
        .iter()
        .filter(|e| e.track == Some(0))
        .skip(11)
        .collect();

    // print 60 first for debugging
    // for (i, event) in rhythm_track_events.iter().enumerate().take(100) {
    //     eprintln!("{} {:?}", i, event);
    // }

    let event = &rhythm_track_events[44];
    assert_eq!(event.tick, 4800);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

    let event = &rhythm_track_events[45];
    assert_eq!(event.tick, 4915);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

    let event = &rhythm_track_events[46];
    assert_eq!(event.tick, 5040);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

    let event = &rhythm_track_events[47];
    assert_eq!(event.tick, 5155);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

    let event = &rhythm_track_events[48];
    assert_eq!(event.tick, 5280);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

    let event = &rhythm_track_events[49];
    assert_eq!(event.tick, 5395);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

    let event = &rhythm_track_events[50];
    assert_eq!(event.tick, 5400);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

    let event = &rhythm_track_events[51];
    assert_eq!(event.tick, 5515);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

    let event = &rhythm_track_events[52];
    assert_eq!(event.tick, 5520);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

    let event = &rhythm_track_events[50];
    assert_eq!(event.tick, 5400);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

    let event = &rhythm_track_events[51];
    assert_eq!(event.tick, 5515);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));

    let event = &rhythm_track_events[52];
    assert_eq!(event.tick, 5520);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOn(0, 39, 95)));

    let event = &rhythm_track_events[53];
    assert_eq!(event.tick, 5635);
    assert_eq!(event.track, Some(0));
    assert!(matches!(event.event, MidiEventType::NoteOff(0, 39)));
}

#[test]
fn playback_order_damage_control() {
    const FILE_PATH: &str = "test-files/John Petrucci - Damage Control (ver 6 by Feio666).gp5";
    let song = parse_gp_file(FILE_PATH).unwrap();
    let headers = &song.measure_headers;

    // discover repeat structure
    let repeats: Vec<(usize, bool, i8, u8)> = headers
        .iter()
        .enumerate()
        .filter(|(_, h)| h.repeat_open || h.repeat_close > 0 || h.repeat_alternative > 0)
        .map(|(i, h)| (i, h.repeat_open, h.repeat_close, h.repeat_alternative))
        .collect();
    assert!(!repeats.is_empty(), "Expected repeat markers");

    let order = compute_playback_order(headers);
    let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();

    // playback order should be longer than header count due to repeats
    assert!(
        order.len() > headers.len(),
        "Playback order ({}) should be > header count ({})",
        order.len(),
        headers.len()
    );

    // verify the first repeat section has alternative endings
    // find first measure with repeat_alternative
    let first_alt = repeats.iter().find(|(_, _, _, alt)| *alt > 0);
    assert!(first_alt.is_some(), "Expected alternative endings");

    // verify repeated measures appear multiple times in playback order
    let first_repeat_open = repeats.iter().find(|(_, open, _, _)| *open).unwrap().0;
    let appearances = indices
        .iter()
        .filter(|&&idx| idx == first_repeat_open)
        .count();
    assert!(
        appearances > 1,
        "First repeated measure should appear more than once"
    );

    // verify all playback ticks are monotonically increasing
    let playback_ticks: Vec<i64> = order
        .iter()
        .map(|(idx, offset)| i64::from(headers[*idx].start) + offset)
        .collect();
    for window in playback_ticks.windows(2) {
        assert!(
            window[0] < window[1],
            "Playback ticks not monotonically increasing: {} >= {}",
            window[0],
            window[1]
        );
    }

    // verify all measure indices are valid
    for (idx, _) in &order {
        assert!(*idx < headers.len(), "Invalid measure index {idx}");
    }

    // build MIDI events and verify they are sorted
    let song = Rc::new(song);
    let builder = MidiBuilder::new();
    let events = builder.build_for_song(&song);
    assert!(!events.is_empty());
    assert!(
        events.windows(2).all(|w| w[0].tick <= w[1].tick),
        "Events not sorted by tick"
    );
}

#[test]
fn triplet_feel_guthrie_eric() {
    const FILE_PATH: &str = "test-files/Guthrie Govan - Eric.gp5";
    let song = parse_gp_file(FILE_PATH).unwrap();

    // verify triplet feel is parsed
    let triplet_measure_indices: Vec<usize> = song
        .measure_headers
        .iter()
        .enumerate()
        .filter(|(_, h)| h.triplet_feel != TripletFeel::None)
        .map(|(i, _)| i)
        .collect();
    assert!(
        !triplet_measure_indices.is_empty(),
        "Expected triplet feel measures in Guthrie Govan - Eric"
    );

    let first_triplet_idx = triplet_measure_indices[0];
    let measure_start = song.measure_headers[first_triplet_idx].start;
    let measure_end = measure_start + song.measure_headers[first_triplet_idx].length();

    // build events and verify they are sorted
    let song = Rc::new(song);
    let builder = MidiBuilder::new();
    let events = builder.build_for_song(&song);
    assert!(!events.is_empty());
    assert!(
        events.windows(2).all(|w| w[0].tick <= w[1].tick),
        "Events not sorted by tick"
    );

    let note_ons: Vec<u32> = events
        .iter()
        .filter(|e| {
            e.tick >= measure_start
                && e.tick < measure_end
                && matches!(e.event, MidiEventType::NoteOn(_, _, _))
        })
        .map(|e| e.tick)
        .collect();

    // if there are consecutive eighth notes, the gaps should be uneven (640 + 320)
    // rather than even (480 + 480)
    if note_ons.len() >= 3 {
        let gaps: Vec<u32> = note_ons.windows(2).map(|w| w[1] - w[0]).collect();
        let has_uneven_gaps = gaps.windows(2).any(|w| w[0] != w[1]);
        // at least some gaps should differ (swing feel)
        assert!(
            has_uneven_gaps || gaps.iter().all(|&g| g != 480),
            "Expected uneven note spacing from triplet feel in measure {} (gaps: {gaps:?})",
            first_triplet_idx + 1
        );
    }
}

#[test]
fn triplet_feel_none_no_change() {
    let beat = Beat {
        start: 960,
        ..Beat::default()
    };
    let adj = apply_triplet_feel(&beat, None, None, TripletFeel::None);
    assert_eq!(adj.start, 960);
    assert_eq!(adj.duration, beat.duration.time());
}

#[test]
fn triplet_feel_eighth_first_beat() {
    // first eighth note on quarter boundary → extended to 2/3 triplet * 2
    let mut beat = Beat {
        start: 960,
        ..Beat::default()
    };
    beat.duration.value = u16::from(DURATION_EIGHTH);
    let adj = apply_triplet_feel(&beat, None, None, TripletFeel::Eighth);
    // triplet_duration = 480 * 2 / 3 = 320, long note = 640
    assert_eq!(adj.start, 960);
    assert_eq!(adj.duration, 640);
}

#[test]
fn triplet_feel_eighth_second_beat() {
    // second eighth note on half-quarter boundary → shortened to 1/3 triplet
    let mut beat = Beat {
        start: 960 + 480, // half-quarter boundary
        ..Beat::default()
    };
    beat.duration.value = u16::from(DURATION_EIGHTH);
    let adj = apply_triplet_feel(&beat, None, None, TripletFeel::Eighth);
    // triplet_duration = 320, short note, start shifts to 960 + 640 = 1600
    assert_eq!(adj.start, 1600);
    assert_eq!(adj.duration, 320);
}

#[test]
fn triplet_feel_preserves_total_time() {
    // first + second beat durations should sum to the original pair
    let mut first = Beat {
        start: 960,
        ..Beat::default()
    };
    first.duration.value = u16::from(DURATION_EIGHTH);
    let mut second = Beat {
        start: 960 + 480,
        ..Beat::default()
    };
    second.duration.value = u16::from(DURATION_EIGHTH);
    let adj1 = apply_triplet_feel(&first, None, Some(&second), TripletFeel::Eighth);
    let adj2 = apply_triplet_feel(&second, Some(&first), None, TripletFeel::Eighth);
    // total should be 960 (one quarter note)
    assert_eq!(adj1.duration + adj2.duration, 960);
    // second starts where first ends
    assert_eq!(adj2.start, adj1.start + adj1.duration);
}

#[test]
fn triplet_feel_wrong_duration_no_change() {
    // quarter note should not be affected by eighth triplet feel
    let beat = Beat {
        start: 960,
        ..Beat::default()
    };
    // default duration is quarter (960), not eighth
    let adj = apply_triplet_feel(&beat, None, None, TripletFeel::Eighth);
    assert_eq!(adj.start, 960);
    assert_eq!(adj.duration, 960);
}

#[test]
fn triplet_feel_sixteenth_pair() {
    // sixteenth pair on eighth-note boundary
    // target_duration = 240, boundary = 480
    // triplet_duration = 240 * 2 / 3 = 160
    let mut first = Beat {
        start: 960,
        ..Beat::default()
    };
    first.duration.value = u16::from(DURATION_SIXTEENTH);
    let mut second = Beat {
        start: 960 + 240,
        ..Beat::default()
    };
    second.duration.value = u16::from(DURATION_SIXTEENTH);
    let adj1 = apply_triplet_feel(&first, None, Some(&second), TripletFeel::Sixteenth);
    let adj2 = apply_triplet_feel(&second, Some(&first), None, TripletFeel::Sixteenth);
    assert_eq!(adj1.start, 960);
    assert_eq!(adj1.duration, 320); // long: 160 * 2
    assert_eq!(adj2.start, 1280); // 960 + 320
    assert_eq!(adj2.duration, 160); // short: 160
    assert_eq!(adj1.duration + adj2.duration, 480); // total = one eighth note
}

fn make_note(string: i8) -> Note {
    let mut note = Note::new(NoteEffect::default());
    note.string = string;
    note.kind = NoteType::Normal;
    note
}

#[test]
fn stroke_offsets_no_stroke() {
    let beat = Beat::default();
    let offsets = compute_stroke_offsets(&beat, 0, 6);
    assert_eq!(offsets, vec![0, 0, 0, 0, 0, 0]);
}

#[test]
fn stroke_offsets_down_stroke() {
    // down stroke: thickest string (6, index 5) plays first
    let mut beat = Beat::default();
    beat.effect.stroke.direction = BeatStrokeDirection::Down;
    beat.notes = vec![make_note(1), make_note(3), make_note(5)];
    let increment = 10;
    let offsets = compute_stroke_offsets(&beat, increment, 6);
    // string 5 (index 4) plays first (offset 0), string 3 (index 2) second, string 1 (index 0) third
    assert_eq!(offsets[4], 0); // string 5: first
    assert_eq!(offsets[2], 10); // string 3: second
    assert_eq!(offsets[0], 20); // string 1: third
    // strings without notes have 0 offset
    assert_eq!(offsets[1], 0);
    assert_eq!(offsets[3], 0);
    assert_eq!(offsets[5], 0);
}

#[test]
fn stroke_offsets_up_stroke() {
    // up stroke: thinnest string (1, index 0) plays first
    let mut beat = Beat::default();
    beat.effect.stroke.direction = BeatStrokeDirection::Up;
    beat.notes = vec![make_note(1), make_note(3), make_note(5)];
    let increment = 10;
    let offsets = compute_stroke_offsets(&beat, increment, 6);
    assert_eq!(offsets[0], 0); // string 1: first
    assert_eq!(offsets[2], 10); // string 3: second
    assert_eq!(offsets[4], 20); // string 5: third
}
