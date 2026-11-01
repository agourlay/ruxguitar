use crate::parser::song_parser::{GpVersion, Song};

/// The song details shown in the title and status bars.
#[derive(Debug)]
pub struct SongDisplayInfo {
    pub name: String,
    pub artist: String,
    subtitle: String,
    album: String,
    author: String,
    writer: String,
    copyright: String,
    pub gp_version: GpVersion,
    pub file_name: String,
}

impl SongDisplayInfo {
    pub fn new(song: &Song, file_name: String) -> Self {
        Self {
            name: song.song_info.name.clone(),
            artist: song.song_info.artist.clone(),
            subtitle: song.song_info.subtitle.clone(),
            album: song.song_info.album.clone(),
            author: song.song_info.author.clone(),
            writer: song.song_info.writer.clone(),
            copyright: song.song_info.copyright.clone(),
            gp_version: song.version,
            file_name,
        }
    }

    /// The song name, followed by its artist when known.
    pub fn title_line(&self) -> String {
        if self.artist.is_empty() {
            self.name.clone()
        } else {
            format!("{} - {}", self.name, self.artist)
        }
    }

    /// Metadata fields joined with " \u{2022} ", skipping empty ones.
    /// Returns `None` if no metadata is available.
    pub fn metadata_line(&self) -> Option<String> {
        let parts: Vec<&str> = [
            self.subtitle.as_str(),
            self.album.as_str(),
            self.author.as_str(),
            self.writer.as_str(),
            self.copyright.as_str(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" \u{2022} "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::song_parser::SongInfo;

    fn display_info(name: &str, artist: &str, album: &str, copyright: &str) -> SongDisplayInfo {
        let song = Song {
            song_info: SongInfo {
                name: name.to_string(),
                artist: artist.to_string(),
                album: album.to_string(),
                copyright: copyright.to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        SongDisplayInfo::new(&song, "song.gp5".to_string())
    }

    #[test]
    fn the_title_names_the_artist_when_known() {
        assert_eq!(display_info("Song", "", "", "").title_line(), "Song");
        assert_eq!(
            display_info("Song", "Band", "", "").title_line(),
            "Song - Band"
        );
    }

    #[test]
    fn metadata_skips_empty_fields() {
        assert_eq!(display_info("Song", "Band", "", "").metadata_line(), None);
        assert_eq!(
            display_info("Song", "Band", "Album", "2024").metadata_line(),
            Some("Album \u{2022} 2024".to_string())
        );
    }
}
