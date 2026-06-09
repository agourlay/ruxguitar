//! Top-level parsing entry point: dispatch a Guitar Pro file to the right
//! format parser based on its container magic.

use crate::RuxError;
use crate::parser::gp67::song_builder::{parse_gp7_data, parse_gpx_data};
use crate::parser::gp345::song_parser::parse_gp345_data;
use crate::parser::model::Song;

/// Parse any supported Guitar Pro file into a [`Song`].
///
/// - `BCFS` / `BCFZ` magic → GP6 (`.gpx`) container.
/// - `PK\x03\x04` (ZIP) magic → GP7 (`.gp`) container.
/// - otherwise → GP3/GP4/GP5 flat binary.
pub fn parse_gp_data(file_data: &[u8]) -> Result<Song, RuxError> {
    if file_data.starts_with(b"BCFS") || file_data.starts_with(b"BCFZ") {
        parse_gpx_data(file_data)
    } else if file_data.starts_with(b"PK\x03\x04") {
        parse_gp7_data(file_data)
    } else {
        parse_gp345_data(file_data)
    }
}
