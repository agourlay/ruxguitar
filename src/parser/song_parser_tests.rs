#[cfg(test)]
use crate::parser::song_parser::{parse_gp_data, Song};
#[cfg(test)]
use crate::RuxError;
#[cfg(test)]
use std::io::Read;

#[cfg(test)]
pub fn parse_gp_file(file_path: &str) -> Result<Song, RuxError> {
    let mut file = std::fs::File::open(file_path)?;
    let mut file_data: Vec<u8> = vec![];
    file.read_to_end(&mut file_data)?;
    parse_gp_data(&file_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::song_parser::{
        BendEffect, BendPoint, Duration, GpVersion, KeySignature, Marker, NoteType, Padding, Point,
        TripletFeel,
    };

    fn init_logger() {
        env_logger::builder()
            .is_test(true)
            .try_init()
            .unwrap_or_default();
    }

    fn parse_all_files_successfully(with_extension: &str) {
        init_logger();
        let test_dir = std::path::Path::new("test-files");
        for entry in std::fs::read_dir(test_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            if path.extension().unwrap() != with_extension {
                continue;
            }
            let file_name = path.file_name().unwrap().to_str().unwrap();
            eprintln!("Parsing file: {file_name}");
            let file_path = path.to_str().unwrap();
            let song = parse_gp_file(file_path)
                .unwrap_or_else(|err| panic!("Failed to parse file: {file_name}\n{err}"));
            // no empty tracks
            assert!(!song.tracks.is_empty(), "File: {file_name}");
            // assert global invariant across all measures
            for (t_id, t) in song.tracks.iter().enumerate() {
                assert_eq!(
                    t.measures.len(),
                    song.measure_headers.len(),
                    "Track:{t_id} File:{file_name}"
                );
                for (m_id, m) in t.measures.iter().enumerate() {
                    assert_eq!(
                        m.track_index, t_id,
                        "Track:{t_id} Measure:{m_id} File:{file_name}"
                    );
                    assert_eq!(
                        m.header_index, m_id,
                        "Track:{t_id} Measure:{m_id} File:{file_name}"
                    );
                    let voice_count = if with_extension == "gp4" { 1 } else { 2 };
                    assert_eq!(
                        m.voices.len(),
                        voice_count,
                        "Track:{t_id} Measure:{m_id} File:{file_name}"
                    );
                    let measure_header = &song.measure_headers[m_id];
                    let measure_start = measure_header.start;
                    for v in &m.voices {
                        v.beats.iter().enumerate().for_each(|(i, b)| {
                            assert!(
                                b.start >= measure_start,
                                "track:{t_id} measure:{m_id} beat:{i} file:{file_name}"
                            );
                        });
                    }
                }
            }
        }
    }

    #[test]
    fn parse_all_gp5_files_successfully() {
        parse_all_files_successfully("gp5");
    }

    #[test]
    fn parse_all_gp4_files_successfully() {
        parse_all_files_successfully("gp4");
    }

    #[test]
    fn parse_gp4_06_canon_rock() {
        init_logger();
        const FILE_PATH: &str = "test-files/canon_rock.gp4";
        let song = parse_gp_file(FILE_PATH).unwrap();
        assert_eq!(song.version, GpVersion::GP4_06);
        assert_eq!(song.tempo.value, 90);
        assert_eq!(song.tracks.len(), 1);
        assert_eq!(song.tracks[0].name, "\u{ad}µ\u{ad}y 1");
        assert_eq!(song.tracks[0].number, 1);
        assert_eq!(song.tracks[0].offset, 0);
        assert_eq!(song.tracks[0].channel_id, 0);

        // inspect headers
        assert_eq!(song.measure_headers.len(), 220);
        assert_eq!(song.tracks[0].measures.len(), 220);
    }

    #[test]
    fn parse_gp5_00_demo() {
        init_logger();
        // test file from https://github.com/slundi/guitarpro/tree/master/test
        const FILE_PATH: &str = "test-files/Demo v5.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        assert_eq!(song.version, GpVersion::GP5);
        assert_eq!(song.tempo.value, 165);
        assert_eq!(song.tracks.len(), 5);
        assert_eq!(song.tracks[0].name, "Rhythm Guitar");
        assert_eq!(song.tracks[0].number, 1);
        assert_eq!(song.tracks[0].offset, 0);
        assert_eq!(song.tracks[0].channel_id, 0);

        assert_eq!(song.tracks[1].name, "Solo Guitar");
        assert_eq!(song.tracks[1].number, 2);
        assert_eq!(song.tracks[1].offset, 0);
        assert_eq!(song.tracks[1].channel_id, 2);

        assert_eq!(song.tracks[2].name, "Melody");
        assert_eq!(song.tracks[2].number, 3);
        assert_eq!(song.tracks[2].offset, 0);
        assert_eq!(song.tracks[2].channel_id, 6);

        assert_eq!(song.tracks[3].name, "Bass");
        assert_eq!(song.tracks[3].number, 4);
        assert_eq!(song.tracks[3].offset, 0);
        assert_eq!(song.tracks[3].channel_id, 4);

        assert_eq!(song.tracks[4].name, "Percussions");
        assert_eq!(song.tracks[4].number, 5);
        assert_eq!(song.tracks[4].offset, 0);
        assert_eq!(song.tracks[4].channel_id, 9);

        // inspect headers
        assert_eq!(song.measure_headers.len(), 49);
        assert_eq!(song.tracks[0].measures.len(), 49);

        let header = &song.measure_headers[0];
        assert_eq!(header.start, 960);
        assert_eq!(header.tempo.value, 165);
        assert_eq!(header.time_signature.numerator, 4);
        assert_eq!(
            header.time_signature.denominator,
            Duration {
                value: 4,
                dotted: false,
                double_dotted: false,
                tuplet_enters: 1,
                tuplet_times: 1,
            }
        );
        assert_eq!(header.time_signature.denominator.time(), 960);
        // In a 4/4 time signature, the total measure duration is the equivalent of 4 quarter notes.
        // In this case, the duration of a quarter note is 960 ticks.
        // Therefore, the total measure duration is 3840 ticks.
        // 4*960 = 3840
        assert_eq!(header.length(), 3840);
        assert_eq!(
            header.marker,
            Some(Marker {
                title: "Intro".to_string(),
                color: 16_711_680
            })
        );
        assert!(header.repeat_open);
        assert_eq!(header.repeat_close, 0);
        assert_eq!(header.triplet_feel, TripletFeel::None);

        let header = &song.measure_headers[1];
        // 3840 + 960 (offset of previous measure) = 4800
        assert_eq!(header.start, 4800);
        assert_eq!(header.tempo.value, 165);
        assert_eq!(header.time_signature.numerator, 4);
        assert_eq!(header.length(), 3840);
        assert_eq!(header.marker, None);
        assert!(!header.repeat_open);
        assert_eq!(header.repeat_close, 0);
        assert_eq!(header.triplet_feel, TripletFeel::None);

        let header = &song.measure_headers[2];
        assert_eq!(header.start, 8640);
        assert_eq!(header.tempo.value, 165);
        assert_eq!(header.time_signature.numerator, 4);
        assert_eq!(header.length(), 3840);
        assert_eq!(header.marker, None);
        assert!(!header.repeat_open);
        assert_eq!(header.repeat_close, 0);
        assert_eq!(header.triplet_feel, TripletFeel::None);

        let header = &song.measure_headers[3];
        assert_eq!(header.start, 12480);
        assert_eq!(header.tempo.value, 165);
        assert_eq!(header.time_signature.numerator, 4);
        assert_eq!(header.length(), 3840);
        assert_eq!(header.marker, None);
        assert!(!header.repeat_open);
        assert_eq!(header.repeat_close, 1);
        assert_eq!(header.triplet_feel, TripletFeel::None);

        // first measure
        let measure = &song.tracks[0].measures[0];
        assert_eq!(measure.track_index, 0);
        assert_eq!(measure.voices.len(), 2);

        assert_eq!(measure.voices[1].beats.len(), 1);
        assert_eq!(measure.voices[1].beats[0].notes.len(), 0);
        assert_eq!(measure.voices[1].beats[0].start, 960);
        assert!(measure.voices[1].beats[0].empty);

        assert_eq!(measure.voices[0].beats.len(), 8);

        assert_eq!(measure.voices[0].beats[0].start, 960);
        // if there are 8 beats per measure, then each beat is an eighth note long (quarter note / 2 == 960/2).
        assert_eq!(measure.voices[0].beats[0].duration.time(), 480);
        assert!(!measure.voices[0].beats[0].empty);
        assert_eq!(measure.voices[0].beats[0].notes.len(), 3); // C5 chord

        assert_eq!(measure.voices[0].beats[1].start, 1440);
        assert_eq!(measure.voices[0].beats[1].duration.time(), 480);
        assert!(!measure.voices[0].beats[1].empty);
        assert_eq!(measure.voices[0].beats[1].notes.len(), 1); // E2 single

        assert_eq!(measure.voices[0].beats[2].start, 1920);
        assert_eq!(measure.voices[0].beats[2].duration.time(), 480);
        assert!(!measure.voices[0].beats[2].empty);
        assert_eq!(measure.voices[0].beats[2].notes.len(), 1); // E2 single

        assert_eq!(measure.voices[0].beats[3].start, 2400);
        assert_eq!(measure.voices[0].beats[3].duration.time(), 480);
        assert!(!measure.voices[0].beats[3].empty);
        assert_eq!(measure.voices[0].beats[3].notes.len(), 3); // C5 chord

        assert_eq!(measure.voices[0].beats[4].start, 2880);
        assert_eq!(measure.voices[0].beats[4].duration.time(), 480);
        assert!(!measure.voices[0].beats[4].empty);
        assert_eq!(measure.voices[0].beats[4].notes.len(), 1); // E2 single

        assert_eq!(measure.voices[0].beats[5].start, 3360);
        assert_eq!(measure.voices[0].beats[5].duration.time(), 480);
        assert!(!measure.voices[0].beats[5].empty);
        assert_eq!(measure.voices[0].beats[5].notes.len(), 1); // E2 single

        assert_eq!(measure.voices[0].beats[6].start, 3840);
        assert_eq!(measure.voices[0].beats[6].duration.time(), 480);
        assert!(!measure.voices[0].beats[6].empty);
        assert_eq!(measure.voices[0].beats[6].notes.len(), 3); // C5 chord

        assert_eq!(measure.voices[0].beats[7].start, 4320);
        assert_eq!(measure.voices[0].beats[7].duration.time(), 480);
        assert!(!measure.voices[0].beats[7].empty);
        assert_eq!(measure.voices[0].beats[7].notes.len(), 1); // E2 single

        // second measure
        let measure = &song.tracks[0].measures[1];
        assert_eq!(measure.track_index, 0);
        assert_eq!(measure.voices.len(), 2);

        assert_eq!(measure.voices[1].beats.len(), 1);
        assert_eq!(measure.voices[1].beats[0].notes.len(), 0);
        assert_eq!(measure.voices[1].beats[0].start, 4800);
        assert!(measure.voices[1].beats[0].empty);

        assert_eq!(measure.voices[0].beats.len(), 8);

        assert_eq!(measure.voices[0].beats[0].start, 4800);
        assert_eq!(measure.voices[0].beats[0].duration.time(), 480);
        assert!(!measure.voices[0].beats[0].empty);
        assert_eq!(measure.voices[0].beats[0].notes.len(), 3); // C5 chord

        assert_eq!(measure.voices[0].beats[1].start, 5280);
        assert_eq!(measure.voices[0].beats[1].duration.time(), 480);
        assert!(!measure.voices[0].beats[1].empty);
        assert_eq!(measure.voices[0].beats[1].notes.len(), 1); // E2 single

        // inspect midi channels
        assert_eq!(song.midi_channels.len(), 64);
        assert_eq!(song.midi_channels[0].channel_id, 0);
        assert_eq!(song.midi_channels[0].effect_channel_id, 1);
        assert_eq!(song.midi_channels[0].instrument, 29);

        assert_eq!(song.midi_channels[1].channel_id, 1);
        assert_eq!(song.midi_channels[1].effect_channel_id, 0);
        assert_eq!(song.midi_channels[1].instrument, 29);

        assert_eq!(song.midi_channels[2].channel_id, 2);
        assert_eq!(song.midi_channels[2].effect_channel_id, 3);
        assert_eq!(song.midi_channels[2].instrument, 30);

        assert_eq!(song.midi_channels[3].channel_id, 3);
        assert_eq!(song.midi_channels[3].effect_channel_id, 0);
        assert_eq!(song.midi_channels[3].instrument, 30);
    }

    #[test]
    fn parse_gp5_10_bleed() {
        init_logger();
        const FILE_PATH: &str = "test-files/Meshuggah - Bleed.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        assert_eq!(song.version, GpVersion::GP5_10);
        assert_eq!(song.song_info.name, "Bleed");

        assert_eq!(song.tracks.len(), 7);
        assert_eq!(song.tracks[0].name, "Fredrik Thordendal (Guitar 1)");
        assert_eq!(song.tracks[1].name, "Marten Hagstrom (Guitar 2)");
        assert_eq!(song.tracks[2].name, "Dick Lovgren (Bass)");
        assert_eq!(song.tracks[3].name, "Tomas Haake (Drums)");
        assert_eq!(song.tracks[4].name, "Fredrik Thodendal (Solo/Atmospheric)");
        assert_eq!(song.tracks[5].name, "(bass tone)");
        assert_eq!(song.tracks[6].name, "(more atmosphere)");

        // inspect headers
        assert_eq!(song.measure_headers.len(), 209);
        assert_eq!(song.tracks[0].measures.len(), 209);

        let header = &song.measure_headers[0];
        assert_eq!(header.start, 960);
        assert_eq!(header.tempo.value, 115);
        assert_eq!(header.time_signature.numerator, 4);
        assert_eq!(
            header.time_signature.denominator,
            Duration {
                value: 4,
                dotted: false,
                double_dotted: false,
                tuplet_enters: 1,
                tuplet_times: 1,
            }
        );
        assert_eq!(header.time_signature.denominator.time(), 960);
        // In a 4/4 time signature, the total measure duration is the equivalent of 4 quarter notes.
        // In this case, the duration of a quarter note is 960 ticks.
        // Therefore, the total measure duration is 3840 ticks.
        // 4*960 = 3840
        assert_eq!(header.length(), 3840);
        assert_eq!(
            header.marker,
            Some(Marker {
                title: "BLEED".to_string(),
                color: 0
            })
        );
        assert!(!header.repeat_open);
        assert_eq!(header.repeat_close, 0);
        assert_eq!(header.triplet_feel, TripletFeel::None);

        let header = &song.measure_headers[1];
        // 3840 + 960 (offset of previous measure) = 4800
        assert_eq!(header.start, 4800);
        assert_eq!(header.tempo.value, 115);
        assert_eq!(header.time_signature.numerator, 4);
        assert_eq!(header.length(), 3840);
        assert_eq!(header.marker, None);
        assert!(!header.repeat_open);
        assert_eq!(header.repeat_close, 0);
        assert_eq!(header.triplet_feel, TripletFeel::None);

        let header = &song.measure_headers[2];
        assert_eq!(header.start, 8640);
        assert_eq!(header.tempo.value, 115);
        assert_eq!(header.time_signature.numerator, 4);
        assert_eq!(header.length(), 3840);
        assert_eq!(header.marker, None);
        assert!(!header.repeat_open);
        assert_eq!(header.repeat_close, 0);
        assert_eq!(header.triplet_feel, TripletFeel::None);

        // second measure with the low bends
        let measure = &song.tracks[0].measures[1];
        assert_eq!(measure.track_index, 0);
        assert_eq!(measure.voices.len(), 2);

        assert_eq!(measure.voices[1].beats.len(), 1);
        assert_eq!(measure.voices[1].beats[0].notes.len(), 0);
        assert_eq!(measure.voices[1].beats[0].start, 4800);
        assert!(measure.voices[1].beats[0].empty);

        assert_eq!(measure.voices[0].beats.len(), 21);

        assert_eq!(measure.voices[0].beats[0].start, 4800);
        assert_eq!(measure.voices[0].beats[0].duration.time(), 240);
        assert!(!measure.voices[0].beats[0].empty);
        assert_eq!(measure.voices[0].beats[0].notes.len(), 1);

        assert_eq!(measure.voices[0].beats[1].start, 5040);
        assert_eq!(measure.voices[0].beats[1].duration.time(), 240);
        assert!(!measure.voices[0].beats[1].empty);
        assert_eq!(measure.voices[0].beats[1].notes.len(), 1);

        assert_eq!(measure.voices[0].beats[2].start, 5280);
        assert_eq!(measure.voices[0].beats[2].duration.time(), 120);
        assert!(!measure.voices[0].beats[2].empty);
        assert_eq!(measure.voices[0].beats[2].notes.len(), 1);

        assert_eq!(measure.voices[0].beats[3].start, 5400);
        assert_eq!(measure.voices[0].beats[3].duration.time(), 120);
        assert!(!measure.voices[0].beats[3].empty);
        assert_eq!(measure.voices[0].beats[3].notes.len(), 1);

        assert_eq!(measure.voices[0].beats[4].start, 5520);
        assert_eq!(measure.voices[0].beats[4].duration.time(), 240);
        assert_eq!(
            measure.voices[0].beats[4].duration,
            Duration {
                value: 16,
                dotted: false,
                double_dotted: false,
                tuplet_enters: 1,
                tuplet_times: 1
            }
        );
        assert!(!measure.voices[0].beats[4].empty);
        assert_eq!(measure.voices[0].beats[4].notes.len(), 1);
        let note = &measure.voices[0].beats[4].notes[0];
        assert_eq!(note.value, 5);
        assert_eq!(note.string, 6);
        assert_eq!(note.effect.bend, None);
        assert_eq!(note.velocity, 95);
        assert_eq!(note.kind, NoteType::Normal);
        assert!(note.effect.palm_mute);

        assert_eq!(measure.voices[0].beats[5].start, 5760);
        assert_eq!(measure.voices[0].beats[5].duration.time(), 240);
        assert_eq!(
            measure.voices[0].beats[5].duration,
            Duration {
                value: 16,
                dotted: false,
                double_dotted: false,
                tuplet_enters: 1,
                tuplet_times: 1
            }
        );
        assert!(!measure.voices[0].beats[5].empty);
        assert_eq!(measure.voices[0].beats[5].notes.len(), 1);
        let note = &measure.voices[0].beats[5].notes[0];
        assert_eq!(note.value, 5);
        assert_eq!(note.string, 6);
        assert_eq!(note.velocity, 95);
        assert_eq!(note.kind, NoteType::Normal);
        assert!(note.effect.palm_mute);
        assert_eq!(
            note.effect.bend,
            Some(BendEffect {
                points: vec![
                    BendPoint {
                        position: 0,
                        value: 0
                    },
                    BendPoint {
                        position: 12,
                        value: 1
                    }
                ]
            })
        );

        assert_eq!(measure.voices[0].beats[6].start, 6000);
        assert_eq!(measure.voices[0].beats[6].duration.time(), 120);
        assert_eq!(
            measure.voices[0].beats[6].duration,
            Duration {
                value: 32,
                dotted: false,
                double_dotted: false,
                tuplet_enters: 1,
                tuplet_times: 1
            }
        );
        assert!(!measure.voices[0].beats[6].empty);
        assert_eq!(measure.voices[0].beats[6].notes.len(), 1);
        let note = &measure.voices[0].beats[6].notes[0];
        assert_eq!(note.value, 5);
        assert_eq!(note.string, 6);
        assert_eq!(note.velocity, 95);
        assert_eq!(note.kind, NoteType::Normal);
        assert!(note.effect.palm_mute);
        assert_eq!(
            note.effect.bend,
            Some(BendEffect {
                points: vec![
                    BendPoint {
                        position: 0,
                        value: 1
                    },
                    BendPoint {
                        position: 12,
                        value: 1
                    }
                ]
            })
        );

        assert_eq!(measure.voices[0].beats[7].start, 6120);
        assert_eq!(measure.voices[0].beats[7].duration.time(), 120);
        assert!(!measure.voices[0].beats[7].empty);
        assert_eq!(measure.voices[0].beats[7].notes.len(), 1);
        let note = &measure.voices[0].beats[7].notes[0];
        assert_eq!(note.value, 5);
        assert_eq!(note.string, 6);
        assert_eq!(note.velocity, 95);
        assert_eq!(note.kind, NoteType::Normal);
        assert!(note.effect.palm_mute);
        assert_eq!(
            note.effect.bend,
            Some(BendEffect {
                points: vec![
                    BendPoint {
                        position: 0,
                        value: 1
                    },
                    BendPoint {
                        position: 3,
                        value: 1
                    },
                    BendPoint {
                        position: 12,
                        value: 1
                    }
                ]
            })
        );

        assert_eq!(measure.voices[0].beats[8].start, 6240);
        assert_eq!(measure.voices[0].beats[8].duration.time(), 240);
        assert!(!measure.voices[0].beats[8].empty);
        assert_eq!(measure.voices[0].beats[8].notes.len(), 1);
        let note = &measure.voices[0].beats[8].notes[0];
        assert_eq!(note.value, 5);
        assert_eq!(note.string, 6);
        assert_eq!(note.velocity, 95);
        assert_eq!(note.kind, NoteType::Normal);
        assert!(note.effect.palm_mute);
        assert_eq!(
            note.effect.bend,
            Some(BendEffect {
                points: vec![
                    BendPoint {
                        position: 0,
                        value: 1
                    },
                    BendPoint {
                        position: 12,
                        value: 1
                    }
                ]
            })
        );

        assert_eq!(measure.voices[0].beats[9].start, 6480);
        assert_eq!(measure.voices[0].beats[9].duration.time(), 240);
        assert!(!measure.voices[0].beats[9].empty);
        assert_eq!(measure.voices[0].beats[9].notes.len(), 1);
        let note = &measure.voices[0].beats[9].notes[0];
        assert_eq!(note.value, 5);
        assert_eq!(note.string, 6);
        assert_eq!(note.velocity, 95);
        assert_eq!(note.kind, NoteType::Normal);
        assert!(note.effect.palm_mute);
        assert_eq!(
            note.effect.bend,
            Some(BendEffect {
                points: vec![
                    BendPoint {
                        position: 0,
                        value: 1
                    },
                    BendPoint {
                        position: 3,
                        value: 1
                    },
                    BendPoint {
                        position: 12,
                        value: 1
                    }
                ]
            })
        );

        assert_eq!(measure.voices[0].beats[10].start, 6720);
        assert_eq!(measure.voices[0].beats[10].start, 6720);
        assert_eq!(measure.voices[0].beats[10].duration.time(), 120);
        assert!(!measure.voices[0].beats[10].empty);
        assert_eq!(measure.voices[0].beats[10].notes.len(), 1);
        let note = &measure.voices[0].beats[10].notes[0];
        assert_eq!(note.value, 5);
        assert_eq!(note.string, 6);
        assert_eq!(note.velocity, 95);
        assert_eq!(note.kind, NoteType::Normal);
        assert!(note.effect.palm_mute);
        assert_eq!(
            note.effect.bend,
            Some(BendEffect {
                points: vec![
                    BendPoint {
                        position: 0,
                        value: 1
                    },
                    BendPoint {
                        position: 3,
                        value: 1
                    },
                    BendPoint {
                        position: 12,
                        value: 1
                    }
                ]
            })
        );
    }

    #[test]
    fn parse_gp5_10_ghost() {
        init_logger();
        const FILE_PATH: &str = "test-files/Ghost - Cirice.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        assert_eq!(song.version, GpVersion::GP5_10);
        assert_eq!(song.song_info.name, "Cirice");
        assert_eq!(song.song_info.subtitle, "");
        assert_eq!(song.song_info.artist, "Ghost");
        assert_eq!(song.song_info.album, "Meliora");
        assert_eq!(song.song_info.author, "A Ghoul Writer");
        assert_eq!(song.song_info.words, Some("A Ghoul Writer".to_string()));
        assert_eq!(song.song_info.copyright, "");
        assert_eq!(song.song_info.writer, "TheManPF");
        assert_eq!(song.song_info.instructions, "");
        assert!(song.song_info.notices.is_empty());
        assert_eq!(song.triplet_feel, None);
        assert!(song.lyrics.is_some());
        let lyrics = song.lyrics.unwrap();
        assert_eq!(lyrics.track_choice, 0);
        assert_eq!(lyrics.lines.len(), 5);
        assert_eq!(lyrics.lines[0].1, "I feel your presence amongst us\r\nYou cannot hide in the darkness\r\nCan you hear the rumble?\r\nCan you hear the rumble that's calling?\r\n\r\nI know your soul is not tainted\r\nEven though you've been told so\r\nCan you hear the rumble?\r\nCan you hear the rumble that's calling?\r\n\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you\r\n\r\nA candle casting a faint glow\r\nYou and I see eye to eye\r\nCan you hear the thunder?\r\nOh can you hear the thunder that's breaking?\r\n\r\nNow there is nothing between us\r\nFor now our merge is eternal\r\nCan't you see that you're lost?\r\nCan't you see that you're lost without me?\r\n\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you\r\n\r\nCan't you see that you're lost without me?\r\n\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you\r\n\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you\r\nI can feel the thunder that's breaking in your heart\r\nI can see through the scars inside you");
        assert!(song.page_setup.is_some());
        let page_setup = song.page_setup.unwrap();
        assert_eq!(page_setup.page_size, Point { x: 216, y: 279 });
        assert_eq!(
            page_setup.page_margin,
            Padding {
                right: 10,
                top: 10,
                left: 15,
                bottom: 10
            }
        );

        assert_eq!(page_setup.score_size_proportion, 1.0);
        assert_eq!(page_setup.header_and_footer, 511);
        assert_eq!(page_setup.title, "\u{7}%TITLE%");
        assert_eq!(page_setup.subtitle, "\n%SUBTITLE%");
        assert_eq!(page_setup.artist, "\u{8}%ARTIST%");
        assert_eq!(page_setup.album, "\u{7}%ALBUM%");
        assert_eq!(page_setup.words, "\u{10}Words by %WORDS%");
        assert_eq!(page_setup.music, "\u{10}Music by %MUSIC%");
        assert_eq!(
            page_setup.word_and_music,
            "\u{1d}Words & Music by %WORDSMUSIC%"
        );
        assert_eq!(
            page_setup.copyright,
            "\u{15}Copyright %COPYRIGHT%\n5All Rights Reserved - International Copyright Secured"
        );
        assert_eq!(page_setup.page_number, "\u{c}Page %N%/%P%");

        assert_eq!(song.tempo.name, Some("\u{8}Moderate".to_string()));
        assert_eq!(song.tempo.value, 90);
        assert_eq!(song.hide_tempo, Some(false));
        assert_eq!(song.key_signature, 0);
        assert_eq!(song.octave, Some(0));

        assert_eq!(song.midi_channels.len(), 64);
        assert_eq!(song.midi_channels[0].effect_channel_id, 0);
        assert_eq!(song.midi_channels[0].instrument, 25);
        assert_eq!(song.midi_channels[0].volume, 16);
        assert_eq!(song.midi_channels[0].balance, 0);
        assert_eq!(song.midi_channels[0].chorus, 0);
        assert_eq!(song.midi_channels[0].reverb, 0);
        assert_eq!(song.midi_channels[0].phaser, 0);
        assert_eq!(song.midi_channels[0].tremolo, 0);
        assert_eq!(song.midi_channels[0].bank, 0);

        assert_eq!(song.tracks.len(), 14);
        assert_eq!(song.tracks[0].name, "Vocals");
        assert_eq!(song.tracks[1].name, "Acoustic Guitar L");
        assert_eq!(song.tracks[2].name, "Acoustic Guitar R");
        assert_eq!(song.tracks[3].name, "Rythm Guitar L");
        assert_eq!(song.tracks[4].name, "Rythm Guitar R");
        assert_eq!(song.tracks[5].name, "Lead Guitar");
        assert_eq!(song.tracks[6].name, "Bass");
        assert_eq!(song.tracks[7].name, "Piano");
        assert_eq!(song.tracks[8].name, "Organ");
        assert_eq!(song.tracks[9].name, "Synth");
        assert_eq!(song.tracks[10].name, "Strings");
        assert_eq!(song.tracks[11].name, "Drums");
        assert_eq!(song.tracks[12].name, "Timpani");
        assert_eq!(song.tracks[13].name, "Reverse");

        for t in &song.tracks {
            assert_eq!(t.measures.len(), song.measure_headers.len());
        }

        let guitar_track = &song.tracks[2];
        assert_eq!(guitar_track.name, "Acoustic Guitar R");
        assert_eq!(guitar_track.strings.len(), 6);
        assert_eq!(guitar_track.strings[0].1, 62);
        assert_eq!(guitar_track.strings[1].1, 57);
        assert_eq!(guitar_track.strings[2].1, 53);
        assert_eq!(guitar_track.strings[3].1, 48);
        assert_eq!(guitar_track.strings[4].1, 43);
        assert_eq!(guitar_track.strings[5].1, 38);

        assert_eq!(guitar_track.measures.len(), 124);
        let measure = &guitar_track.measures[0];
        assert_eq!(measure.time_signature.numerator, 4);
        assert_eq!(
            measure.time_signature.denominator,
            Duration {
                value: 4,
                dotted: false,
                double_dotted: false,
                tuplet_enters: 1,
                tuplet_times: 1
            }
        );
        assert_eq!(measure.key_signature, KeySignature::new(0, false));

        assert_eq!(measure.voices.len(), 2);
        for v in &measure.voices {
            assert_eq!(v.beats.len(), 1);
            assert_eq!(v.measure_index, 0);
            assert!(v.beats[0].notes.is_empty());
            assert!(v.beats[0].empty);
        }
        let beat = &measure.voices[1].beats[0];
        assert_eq!(beat.start, 960);
        assert_eq!(beat.text, "");
        assert_eq!(
            beat.duration,
            Duration {
                value: 4,
                dotted: false,
                double_dotted: false,
                tuplet_enters: 1,
                tuplet_times: 1
            }
        );
        assert_eq!(beat.notes.len(), 0);
    }
}
