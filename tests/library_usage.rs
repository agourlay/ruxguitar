//! Integration tests for ruxguitar library usage.
//!
//! These tests verify that the library can be used as a dependency
//! from external projects.

use ruxguitar::{
    parse_gp_data, MidiBuilder, MidiEventType, RuxError, Song, FIRST_TICK, QUARTER_TIME,
};
use std::io::Read;
use std::rc::Rc;

/// Test that all major types are accessible from the library.
#[test]
fn test_types_accessible() {
    // This test verifies that the public API types compile and are usable.
    // If any re-export is missing, this test will fail to compile.

    fn _assert_types() {
        let _: fn(&[u8]) -> Result<Song, RuxError> = parse_gp_data;
        let _: u32 = FIRST_TICK;
        let _: u32 = QUARTER_TIME;
    }
}

/// Test parsing a GP5 file from the test-files directory.
#[test]
fn test_parse_gp5_file() {
    let mut file = std::fs::File::open("test-files/Demo v5.gp5").expect("Failed to open test file");
    let mut file_data: Vec<u8> = vec![];
    file.read_to_end(&mut file_data)
        .expect("Failed to read test file");

    let song = parse_gp_data(&file_data).expect("Failed to parse GP5 file");

    // Verify basic song structure
    assert!(
        !song.tracks.is_empty(),
        "Song should have at least one track"
    );
    assert!(
        !song.measure_headers.is_empty(),
        "Song should have at least one measure"
    );
}

/// Test generating MIDI events from a parsed song.
#[test]
fn test_midi_generation() {
    let mut file = std::fs::File::open("test-files/Demo v5.gp5").expect("Failed to open test file");
    let mut file_data: Vec<u8> = vec![];
    file.read_to_end(&mut file_data)
        .expect("Failed to read test file");

    let song = parse_gp_data(&file_data).expect("Failed to parse GP5 file");
    let song_rc = Rc::new(song);
    let midi_events = MidiBuilder::new().build_for_song(&song_rc);

    // Verify MIDI events were generated
    assert!(!midi_events.is_empty(), "Should generate MIDI events");

    // Verify we have expected event types
    let has_note_on = midi_events
        .iter()
        .any(|e| matches!(e.event, MidiEventType::NoteOn { .. }));
    assert!(has_note_on, "Should have NoteOn events");
}

/// Test error handling for invalid data.
#[test]
fn test_parse_error() {
    let invalid_data = vec![0u8; 10]; // Not a valid GP file
    let result = parse_gp_data(&invalid_data);

    assert!(result.is_err(), "Should return error for invalid data");
    let err = result.unwrap_err();
    assert!(
        matches!(err, RuxError::ParsingError(_)),
        "Should be a ParsingError"
    );
}
