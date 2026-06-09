//! GP7 (`.gp`) container reader.
//!
//! A `.gp` file is a plain ZIP archive (unlike the BCFZ/BCFS container used by
//! `.gpx`); the score lives at `Content/score.gpif`. Port of Tuxguitar's
//! `v7/GPXFileSystem`.

use crate::RuxError;
use std::io::{Cursor, Read};

const RESOURCE_SCORE: &str = "Content/score.gpif";

/// Extract the raw `score.gpif` XML bytes from a GP7 archive.
pub fn read_gp7_gpif(data: &[u8]) -> Result<Vec<u8>, RuxError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| RuxError::ParsingError(format!("invalid GP7 archive: {e}")))?;
    let mut entry = archive
        .by_name(RESOURCE_SCORE)
        .map_err(|e| RuxError::ParsingError(format!("no {RESOURCE_SCORE} in GP7 archive: {e}")))?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .map_err(|e| RuxError::ParsingError(format!("failed to read {RESOURCE_SCORE}: {e}")))?;
    Ok(bytes)
}
