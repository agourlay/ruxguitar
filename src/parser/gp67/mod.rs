mod archive;
mod bit_reader;
// Faithful in-memory model of the GPIF schema; not every field is consumed by
// the player (e.g. clef, automation flags), so some remain read-only.
#[allow(dead_code)]
pub mod document;
pub mod document_reader;
pub mod file_system;
pub mod song_builder;
