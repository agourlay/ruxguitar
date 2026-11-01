use crate::RuxError;
use crate::parser::gp345::music_parser::MusicParser;
use crate::parser::gp345::primitive_parser::{
    parse_bool, parse_byte_size_string, parse_i8, parse_int, parse_int_byte_sized_string,
    parse_int_sized_string, parse_short, parse_u8, skip,
};
use nom::IResult;
use nom::Parser;
use nom::bytes::complete::take;
use nom::combinator::{cond, flat_map, map};
use nom::multi::count;
use nom::sequence::preceded;

// GP4 docs at <https://dguitar.sourceforge.net/GP4format.html>
// GP5 docs thanks to Tuxguitar and <https://github.com/slundi/guitarpro> for the help

pub use crate::parser::model::*;

/// Read beat duration.
/// Duration is composed of byte signifying duration and an integer that maps to `Tuplet`. The byte maps to following values:
///
/// * *-2*: whole note
/// * *-1*: half note
/// * *0*: quarter note
/// * *1*: eighth note
/// * *2*: sixteenth note
/// * *3*: thirty-second note
///
/// If flag at *0x20* is true, the tuplet is read
pub fn parse_duration(flags: u8) -> impl FnMut(&[u8]) -> IResult<&[u8], Duration> {
    move |i: &[u8]| {
        log::debug!("Parsing duration");
        let mut i = i;
        let mut d = Duration::default();
        let (inner, value) = parse_i8(i)?;
        i = inner;
        // clamp the exponent so corrupt values can neither overflow nor
        // produce a zero duration value
        let exponent = (i32::from(value) + 4).clamp(2, 16) as u32;
        d.value = (2_u32.pow(exponent) / 4) as u16;
        log::debug!("Duration value: {}", d.value);
        d.dotted = flags & 0x01 != 0;

        if (flags & 0x20) != 0 {
            let (inner, i_tuplet) = parse_int(i)?;
            i = inner;

            match i_tuplet {
                3 => {
                    d.tuplet_enters = i_tuplet as u8;
                    d.tuplet_times = 2;
                }
                5..=7 => {
                    d.tuplet_enters = i_tuplet as u8;
                    d.tuplet_times = 4;
                }
                9..=13 => {
                    d.tuplet_enters = i_tuplet as u8;
                    d.tuplet_times = 8;
                }
                x => log::debug!("Unknown tuplet: {x}"),
            }
        }

        Ok((i, d))
    }
}

pub fn parse_color(i: &[u8]) -> IResult<&[u8], i32> {
    log::debug!("Parsing RGB color");
    map(
        (parse_u8, parse_u8, parse_u8, parse_u8),
        |(r, g, b, _alpha)| (i32::from(r) << 16) | (i32::from(g) << 8) | i32::from(b),
    )
    .parse(i)
}

pub fn parse_marker(i: &[u8]) -> IResult<&[u8], Marker> {
    log::debug!("Parsing marker");
    map(
        (parse_int_byte_sized_string, parse_color),
        |(title, color)| Marker { title, color },
    )
    .parse(i)
}

pub fn parse_triplet_feel(i: &[u8]) -> IResult<&[u8], TripletFeel> {
    log::debug!("Parsing triplet feel");
    map(parse_i8, |triplet_feel| match triplet_feel {
        1 => TripletFeel::Eighth,
        2 => TripletFeel::Sixteenth,
        // like TuxGuitar, unknown values fall back to no triplet feel
        _ => TripletFeel::None,
    })
    .parse(i)
}

/// Parse measure header.
/// the time and key signatures are propagated to the next measure
pub fn parse_measure_header(
    previous_time_signature: TimeSignature,
    previous_key_signature: KeySignature,
    song_tempo: u32,
    song_version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], MeasureHeader> {
    move |i: &[u8]| {
        log::debug!("Parsing measure header");
        let (mut i, flags) = parse_u8(i)?;
        log::debug!("Flags: {flags:08b}");
        let mut mh = MeasureHeader::default();
        mh.tempo.value = song_tempo; // value updated later when parsing beats
        mh.repeat_open = (flags & 0x04) != 0;
        // propagate key signature, overridden by a 0x40 tonality change below
        mh.key_signature = previous_key_signature;
        // propagate time signature
        mh.time_signature = previous_time_signature.clone();

        // Numerator of the (key) signature
        if (flags & 0x01) != 0 {
            log::debug!("Parsing numerator");
            let (inner, numerator) = parse_i8(i)?;
            i = inner;
            mh.time_signature.numerator = numerator as u8;
        }

        // Denominator of the (key) signature
        if (flags & 0x02) != 0 {
            log::debug!("Parsing denominator");
            let (inner, denominator_value) = parse_i8(i)?;
            i = inner;
            let denominator = Duration {
                value: denominator_value as u16,
                ..Default::default()
            };
            mh.time_signature.denominator = denominator;
        }

        if song_version >= GpVersion::GP5 {
            // Beginning of repeat
            if (flags & 0x08) != 0 {
                log::debug!("Parsing repeat close");
                let (inner, repeat_close) = parse_i8(i)?;
                i = inner;
                mh.repeat_close = repeat_close - 1; // GP5 specific logic
            }

            // Presence of a marker
            if (flags & 0x20) != 0 {
                let (inner, marker) = parse_marker(i)?;
                i = inner;
                mh.marker = Some(marker);
            }

            // Tonality of the measure
            if (flags & 0x40) != 0 {
                log::debug!("Parsing key signature");
                let (inner, key_signature) = parse_i8(i)?;
                mh.key_signature.key = key_signature;
                i = inner;
                let (inner, is_minor) = parse_i8(i)?;
                i = inner;
                mh.key_signature.is_minor = is_minor != 0;
            }

            if (flags & 0x01) != 0 || (flags & 0x02) != 0 {
                log::debug!("Skip 4");
                i = skip(i, 4);
            }

            // Number of alternate ending
            if (flags & 0x10) != 0 {
                log::debug!("Parsing repeat alternative");
                let (inner, alternative) = parse_u8(i)?;
                i = inner;
                mh.repeat_alternative = alternative;
            }

            if (flags & 0x10) == 0 {
                log::debug!("Skip one");
                i = skip(i, 1);
            }

            // Triplet feel
            let (inner, triplet_feel) = parse_triplet_feel(i)?;
            i = inner;
            mh.triplet_feel = triplet_feel;
        } else if song_version <= GpVersion::GP4_06 {
            // Beginning of repeat
            if (flags & 0x08) != 0 {
                log::debug!("Parsing repeat close");
                let (inner, repeat_close) = parse_i8(i)?;
                i = inner;
                mh.repeat_close = repeat_close;
            }

            // Number of alternate ending
            if (flags & 0x10) != 0 {
                log::debug!("Parsing repeat alternative");
                let (inner, alternative) = parse_u8(i)?;
                i = inner;
                mh.repeat_alternative = alternative;
            }

            // Presence of a marker
            if (flags & 0x20) != 0 {
                let (inner, marker) = parse_marker(i)?;
                i = inner;
                mh.marker = Some(marker);
            }

            // Tonality of the measure
            if (flags & 0x40) != 0 {
                log::debug!("Parsing key signature");
                let (inner, key_signature) = parse_i8(i)?;
                mh.key_signature.key = key_signature;
                i = inner;
                let (inner, is_minor) = parse_i8(i)?;
                i = inner;
                mh.key_signature.is_minor = is_minor != 0;
            }
        }

        log::debug!("{mh:?}");

        Ok((i, mh))
    }
}

/// Convert a GP3/GP4 alternate-ending number into an ending bitmask.
///
/// GP3/GP4 store the volta as a count ("covers endings up to N") while GP5
/// stores a bitmask. Port of TuxGuitar's `parseRepeatAlternative`: set the
/// bits below the count not in `existing_alternatives` (the endings already
/// used by earlier brackets in the same repeat section).
fn convert_repeat_alternative(existing_alternatives: u8, value: u8) -> u8 {
    let mut repeat_alternative = 0_u8;
    for ending in 0..8_u8 {
        if value > ending && existing_alternatives & (1 << ending) == 0 {
            repeat_alternative |= 1 << ending;
        }
    }
    repeat_alternative
}

pub fn parse_measure_headers(
    measure_count: i32,
    song_tempo: u32,
    version: GpVersion,
    song_key: i8,
) -> impl FnMut(&[u8]) -> IResult<&[u8], Vec<MeasureHeader>> {
    move |i: &[u8]| {
        log::debug!("Parsing {measure_count} measure headers");
        // GP3/GP4 store the alternate ending as a count to convert into a bitmask
        let needs_alternative_conversion = version <= GpVersion::GP4_06;
        // endings already used since the last repeat_open
        let mut existing_alternatives = 0_u8;
        let mut convert_alternative = |header: &mut MeasureHeader| {
            if !needs_alternative_conversion {
                return;
            }
            // a measure's own repeat_open does not affect its own conversion
            header.repeat_alternative =
                convert_repeat_alternative(existing_alternatives, header.repeat_alternative);
            if header.repeat_open {
                existing_alternatives = 0;
            }
            existing_alternatives |= header.repeat_alternative;
        };
        // the first measure inherits the song-level key signature
        let song_key_signature = KeySignature::new(song_key, false);
        // parse first header to account for the byte in between each header in GP5
        let (mut i, mut first_header) = parse_measure_header(
            TimeSignature::default(),
            song_key_signature,
            song_tempo,
            version,
        )(i)?;
        convert_alternative(&mut first_header);
        let mut previous_time_signature = first_header.time_signature.clone();
        let mut previous_key_signature = first_header.key_signature;
        let mut headers = vec![first_header];
        for _ in 1..measure_count {
            let (rest, mut header) = preceded(
                cond(version >= GpVersion::GP5, parse_u8),
                parse_measure_header(
                    previous_time_signature,
                    previous_key_signature,
                    song_tempo,
                    version,
                ),
            )
            .parse(i)?;
            convert_alternative(&mut header);
            // propagate time and key signatures
            previous_time_signature = header.time_signature.clone();
            previous_key_signature = header.key_signature;
            i = rest;
            headers.push(header);
        }
        debug_assert_eq!(headers.len(), measure_count as usize);
        Ok((i, headers))
    }
}

pub fn parse_midi_channels(i: &[u8]) -> IResult<&[u8], Vec<MidiChannel>> {
    log::debug!("Parsing midi channels");
    let mut channels = Vec::with_capacity(64);
    let mut i = i;
    for channel_index in 0..64 {
        let (inner, channel) = parse_midi_channel(channel_index)(i)?;
        i = inner;
        channels.push(channel);
    }
    Ok((i, channels))
}

pub fn parse_midi_channel(channel_id: i32) -> impl FnMut(&[u8]) -> IResult<&[u8], MidiChannel> {
    move |i: &[u8]| {
        map(
            (
                parse_int, parse_i8, parse_i8, parse_i8, parse_i8, parse_i8, parse_i8, parse_u8,
                parse_u8,
            ),
            |(
                mut instrument,
                volume,
                balance,
                chorus,
                reverb,
                phaser,
                tremolo,
                _blank,
                _blank2,
            )| {
                let bank = if channel_id == 9 {
                    DEFAULT_PERCUSSION_BANK
                } else {
                    DEFAULT_BANK
                };
                if instrument < 0 {
                    instrument = 0;
                }
                MidiChannel {
                    channel_id: channel_id as u8,
                    effect_channel_id: 0, // filled at the track level
                    instrument,
                    volume,
                    balance,
                    chorus,
                    reverb,
                    phaser,
                    tremolo,
                    bank,
                }
            },
        )
        .parse(i)
    }
}

pub fn parse_page_setup(i: &[u8]) -> IResult<&[u8], PageSetup> {
    log::debug!("Parsing page setup");
    map(
        (
            parse_point,
            parse_padding,
            parse_int,
            parse_short,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
        ),
        |(
            page_size,
            page_margin,
            score_size_proportion,
            header_and_footer,
            title,
            subtitle,
            artist,
            album,
            words,
            music,
            word_and_music,
            copyright_1,
            copyright_2,
            page_number,
        )| PageSetup {
            page_size,
            page_margin,
            score_size_proportion: score_size_proportion as f32 / 100.0,
            header_and_footer,
            title,
            subtitle,
            artist,
            album,
            words,
            music,
            word_and_music,
            copyright: format!("{copyright_1}\n{copyright_2}"),
            page_number,
        },
    )
    .parse(i)
}

pub fn parse_point(i: &[u8]) -> IResult<&[u8], Point> {
    log::debug!("Parsing point");
    map((parse_int, parse_int), |(x, y)| Point { x, y }).parse(i)
}

pub fn parse_padding(i: &[u8]) -> IResult<&[u8], Padding> {
    log::debug!("Parsing padding");
    map(
        (parse_int, parse_int, parse_int, parse_int),
        |(right, top, left, bottom)| Padding {
            right,
            top,
            left,
            bottom,
        },
    )
    .parse(i)
}

pub fn parse_lyrics(i: &[u8]) -> IResult<&[u8], Lyrics> {
    log::debug!("Parsing lyrics");
    map(
        (parse_int, count((parse_int, parse_int_sized_string), 5)),
        |(track_choice, lines)| Lyrics {
            track_choice,
            lines,
        },
    )
    .parse(i)
}

/// Parse the version string from the file header.
///
/// 30 character string (not counting the byte announcing the real length of the string)
///
/// <https://dguitar.sourceforge.net/GP4format.html#VERSIONS>
pub fn parse_gp_version(i: &[u8]) -> IResult<&[u8], GpVersion> {
    log::debug!("Parsing GP version");
    let (rest, version_string) = parse_byte_size_string(30)(i)?;
    let version = match version_string.as_str() {
        "FICHIER GUITAR PRO v3.00" => GpVersion::GP3,
        "FICHIER GUITAR PRO v4.00" => GpVersion::GP4,
        // "L4.06" is written by a common GP4 clone
        "FICHIER GUITAR PRO v4.06" | "FICHIER GUITAR PRO L4.06" => GpVersion::GP4_06,
        "FICHIER GUITAR PRO v5.00" => GpVersion::GP5,
        "FICHIER GUITAR PRO v5.10" => GpVersion::GP5_10,
        _ => {
            log::warn!("Unsupported GP version: {version_string}");
            return Err(nom::Err::Error(nom::error::Error::new(
                i,
                nom::error::ErrorKind::Tag,
            )));
        }
    };
    Ok((rest, version))
}

fn parse_notices(i: &[u8]) -> IResult<&[u8], Vec<String>> {
    flat_map(parse_int, |notice_count| {
        log::debug!("Notice count: {notice_count}");
        count(parse_int_byte_sized_string, notice_count as usize)
    })
    .parse(i)
}

/// Par information about the piece of music.
/// <https://dguitar.sourceforge.net/GP4format.html#Information_About_the_Piece>
fn parse_info(version: GpVersion) -> impl FnMut(&[u8]) -> IResult<&[u8], SongInfo> {
    move |i: &[u8]| {
        log::debug!("Parsing song info");
        map(
            (
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                cond(version >= GpVersion::GP5, parse_int_byte_sized_string),
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_notices,
            ),
            |(
                name,
                subtitle,
                artist,
                album,
                author,
                words,
                copyright,
                writer,
                instructions,
                notices,
            )| {
                SongInfo {
                    name,
                    subtitle,
                    artist,
                    album,
                    author,
                    words,
                    copyright,
                    writer,
                    instructions,
                    notices,
                }
            },
        )
        .parse(i)
    }
}

/// Parse a GP3/GP4/GP5 flat-binary file. Container formats (GP6 `.gpx`, GP7
/// `.gp`) are dispatched separately; see [`crate::parser::parse_gp_data`].
pub fn parse_gp345_data(file_data: &[u8]) -> Result<Song, RuxError> {
    let (rest, base_song) = flat_map(parse_gp_version, |version| {
        map(
            (
                parse_info(version),                                     // Song info
                cond(version < GpVersion::GP5, parse_bool),              // Triplet feel
                cond(version >= GpVersion::GP4, parse_lyrics),           // Lyrics
                cond(version >= GpVersion::GP5_10, take(19usize)),       // Skip RSE master effect
                cond(version >= GpVersion::GP5, parse_page_setup),       // Page setup
                cond(version >= GpVersion::GP5, parse_int_sized_string), // Tempo name
                parse_int,                                               // Tempo value
                cond(version > GpVersion::GP5, parse_bool),              // Tempo hide
                parse_i8,                                                // Key signature
                take(3usize),                                            // unknown
                cond(version > GpVersion::GP3, parse_i8),                // Octave
                parse_midi_channels,                                     // Midi channels
            ),
            move |(
                song_info,
                triplet_feel,
                lyrics,
                _master_effect,
                page_setup,
                tempo_name,
                tempo,
                hide_tempo,
                key_signature,
                _unknown,
                octave,
                midi_channels,
            )| {
                // init base song
                let tempo = Tempo::new(tempo as u32, tempo_name);
                Song {
                    version,
                    song_info,
                    triplet_feel,
                    lyrics,
                    page_setup,
                    tempo,
                    hide_tempo,
                    key_signature,
                    octave,
                    midi_channels,
                    measure_headers: vec![],
                    tracks: vec![],
                }
            },
        )
    })
    .parse(file_data)
    .map_err(|_err| {
        log::error!("Failed to parse GP data");
        RuxError::ParsingError("Failed to parse GP data".to_string())
    })?;

    // make parser and parse music data
    let mut parser = MusicParser::new(base_song);
    let (_rest, ()) = parser.parse_music_data(rest).map_err(|e| {
        log::error!("Failed to parse music data: {e:?}");
        RuxError::ParsingError("Failed to parse music data".to_string())
    })?;
    let mut song = parser.take_song();

    // For GP4 and earlier, triplet feel is defined at the song level.
    // Propagate it to all measure headers so the MIDI builder can apply it.
    if song.version < GpVersion::GP5
        && let Some(true) = song.triplet_feel
    {
        for header in &mut song.measure_headers {
            header.triplet_feel = TripletFeel::Eighth;
        }
    }

    Ok(song)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gp_ordering() {
        assert!(GpVersion::GP4 < GpVersion::GP5);
        assert!(GpVersion::GP5 >= GpVersion::GP5);
        assert!(GpVersion::GP3 < GpVersion::GP4);
        assert!(GpVersion::GP3 < GpVersion::GP5);
    }

    #[test]
    fn test_convert_repeat_alternative() {
        // successive voltas 1, 2, 3 become bits 0, 1, 2
        assert_eq!(convert_repeat_alternative(0b000, 1), 0b001);
        assert_eq!(convert_repeat_alternative(0b001, 2), 0b010);
        assert_eq!(convert_repeat_alternative(0b011, 3), 0b100);
    }

    #[test]
    fn test_convert_repeat_alternative_combined_volta() {
        // a combined "1.2." first volta is stored as 2 and covers both endings
        assert_eq!(convert_repeat_alternative(0b000, 2), 0b011);
    }

    #[test]
    fn test_convert_repeat_alternative_count_beyond_eight() {
        // counts above 8 endings saturate at the 8-bit mask
        assert_eq!(convert_repeat_alternative(0b000, 255), 0b1111_1111);
        assert_eq!(convert_repeat_alternative(0b1111_1111, 255), 0);
    }
}
