//! GP7 (`.gp`) container reader.
//!
//! A `.gp` file is a plain ZIP archive (unlike the BCFZ/BCFS container used by
//! `.gpx`); the score lives at `Content/score.gpif`. Port of Tuxguitar's
//! `v7/GPXFileSystem`.

use crate::RuxError;
use std::io::{Cursor, Read};

const RESOURCE_SCORE: &str = "Content/score.gpif";

/// `score.gpif` documents are a few MB at most; cap reads so a lying ZIP
/// header cannot pre-allocate or inflate without bound (deflate bomb).
const MAX_SCORE_SIZE: u64 = 64 * 1024 * 1024;

/// Extract the raw `score.gpif` XML bytes from a GP7 archive.
pub fn read_gp7_gpif(data: &[u8]) -> Result<Vec<u8>, RuxError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| RuxError::ParsingError(format!("invalid GP7 archive: {e}")))?;
    let entry = archive
        .by_name(RESOURCE_SCORE)
        .map_err(|e| RuxError::ParsingError(format!("no {RESOURCE_SCORE} in GP7 archive: {e}")))?;
    let mut bytes = Vec::with_capacity(entry.size().min(MAX_SCORE_SIZE) as usize);
    entry
        .take(MAX_SCORE_SIZE)
        .read_to_end(&mut bytes)
        .map_err(|e| RuxError::ParsingError(format!("failed to read {RESOURCE_SCORE}: {e}")))?;
    if bytes.len() as u64 == MAX_SCORE_SIZE {
        return Err(RuxError::ParsingError(format!(
            "{RESOURCE_SCORE} exceeds the {MAX_SCORE_SIZE} bytes limit"
        )));
    }
    Ok(bytes)
}
