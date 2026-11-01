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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn archive_with(name: &str, content: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn the_score_is_read_from_the_archive() {
        let data = archive_with(RESOURCE_SCORE, b"<GPIF/>");
        assert_eq!(read_gp7_gpif(&data).unwrap(), b"<GPIF/>");
    }

    #[test]
    fn an_archive_without_a_score_is_rejected() {
        let data = archive_with("Content/other.xml", b"<x/>");
        assert!(read_gp7_gpif(&data).is_err());
    }

    #[test]
    fn a_file_that_is_not_an_archive_is_rejected() {
        assert!(read_gp7_gpif(b"not a zip file").is_err());
    }
}
