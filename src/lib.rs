//! Ruxguitar - Guitar Pro tablature parser and MIDI generator
//!
//! This library provides:
//! - Parsing of Guitar Pro 4/5 (.gp4, .gp5) files
//! - MIDI event generation from parsed songs
//! - MIDI sequencing utilities
//!
//! # Example
//!
//! ```no_run
//! use ruxguitar::{parse_gp_data, MidiBuilder};
//! use std::rc::Rc;
//!
//! let file_data = std::fs::read("song.gp5").unwrap();
//! let song = parse_gp_data(&file_data).unwrap();
//! let song_rc = Rc::new(song);
//! let midi_events = MidiBuilder::new().build_for_song(&song_rc);
//! ```

pub mod audio;
pub mod error;
pub mod parser;

// Re-export main types for convenience
pub use audio::{
    midi_builder::MidiBuilder,
    midi_event::{MidiEvent, MidiEventType},
    midi_sequencer::MidiSequencer,
    FIRST_TICK,
};
pub use error::RuxError;
pub use parser::song_parser::{
    parse_gp_data, Beat, BeatEffects, Duration, Measure, MeasureHeader, MidiChannel, Note,
    NoteEffect, Song, Tempo, TimeSignature, Track, QUARTER_TIME,
};
