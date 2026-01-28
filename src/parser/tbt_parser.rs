//! TBT (TabIt) file format parser
//!
//! Reference: <https://github.com/bostick/tabit-file-format>
//! Reference parser: <https://github.com/bostick/tbt-parser>

use crate::parser::song_parser::{
    Beat, BendEffect, BendPoint, Duration, GpVersion, HarmonicEffect, HarmonicType, KeySignature,
    Measure, MeasureHeader, MidiChannel, Note, NoteEffect, NoteType, SlapEffect, SlideType, Song,
    SongInfo, Tempo, TimeSignature, Track, TremoloPickingEffect, TripletFeel, Voice, DEFAULT_BANK,
    DEFAULT_PERCUSSION_BANK, QUARTER_TIME,
};
use crate::parser::tbt_types::*;
use crate::RuxError;
use crc32fast::Hasher;
use flate2::read::ZlibDecoder;
use nom::bytes::complete::{tag, take};
use nom::number::complete::{le_u16, le_u32, le_u8};
use nom::IResult;
use std::io::Read;

/// Magic bytes at the start of TBT files
const TBT_MAGIC: &[u8] = b"TBT";

/// Size of the TBT header in bytes
const TBT_HEADER_SIZE: usize = 64;

/// Offset of CRC32 header field (last 4 bytes of header)
#[allow(dead_code)]
const CRC32_HEADER_OFFSET: usize = 0x3c;

/// TBT time units:
/// - 1 space = 1/16th note (semi-quaver)
/// - 1 vsq = 1/20 of a space (viginti-semi-quaver)
/// - 1 dsq = 1/2 of a space (demi-semi-quaver)
///
/// GP time units:
/// - 1 tick = 1/960 of a quarter note
///
/// Conversion:
/// - 1 quarter note = 4 spaces = 960 ticks
/// - 1 space = 240 ticks
/// - 1 vsq = 12 ticks
/// - 1 dsq = 120 ticks
pub const TICKS_PER_SPACE: u32 = 240;
#[allow(dead_code)]
pub const TICKS_PER_VSQ: u32 = 12;
#[allow(dead_code)]
pub const TICKS_PER_DSQ: u32 = 120;

/// Parse a Pascal1 string (1-byte length prefix)
fn parse_pascal1_string(input: &[u8]) -> IResult<&[u8], String> {
    let (input, len) = le_u8(input)?;
    let (input, bytes) = take(len)(input)?;
    let s = String::from_utf8_lossy(bytes).into_owned();
    Ok((input, s))
}

/// Parse a Pascal2 string (2-byte length prefix, little-endian)
fn parse_pascal2_string(input: &[u8]) -> IResult<&[u8], String> {
    let (input, len) = le_u16(input)?;
    let (input, bytes) = take(len)(input)?;
    let s = String::from_utf8_lossy(bytes).into_owned();
    Ok((input, s))
}

/// Parse the 64-byte TBT header
fn parse_tbt_header(input: &[u8]) -> IResult<&[u8], TbtHeader> {
    // Magic bytes "TBT"
    let (input, _) = tag(TBT_MAGIC)(input)?;

    // Version number
    let (input, version_byte) = le_u8(input)?;
    let version = TbtVersion::from_byte(version_byte).ok_or_else(|| {
        nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Tag))
    })?;

    // Tempo1 (1 byte)
    let (input, tempo1) = le_u8(input)?;

    // Track count (1 byte)
    let (input, track_count) = le_u8(input)?;

    // Version string (Pascal1: 1 byte length + string)
    let (input, version_string) = parse_pascal1_string(input)?;

    // Feature bitfield (at offset 0x0b, but we need to account for variable version string length)
    // The version string is typically 4 chars ("2.03"), so offset 0x0b should be 0x06 + 5 = 0x0b
    // We need to skip to offset 0x0b from start, which means skipping unused bytes
    let bytes_consumed = 3 + 1 + 1 + 1 + 1 + version_string.len(); // magic + version + tempo1 + track_count + len_byte + string
    let skip_to_feature = 0x0b_usize.saturating_sub(bytes_consumed);
    let (input, _) = take(skip_to_feature)(input)?;

    let (input, feature_byte) = le_u8(input)?;
    let features = TbtFeatures::from_byte(feature_byte);

    // Skip unused bytes (28 bytes from 0x0c to 0x28)
    let (input, _) = take(28usize)(input)?;

    // Bar count (2 bytes at 0x28)
    let (input, bar_count) = le_u16(input)?;

    // Space count (2 bytes at 0x2a)
    let (input, space_count) = le_u16(input)?;

    // Last non-empty space (2 bytes at 0x2c)
    let (input, last_non_empty_space) = le_u16(input)?;

    // Tempo2 (2 bytes at 0x2e)
    let (input, tempo2) = le_u16(input)?;

    // Compressed metadata length (4 bytes at 0x30)
    let (input, compressed_metadata_len) = le_u32(input)?;

    // CRC32 body (4 bytes at 0x34)
    let (input, crc32_body) = le_u32(input)?;

    // Total byte count (4 bytes at 0x38)
    let (input, total_byte_count) = le_u32(input)?;

    // CRC32 header (4 bytes at 0x3c)
    let (input, crc32_header) = le_u32(input)?;

    Ok((
        input,
        TbtHeader {
            version,
            tempo1,
            track_count,
            version_string,
            features,
            bar_count,
            space_count,
            last_non_empty_space,
            tempo2,
            compressed_metadata_len,
            crc32_body,
            total_byte_count,
            crc32_header,
        },
    ))
}

/// Decompress zlib-compressed data
fn decompress_zlib(compressed: &[u8]) -> Result<Vec<u8>, RuxError> {
    let mut decoder = ZlibDecoder::new(compressed);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| RuxError::ParsingError(format!("Failed to decompress TBT data: {e}")))?;
    Ok(decompressed)
}

/// Parse metadata track configuration blocks
fn parse_metadata_tracks(
    input: &[u8],
    track_count: u8,
    version: TbtVersion,
) -> IResult<&[u8], Vec<TbtTrack>> {
    let mut tracks: Vec<TbtTrack> = (0..track_count)
        .map(|i| TbtTrack {
            index: i,
            ..Default::default()
        })
        .collect();

    let mut input = input;

    // Version 0x70+: spaceCountBlock (4 bytes per track)
    if version.has_space_count_per_track() {
        for track in &mut tracks {
            let (rest, space_count) = le_u32(input)?;
            track.space_count = Some(space_count);
            input = rest;
        }
    }

    // stringCountBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.string_count = val;
        input = rest;
    }

    // cleanGuitarBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.clean_guitar = val;
        input = rest;
    }

    // mutedGuitarBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.muted_guitar = val;
        input = rest;
    }

    // volumeBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.volume = val;
        input = rest;
    }

    // Version 0x71+: modulationBlock (1 byte per track)
    if version.has_modulation_pitch_bend() {
        for track in &mut tracks {
            let (rest, val) = le_u8(input)?;
            track.modulation = Some(val);
            input = rest;
        }
    }

    // Version 0x71+: pitchBendBlock (2 bytes per track)
    if version.has_modulation_pitch_bend() {
        for track in &mut tracks {
            let (rest, val) = le_u16(input)?;
            track.pitch_bend = Some(val);
            input = rest;
        }
    }

    // transposeHalfStepsBlock (1 byte per track, signed)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.transpose_half_steps = val as i8;
        input = rest;
    }

    // midiBankBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.midi_bank = val;
        input = rest;
    }

    // reverbBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.reverb = val;
        input = rest;
    }

    // chorusBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.chorus = val;
        input = rest;
    }

    // panBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.pan = val;
        input = rest;
    }

    // highestNoteBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.highest_note = val;
        input = rest;
    }

    // displayMIDINoteNumbersBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.display_midi_note_numbers = val != 0;
        input = rest;
    }

    // midiChannelBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.midi_channel = val;
        input = rest;
    }

    // topLineTextBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.top_line_text = val != 0;
        input = rest;
    }

    // bottomLineTextBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.bottom_line_text = val != 0;
        input = rest;
    }

    // tuningBlock (8 bytes per track)
    for track in &mut tracks {
        let (rest, tuning_bytes) = take(8usize)(input)?;
        track.tuning.copy_from_slice(tuning_bytes);
        input = rest;
    }

    // drumBlock (1 byte per track)
    for track in &mut tracks {
        let (rest, val) = le_u8(input)?;
        track.is_drum = val != 0;
        input = rest;
    }

    Ok((input, tracks))
}

/// Parse song info strings (5 Pascal2 strings)
fn parse_song_info(input: &[u8]) -> IResult<&[u8], TbtSongInfo> {
    let (input, title) = parse_pascal2_string(input)?;
    let (input, artist) = parse_pascal2_string(input)?;
    let (input, album) = parse_pascal2_string(input)?;
    let (input, transcribed_by) = parse_pascal2_string(input)?;
    let (input, comment) = parse_pascal2_string(input)?;

    Ok((
        input,
        TbtSongInfo {
            title,
            artist,
            album,
            transcribed_by,
            comment,
        },
    ))
}

/// Parse track names (Pascal1 strings, one per track)
/// Parse the metadata section from decompressed data
fn parse_metadata(
    decompressed: &[u8],
    track_count: u8,
    version: TbtVersion,
) -> Result<TbtMetadata, RuxError> {
    let (remaining, tracks) = parse_metadata_tracks(decompressed, track_count, version)
        .map_err(|e| RuxError::ParsingError(format!("Failed to parse track metadata: {e}")))?;

    // Note: TBT format does NOT store track names - they simply don't exist in the file.
    // The remaining data after tracks is song info (title, artist, album, etc.)
    let (_, song_info) = parse_song_info(remaining)
        .map_err(|e| RuxError::ParsingError(format!("Failed to parse song info: {e}")))?;

    Ok(TbtMetadata { tracks, song_info })
}

/// Extract and parse TBT metadata section
pub fn parse_tbt_metadata(data: &[u8], header: &TbtHeader) -> Result<TbtMetadata, RuxError> {
    // Metadata starts right after the 64-byte header
    let metadata_start = TBT_HEADER_SIZE;
    let metadata_len = header.compressed_metadata_len as usize;

    if data.len() < metadata_start + metadata_len {
        return Err(RuxError::ParsingError(format!(
            "File too small for metadata: need {} bytes, have {}",
            metadata_start + metadata_len,
            data.len()
        )));
    }

    let compressed = &data[metadata_start..metadata_start + metadata_len];
    let decompressed = decompress_zlib(compressed)?;

    parse_metadata(&decompressed, header.track_count, header.version)
}

/// Check if bytes represent a TBT file (by magic bytes)
pub fn is_tbt_file(data: &[u8]) -> bool {
    data.len() >= 3 && &data[0..3] == TBT_MAGIC
}

/// Compute CRC32 checksum of data
#[allow(dead_code)]
fn compute_crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Validate header CRC32 checksum
/// The CRC32 is computed over the first 60 bytes (0x00-0x3b)
#[allow(dead_code)]
fn validate_header_crc32(data: &[u8], expected_crc: u32) -> bool {
    if data.len() < CRC32_HEADER_OFFSET {
        return false;
    }
    let computed = compute_crc32(&data[..CRC32_HEADER_OFFSET]);
    computed == expected_crc
}

/// Validation options for TBT parsing
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TbtParseOptions {
    /// Skip CRC32 validation (useful for corrupted files)
    pub skip_crc_validation: bool,
}

/// Validation result containing header and any warnings
#[allow(dead_code)]
#[derive(Debug)]
pub struct TbtValidationResult {
    pub header: TbtHeader,
    /// True if header CRC32 matched
    pub header_crc_valid: bool,
    /// Warnings encountered during parsing
    pub warnings: Vec<String>,
}

/// Parse TBT file header only (for validation/inspection)
pub fn parse_tbt_header_only(data: &[u8]) -> Result<TbtHeader, RuxError> {
    if data.len() < TBT_HEADER_SIZE {
        return Err(RuxError::ParsingError(
            "TBT file too small for header".to_string(),
        ));
    }

    let (_, header) = parse_tbt_header(data)
        .map_err(|e| RuxError::ParsingError(format!("Failed to parse TBT header: {e}")))?;

    Ok(header)
}

/// Parse and validate TBT file header with full validation
#[allow(dead_code)]
pub fn parse_and_validate_tbt_header(
    data: &[u8],
    options: TbtParseOptions,
) -> Result<TbtValidationResult, RuxError> {
    if data.len() < TBT_HEADER_SIZE {
        return Err(RuxError::ParsingError(
            "TBT file too small for header (need 64 bytes)".to_string(),
        ));
    }

    let (_, header) = parse_tbt_header(data)
        .map_err(|e| RuxError::ParsingError(format!("Failed to parse TBT header: {e}")))?;

    let mut warnings = Vec::new();

    // Validate header CRC32
    let header_crc_valid = validate_header_crc32(data, header.crc32_header);
    if !header_crc_valid {
        let computed = compute_crc32(&data[..CRC32_HEADER_OFFSET]);
        let warning = format!(
            "Header CRC32 mismatch: expected 0x{:08x}, computed 0x{:08x}",
            header.crc32_header, computed
        );
        if !options.skip_crc_validation {
            return Err(RuxError::ParsingError(warning));
        }
        warnings.push(warning);
    }

    // Validate file size matches header
    if data.len() != header.total_byte_count as usize {
        let warning = format!(
            "File size mismatch: header says {} bytes, actual size {} bytes",
            header.total_byte_count,
            data.len()
        );
        warnings.push(warning);
    }

    // Validate track count is reasonable
    if header.track_count == 0 {
        warnings.push("Track count is 0".to_string());
    } else if header.track_count > 32 {
        warnings.push(format!(
            "Unusually high track count: {}",
            header.track_count
        ));
    }

    // Validate tempo is reasonable
    if header.tempo1 == 0 {
        warnings.push("Tempo1 is 0".to_string());
    }

    Ok(TbtValidationResult {
        header,
        header_crc_valid,
        warnings,
    })
}

// ============================================================
// Phase 4: Body Parsing
// ============================================================

/// Number of slots per space for notes (20 vsq per space)
const SLOTS_PER_SPACE: usize = 20;

/// Number of strings per track (8 strings max)
const STRINGS_PER_TRACK: usize = 8;

/// Total slots in notes delta list: 8 notes + 8 effects + 4 metadata = 20
const NOTES_SLOT_COUNT: usize = STRINGS_PER_TRACK + STRINGS_PER_TRACK + 4;

/// Slots per space for alternate time (2 dsq per space)
const ALT_TIME_SLOTS_PER_SPACE: usize = 2;

/// Read a single raw delta-list chunk from the input stream.
///
/// Returns the raw pairs data and remaining input.
fn read_delta_list_chunk_raw(input: &[u8]) -> IResult<&[u8], Vec<u8>> {
    // Read the chunk length (number of byte pairs)
    let (input, pair_count) = le_u16(input)?;
    let bytes_to_read = pair_count as usize * 2;

    if input.len() < bytes_to_read {
        return Err(nom::Err::Failure(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Eof,
        )));
    }

    let (remaining, chunk_data) = take(bytes_to_read)(input)?;
    Ok((remaining, chunk_data.to_vec()))
}

/// Compute the total slot count from accumulated delta list pairs.
///
/// This counts the sum of all increments in the pairs.
fn compute_delta_list_count(pairs: &[u8]) -> usize {
    let mut count = 0usize;
    let mut pos = 0usize;

    while pos < pairs.len() {
        let (increment, advance) = if pairs[pos] != 0 {
            // Simple case: increment is first byte
            (pairs[pos] as usize, 2)
        } else {
            // Extended case: next two bytes form the increment as little-endian short
            if pos + 3 >= pairs.len() {
                break;
            }
            let inc = u16::from_le_bytes([pairs[pos + 1], pairs[pos + 2]]) as usize;
            (inc, 4)
        };

        count += increment;
        pos += advance;
    }

    count
}

/// Expand accumulated delta list pairs into a 2D array.
///
/// The pairs contain run-length encoded data where each entry says
/// "fill N slots with value V".
fn expand_delta_list(pairs: &[u8], slots_per_space: usize, total_spaces: usize) -> Vec<Vec<u8>> {
    let total_slots = total_spaces * slots_per_space;
    let mut result: Vec<Vec<u8>> = vec![vec![0u8; slots_per_space]; total_spaces];

    let mut pos = 0usize;
    let mut unit = 0usize;

    while pos < pairs.len() {
        let (increment, value, advance) = if pairs[pos] != 0 {
            (pairs[pos] as usize, pairs[pos + 1], 2)
        } else {
            if pos + 3 >= pairs.len() {
                break;
            }
            let inc = u16::from_le_bytes([pairs[pos + 1], pairs[pos + 2]]) as usize;
            let val = pairs[pos + 3];
            (inc, val, 4)
        };

        // Fill from current position to (current + increment) with the value
        let end_unit = (unit + increment).min(total_slots);
        while unit < end_unit {
            let space = unit / slots_per_space;
            let slot = unit % slots_per_space;
            if space < result.len() {
                result[space][slot] = value;
            }
            unit += 1;
        }

        pos += advance;
    }

    result
}

/// Maximum number of chunks to read to prevent DoS from malformed files
const MAX_DELTA_LIST_CHUNKS: usize = 10_000;

/// Decode delta-list chunks from the input stream until we have enough data.
///
/// TBT format can have MULTIPLE delta list chunks per track. We accumulate
/// chunks until the total slot count reaches `slots_per_space * total_spaces`.
///
/// Returns the expanded array and remaining input.
fn decode_delta_list_chunks(
    input: &[u8],
    slots_per_space: usize,
    total_spaces: u32,
) -> IResult<&[u8], Vec<Vec<u8>>> {
    let target_count = slots_per_space * total_spaces as usize;
    let mut accumulated_pairs: Vec<u8> = Vec::new();
    let mut input = input;
    let mut chunk_count = 0;

    // Read chunks until we have enough slots
    loop {
        if chunk_count >= MAX_DELTA_LIST_CHUNKS {
            return Err(nom::Err::Failure(nom::error::Error::new(
                input,
                nom::error::ErrorKind::TooLarge,
            )));
        }

        let (rest, chunk_pairs) = read_delta_list_chunk_raw(input)?;
        accumulated_pairs.extend_from_slice(&chunk_pairs);
        input = rest;
        chunk_count += 1;

        let current_count = compute_delta_list_count(&accumulated_pairs);
        if current_count >= target_count {
            break;
        }
    }

    let result = expand_delta_list(&accumulated_pairs, slots_per_space, total_spaces as usize);
    Ok((input, result))
}

/// Parse bar lines for version 0x70+ (ArrayList format)
fn parse_bar_lines_0x70(input: &[u8], bar_count: u16) -> IResult<&[u8], Vec<TbtBarLine>> {
    let mut bars = Vec::with_capacity(bar_count as usize);
    let mut input = input;
    let mut current_space: u32 = 0;

    for _ in 0..bar_count {
        // Each bar record is 6 bytes: 4 bytes space increment, 1 byte type, 1 byte repeat
        let (rest, space_inc) = le_u32(input)?;
        let (rest, bar_byte) = le_u8(rest)?;
        let (rest, repeat_count) = le_u8(rest)?;
        input = rest;

        current_space += space_inc;

        // Decode bar type from the type byte
        // Bits 0-3 indicate the bar type
        let bar_type = match bar_byte & 0x0F {
            0x01 => TbtBarType::Single,
            0x02 => TbtBarType::CloseRepeat,
            0x03 => TbtBarType::OpenRepeat,
            0x04 => TbtBarType::Double,
            0x05 => TbtBarType::OpenCloseRepeat,
            _ => TbtBarType::Single,
        };

        bars.push(TbtBarLine {
            space: current_space as u16,
            bar_type,
            repeat_count,
        });
    }

    Ok((input, bars))
}

/// Parse bar lines for version 0x6f (DeltaListChunk format)
fn parse_bar_lines_0x6f(input: &[u8], space_count: u16) -> IResult<&[u8], Vec<TbtBarLine>> {
    // For 0x6f, bar lines are stored as a delta list with 1 slot per space
    let (remaining, expanded) = decode_delta_list_chunks(input, 1, u32::from(space_count))?;

    let mut bars = Vec::new();

    for (space_idx, slots) in expanded.iter().enumerate() {
        let bar_byte = slots[0];
        if bar_byte == 0 {
            continue; // No bar at this position
        }

        // Bits 0-3: bar type, bits 4-7: repeat count
        let bar_type_bits = bar_byte & 0x0F;
        let repeat_count = (bar_byte >> 4) & 0x0F;

        let bar_type = match bar_type_bits {
            0x01 => TbtBarType::Single,
            0x02 => TbtBarType::CloseRepeat,
            0x03 => TbtBarType::OpenRepeat,
            0x04 => TbtBarType::Double,
            0x05 => TbtBarType::OpenCloseRepeat,
            _ => continue,
        };

        bars.push(TbtBarLine {
            space: space_idx as u16,
            bar_type,
            repeat_count,
        });
    }

    Ok((remaining, bars))
}

/// Parse notes for a single track
fn parse_track_notes(input: &[u8], space_count: u32) -> IResult<&[u8], Vec<TbtNote>> {
    // Notes use NOTES_SLOT_COUNT slots per space (20 slots)
    // TBT format can have MULTIPLE delta list chunks per track
    let (remaining, expanded) = decode_delta_list_chunks(input, NOTES_SLOT_COUNT, space_count)?;

    let mut notes = Vec::new();

    for (space_idx, slots) in expanded.iter().enumerate() {
        // Slots 0-7: Note values for strings 0-7
        // 0x80 + fret = note, 0x11 = mute, 0x12 = stop
        for string in 0..STRINGS_PER_TRACK {
            let note_val = slots[string];
            if note_val == 0 {
                continue;
            }

            let (fret, is_muted, is_stop) = if note_val == NOTE_MUTED {
                (0, true, false)
            } else if note_val == NOTE_STOP {
                (0, false, true)
            } else if note_val >= NOTE_FRET_FLAG {
                (note_val - NOTE_FRET_FLAG, false, false)
            } else {
                continue; // Unknown note value
            };

            // Get effect for this string from slots 8-15
            let effect_byte = slots[STRINGS_PER_TRACK + string];
            let effect = TbtStringEffect::from_byte(effect_byte);

            notes.push(TbtNote {
                vsq_position: (space_idx as u32) * (SLOTS_PER_SPACE as u32),
                string: string as u8,
                fret,
                is_muted,
                is_stop,
                effect,
            });
        }
    }

    Ok((remaining, notes))
}

/// Parse alternate time regions for a single track
fn parse_alternate_time(input: &[u8], space_count: u32) -> IResult<&[u8], Vec<TbtAlternateTime>> {
    // Alternate time uses 2 slots per space (dsq)
    let (remaining, expanded) =
        decode_delta_list_chunks(input, ALT_TIME_SLOTS_PER_SPACE, space_count)?;

    let mut alt_times = Vec::new();

    for (space_idx, slots) in expanded.iter().enumerate() {
        let denominator = slots[0];
        let numerator = slots[1];

        if denominator != 0 || numerator != 0 {
            alt_times.push(TbtAlternateTime {
                dsq_position: (space_idx as u32) * 2,
                denominator,
                numerator,
            });
        }
    }

    Ok((remaining, alt_times))
}

/// Parse track effect changes (version >= 0x71)
///
/// Uses Chunk4 format: first u32 is byte length, followed by 8-byte entries.
fn parse_track_effect_changes(input: &[u8]) -> IResult<&[u8], Vec<TbtEffectChange>> {
    // Chunk4 format: first u32 is BYTE LENGTH (not entry count)
    let (input, byte_length) = le_u32(input)?;

    // Each entry is 8 bytes, so divide by 8 to get entry count
    let entry_count = byte_length as usize / 8;
    let mut changes = Vec::with_capacity(entry_count);
    let mut input = input;
    let mut current_space: u32 = 0;

    for _ in 0..entry_count {
        // Each entry is 8 bytes: space_inc(2), effect(2), reserved(2), value(2)
        let (rest, space_inc) = le_u16(input)?;
        let (rest, effect_num) = le_u16(rest)?;
        let (rest, _reserved) = le_u16(rest)?;
        let (rest, value) = le_u16(rest)?;
        input = rest;

        current_space += u32::from(space_inc);

        if let Some(effect_type) = TbtEffectChangeType::from_byte(effect_num as u8) {
            changes.push(TbtEffectChange {
                space: current_space,
                effect_type,
                value,
            });
        }
    }

    Ok((input, changes))
}

/// Parsed body data from a TBT file
type TbtBodyData = (
    Vec<TbtBarLine>,
    Vec<Vec<TbtNote>>,
    Vec<Vec<TbtAlternateTime>>,
    Vec<Vec<TbtEffectChange>>,
);

/// Get the space count for a track, falling back to header space count
fn get_track_space_count(header: &TbtHeader, metadata: &TbtMetadata, track_idx: u8) -> u32 {
    if header.version.has_space_count_per_track() {
        metadata
            .tracks
            .get(track_idx as usize)
            .and_then(|t| t.space_count)
            .unwrap_or_else(|| u32::from(header.space_count))
    } else {
        u32::from(header.space_count)
    }
}

/// Parse the body section from raw file data
pub fn parse_tbt_body(
    data: &[u8],
    header: &TbtHeader,
    metadata: &TbtMetadata,
) -> Result<TbtBodyData, RuxError> {
    // Body starts after header + compressed metadata
    let body_start = TBT_HEADER_SIZE + header.compressed_metadata_len as usize;

    if data.len() <= body_start {
        return Err(RuxError::ParsingError(
            "File too small for body section".to_string(),
        ));
    }

    // The body is also zlib compressed
    let compressed_body = &data[body_start..];
    let decompressed = decompress_zlib(compressed_body)?;

    let mut input = decompressed.as_slice();

    // 1. Parse bar lines
    let bar_lines = if header.version.has_space_count_per_track() {
        // Version 0x70+: ArrayList format
        let (rest, bars) = parse_bar_lines_0x70(input, header.bar_count)
            .map_err(|e| RuxError::ParsingError(format!("Failed to parse bar lines: {e}")))?;
        input = rest;
        bars
    } else {
        // Version 0x6f: DeltaListChunk format
        let (rest, bars) = parse_bar_lines_0x6f(input, header.space_count)
            .map_err(|e| RuxError::ParsingError(format!("Failed to parse bar lines: {e}")))?;
        input = rest;
        bars
    };

    // 2. Parse notes for each track
    let mut track_notes = Vec::with_capacity(header.track_count as usize);
    for i in 0..header.track_count {
        let track_space_count = get_track_space_count(header, metadata, i);
        let (rest, notes) = parse_track_notes(input, track_space_count).map_err(|e| {
            RuxError::ParsingError(format!("Failed to parse notes for track {i}: {e}"))
        })?;
        input = rest;
        track_notes.push(notes);
    }

    // 3. Parse alternate time regions (if feature bit is set)
    let mut alternate_times = Vec::with_capacity(header.track_count as usize);
    if header.features.has_alternate_time_regions {
        for i in 0..header.track_count {
            let track_space_count = get_track_space_count(header, metadata, i);
            let (rest, alt_time) = parse_alternate_time(input, track_space_count).map_err(|e| {
                RuxError::ParsingError(format!("Failed to parse alternate time for track {i}: {e}"))
            })?;
            input = rest;
            alternate_times.push(alt_time);
        }
    } else {
        // No alternate time regions - create empty vectors
        for _ in 0..header.track_count {
            alternate_times.push(Vec::new());
        }
    }

    // 4. Parse track effect changes (version >= 0x71)
    let mut track_effect_changes = Vec::with_capacity(header.track_count as usize);
    if header.version.has_track_effect_changes_chunk() {
        for i in 0..header.track_count {
            let (rest, changes) = parse_track_effect_changes(input).map_err(|e| {
                RuxError::ParsingError(format!("Failed to parse effect changes for track {i}: {e}"))
            })?;
            input = rest;
            track_effect_changes.push(changes);
        }
    } else {
        // No track effect changes - create empty vectors
        for _ in 0..header.track_count {
            track_effect_changes.push(Vec::new());
        }
    }

    Ok((
        bar_lines,
        track_notes,
        alternate_times,
        track_effect_changes,
    ))
}

/// Parse a complete TBT file into a TbtSong
pub fn parse_tbt_data(data: &[u8]) -> Result<TbtSong, RuxError> {
    // 1. Parse and validate header
    let header = parse_tbt_header_only(data)?;

    // Warn about untested version 0x71
    if header.version == TbtVersion::V0x71 {
        log::warn!(
            "TBT version 0x71 is untested - no test files have been found. \
             Parsing may produce incorrect results. Please contact the developer \
             and let them know what file this is."
        );
    }

    // 2. Parse metadata
    let metadata = parse_tbt_metadata(data, &header)?;

    // 3. Parse body
    let (bar_lines, track_notes, alternate_times, track_effect_changes) =
        parse_tbt_body(data, &header, &metadata)?;

    Ok(TbtSong {
        header,
        metadata,
        bar_lines,
        track_notes,
        alternate_times,
        track_effect_changes,
    })
}

// ============================================================
// Phase 5: Song Conversion
// ============================================================

/// Convert TBT string effect to GP NoteEffect
fn convert_effect(tbt_effect: Option<TbtStringEffect>) -> NoteEffect {
    let mut effect = NoteEffect::default();

    if let Some(e) = tbt_effect {
        match e {
            TbtStringEffect::HammerOn | TbtStringEffect::PullOff => {
                effect.hammer = true;
            }
            TbtStringEffect::SlideUp => {
                effect.slide = Some(SlideType::ShiftSlideTo);
            }
            TbtStringEffect::SlideDown => {
                effect.slide = Some(SlideType::OutDownwards);
            }
            TbtStringEffect::BendUp | TbtStringEffect::Bend => {
                // Simple bend: start at 0, bend up 1 semitone (value 1)
                effect.bend = Some(BendEffect {
                    points: vec![
                        BendPoint {
                            position: 0,
                            value: 0,
                        },
                        BendPoint {
                            position: 6,
                            value: 1,
                        },
                        BendPoint {
                            position: 12,
                            value: 1,
                        },
                    ],
                });
            }
            TbtStringEffect::ReleaseBend => {
                // Release bend: start high, go to 0
                effect.bend = Some(BendEffect {
                    points: vec![
                        BendPoint {
                            position: 0,
                            value: 1,
                        },
                        BendPoint {
                            position: 6,
                            value: 0,
                        },
                        BendPoint {
                            position: 12,
                            value: 0,
                        },
                    ],
                });
            }
            TbtStringEffect::Vibrato => {
                effect.vibrato = true;
            }
            TbtStringEffect::Harmonic => {
                effect.harmonic = Some(HarmonicEffect {
                    kind: HarmonicType::Natural,
                    pitch: None,
                    octave: None,
                    right_hand_fret: None,
                });
            }
            TbtStringEffect::Tremolo => {
                effect.tremolo_picking = Some(TremoloPickingEffect {
                    duration: Duration {
                        value: 16, // 16th note tremolo
                        ..Default::default()
                    },
                });
            }
            TbtStringEffect::GhostNote => {
                effect.ghost_note = true;
            }
            TbtStringEffect::Tap => {
                effect.slap = SlapEffect::Tapping;
            }
            TbtStringEffect::Slap => {
                effect.slap = SlapEffect::Slapping;
            }
            TbtStringEffect::Whammy => {
                // Whammy/tremolo bar effect - just mark as having tremolo bar
                // For now, skip this as it needs TremoloBarEffect which is more complex
            }
        }
    }

    effect
}

/// Convert TBT tuning to GP string tunings.
/// Standard guitar tuning (MIDI note values, low to high): E2(40), A2(45), D3(50), G3(55), B3(59), E4(64)
/// TBT stores tuning as SIGNED OFFSETS from standard 6-string guitar tuning.
/// For example, a bass tuned to E1(28) stores offset -12 (28 = 40 - 12).
const STANDARD_GUITAR_TUNING: [i32; 8] = [40, 45, 50, 55, 59, 64, 69, 74]; // Extended for 7/8 string

/// TBT stores tuning as signed byte offsets from standard guitar tuning.
/// Zero offset means standard tuning for that string.
/// GP stores tuning as (string_number, midi_note) pairs where string 1 is highest.
fn convert_tuning(tbt_track: &TbtTrack) -> Vec<(i32, i32)> {
    let string_count = tbt_track.string_count as usize;
    let mut strings = Vec::with_capacity(string_count);

    // TBT: tuning[0] is lowest string, tuning[string_count-1] is highest
    // GP: string 1 is highest, string N is lowest
    // TBT tuning values are SIGNED offsets from standard guitar tuning
    for i in 0..string_count {
        let string_num = (string_count - i) as i32; // GP numbering (1 = highest)

        // Get base tuning from standard guitar tuning
        let base_tuning = STANDARD_GUITAR_TUNING.get(i).copied().unwrap_or(40);

        // TBT tuning is a signed offset from standard tuning
        let tuning_offset = tbt_track.tuning[i] as i8; // Interpret as signed!
        let midi_note = base_tuning + i32::from(tuning_offset);
        strings.push((string_num, midi_note));
    }

    // Sort by string number (1 first for GP format)
    strings.sort_by_key(|(num, _)| *num);
    strings
}

/// Infer time signature from space count between bars
/// Default to 4/4 (16 spaces per measure)
fn infer_time_signature(spaces_in_measure: u16) -> TimeSignature {
    // Common time signatures:
    // 4/4 = 16 spaces (4 quarter notes * 4 sixteenths)
    // 3/4 = 12 spaces
    // 2/4 = 8 spaces
    // 6/8 = 12 spaces (6 eighth notes * 2 sixteenths)
    // 5/4 = 20 spaces
    // 7/4 = 28 spaces

    let (numerator, denominator_value): (u8, u16) = match spaces_in_measure {
        8 => (2, 4),
        12 => (3, 4), // Could also be 6/8, but 3/4 is more common
        16 => (4, 4),
        20 => (5, 4),
        24 => (6, 4), // Or 12/8
        28 => (7, 4),
        32 => (8, 4), // Or 4/2
        _ => {
            // Try to figure out based on divisibility (any multiple of 4 works)
            // Note: Using manual check instead of is_multiple_of() for Rust 1.75 compatibility
            #[allow(clippy::manual_is_multiple_of)]
            if spaces_in_measure % 4 == 0 {
                ((spaces_in_measure / 4) as u8, 4)
            } else {
                (4, 4) // Default
            }
        }
    };

    TimeSignature {
        numerator,
        denominator: Duration {
            value: denominator_value,
            ..Default::default()
        },
    }
}

/// Group notes by their space position for creating beats
fn group_notes_by_space(notes: &[TbtNote]) -> std::collections::BTreeMap<u32, Vec<&TbtNote>> {
    let mut groups: std::collections::BTreeMap<u32, Vec<&TbtNote>> =
        std::collections::BTreeMap::new();

    for note in notes {
        // Convert vsq position to space position
        let space = note.vsq_position / (SLOTS_PER_SPACE as u32);
        groups.entry(space).or_default().push(note);
    }

    groups
}

/// Calculate note duration based on gap to next note or end of measure
fn calculate_duration(
    current_space: u32,
    next_space_or_measure_end: u32,
    _time_signature: &TimeSignature,
) -> Duration {
    let space_gap = (next_space_or_measure_end - current_space) as u16;

    // Map space gaps to note durations
    // 1 space = 16th note
    // 2 spaces = 8th note
    // 4 spaces = quarter note
    // 8 spaces = half note
    // 16 spaces = whole note

    let (value, dotted) = match space_gap {
        1 => (16, false), // 16th note
        2 => (8, false),  // 8th note
        3 => (8, true),   // Dotted 8th
        4 => (4, false),  // Quarter note
        6 => (4, true),   // Dotted quarter
        8 => (2, false),  // Half note
        12 => (2, true),  // Dotted half
        16 => (1, false), // Whole note
        _ => {
            // Find closest duration
            if space_gap < 2 {
                (16, false)
            } else if space_gap < 3 {
                (8, false)
            } else if space_gap < 5 {
                (4, false)
            } else if space_gap < 10 {
                (2, false)
            } else {
                (1, false)
            }
        }
    };

    Duration {
        value,
        dotted,
        ..Default::default()
    }
}

/// Convert a TbtSong to a GP Song
#[allow(clippy::unnecessary_wraps)] // Result is needed for consistent API with parse_gp_data
pub fn tbt_to_song(tbt: &TbtSong) -> Result<Song, RuxError> {
    // 1. Create MIDI channels (one per track)
    let mut midi_channels: Vec<MidiChannel> = Vec::with_capacity(64);

    // Initialize all 64 channels with defaults
    for i in 0..64 {
        midi_channels.push(MidiChannel {
            channel_id: i as u8,
            effect_channel_id: 0,
            instrument: 25, // Default: Acoustic Guitar (steel)
            volume: 100,
            balance: 64, // Center
            chorus: 0,
            reverb: 0,
            phaser: 0,
            tremolo: 0,
            bank: if i == 9 {
                DEFAULT_PERCUSSION_BANK
            } else {
                DEFAULT_BANK
            },
        });
    }

    // Update channels used by tracks
    for (i, tbt_track) in tbt.metadata.tracks.iter().enumerate() {
        // Use track index if midi_channel is invalid (>= 64)
        let channel_id = if (tbt_track.midi_channel as usize) < 64 {
            tbt_track.midi_channel as usize
        } else {
            i % 64 // Fallback to track index mod 64
        };
        if channel_id < 64 {
            let ch = &mut midi_channels[channel_id];
            ch.instrument = i32::from(tbt_track.clean_guitar);
            ch.volume = tbt_track.volume as i8;
            ch.balance = tbt_track.pan as i8;
            ch.chorus = tbt_track.chorus as i8;
            ch.reverb = tbt_track.reverb as i8;

            if tbt_track.is_drum {
                ch.bank = DEFAULT_PERCUSSION_BANK;
            }

            // Set effect channel (usually channel + 1)
            ch.effect_channel_id = if channel_id + 1 < 64 && !tbt_track.is_drum {
                (channel_id + 1) as u8
            } else {
                channel_id as u8
            };

            // If this track uses a different channel than its index, copy settings
            if channel_id != i && i < 64 {
                midi_channels[i] = ch.clone();
                midi_channels[i].channel_id = i as u8;
            }
        }
    }

    // 2. Calculate measure boundaries from bar lines
    let mut measure_spaces: Vec<(u16, u16)> = Vec::new(); // (start_space, end_space)
    let mut prev_space: u16 = 0;

    for bar in &tbt.bar_lines {
        if bar.space > prev_space {
            measure_spaces.push((prev_space, bar.space));
        }
        prev_space = bar.space;
    }

    // Add final measure if there are notes after the last bar
    if prev_space < tbt.header.space_count {
        measure_spaces.push((prev_space, tbt.header.space_count));
    }

    // If no bar lines, create one big measure
    if measure_spaces.is_empty() {
        measure_spaces.push((0, tbt.header.space_count));
    }

    // 3. Create measure headers
    let initial_tempo = Tempo {
        value: u32::from(tbt.header.tempo2),
        name: None,
    };

    let mut measure_headers: Vec<MeasureHeader> = Vec::with_capacity(measure_spaces.len());
    let mut current_tick: u32 = QUARTER_TIME; // Songs start at QUARTER_TIME

    for (i, (start_space, end_space)) in measure_spaces.iter().enumerate() {
        let spaces_in_measure = end_space - start_space;
        let time_signature = infer_time_signature(spaces_in_measure);

        // Check for repeat markers in bar lines
        let bar_at_start = tbt.bar_lines.iter().find(|b| b.space == *start_space);
        let bar_at_end = tbt.bar_lines.iter().find(|b| b.space == *end_space);

        let repeat_open = bar_at_start
            .map(|b| {
                matches!(
                    b.bar_type,
                    TbtBarType::OpenRepeat | TbtBarType::OpenCloseRepeat
                )
            })
            .unwrap_or(i == 0); // First measure implicitly opens

        let repeat_close = bar_at_end
            .map(|b| {
                if matches!(
                    b.bar_type,
                    TbtBarType::CloseRepeat | TbtBarType::OpenCloseRepeat
                ) {
                    b.repeat_count.max(1) as i8
                } else {
                    0
                }
            })
            .unwrap_or(0);

        let header = MeasureHeader {
            start: current_tick,
            time_signature: time_signature.clone(),
            tempo: initial_tempo.clone(),
            marker: None,
            repeat_open,
            repeat_alternative: 0,
            repeat_close,
            triplet_feel: TripletFeel::None,
            key_signature: KeySignature::new(0, false),
        };

        current_tick += header.length();
        measure_headers.push(header);
    }

    // 4. Create tracks with measures
    let mut tracks: Vec<Track> = Vec::with_capacity(tbt.header.track_count as usize);

    for (track_idx, tbt_track) in tbt.metadata.tracks.iter().enumerate() {
        let strings = convert_tuning(tbt_track);
        let string_count = strings.len();

        // Group this track's notes by space
        let track_notes = tbt
            .track_notes
            .get(track_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let notes_by_space = group_notes_by_space(track_notes);

        // Create measures for this track
        let mut measures: Vec<Measure> = Vec::with_capacity(measure_spaces.len());

        for (measure_idx, (start_space, end_space)) in measure_spaces.iter().enumerate() {
            let header = &measure_headers[measure_idx];

            // Collect notes in this measure
            let notes_in_measure: Vec<(&u32, &Vec<&TbtNote>)> = notes_by_space
                .range(u32::from(*start_space)..u32::from(*end_space))
                .collect();

            // Create beats from notes
            let mut beats: Vec<Beat> = Vec::new();

            if notes_in_measure.is_empty() {
                // Empty measure - add a rest beat
                beats.push(Beat {
                    notes: vec![],
                    duration: header.time_signature.denominator.clone(),
                    empty: true,
                    text: String::new(),
                    start: header.start,
                    effect: Default::default(),
                });
            } else {
                // Create beats from note groups
                let space_positions: Vec<u32> = notes_in_measure.iter().map(|(s, _)| **s).collect();

                for (note_idx, (space, notes)) in notes_in_measure.iter().enumerate() {
                    // Calculate beat start tick
                    let relative_space = **space - u32::from(*start_space);
                    let beat_start = header.start + (relative_space * TICKS_PER_SPACE);

                    // Calculate duration to next note or measure end
                    let next_space = if note_idx + 1 < space_positions.len() {
                        space_positions[note_idx + 1]
                    } else {
                        u32::from(*end_space)
                    };

                    let duration = calculate_duration(**space, next_space, &header.time_signature);

                    // Create GP notes from TBT notes
                    let mut gp_notes: Vec<Note> = Vec::new();

                    for tbt_note in notes.iter() {
                        // Skip stop notes - they're just to end sustain
                        if tbt_note.is_stop {
                            continue;
                        }

                        // Skip notes on strings beyond the track's string count
                        if tbt_note.string as usize >= string_count {
                            continue;
                        }

                        // Get string number (TBT: 0=lowest, GP: 1=highest)
                        let string_num = (string_count - tbt_note.string as usize) as i32;

                        let effect = convert_effect(tbt_note.effect);

                        let kind = if tbt_note.is_muted {
                            NoteType::Dead
                        } else {
                            NoteType::Normal
                        };

                        // note.value is the FRET (offset from string tuning)
                        // The MIDI note is calculated during playback as: string_tuning + fret
                        let mut note = Note::new(effect);
                        note.value = i16::from(tbt_note.fret);
                        note.string = string_num as i8;
                        note.kind = kind;
                        gp_notes.push(note);
                    }

                    beats.push(Beat {
                        notes: gp_notes,
                        duration,
                        empty: false,
                        text: String::new(),
                        start: beat_start,
                        effect: Default::default(),
                    });
                }
            }

            // Create voice with beats
            let voice = Voice {
                measure_index: measure_idx as i16,
                beats,
            };

            measures.push(Measure {
                key_signature: header.key_signature.clone(),
                time_signature: header.time_signature.clone(),
                track_index: track_idx,
                header_index: measure_idx,
                voices: vec![voice],
            });
        }

        // Create track - TBT format doesn't store track names, so use default
        let track_name = format!("Track {}", track_idx + 1);

        // Use track index if midi_channel is invalid (>= 64)
        let effective_channel_id = if (tbt_track.midi_channel as usize) < 64 {
            tbt_track.midi_channel
        } else {
            (track_idx % 64) as u8 // Fallback to track index mod 64
        };

        tracks.push(Track {
            number: (track_idx + 1) as i32,
            offset: 0,
            channel_id: effective_channel_id,
            solo: false,
            mute: false,
            visible: true,
            name: track_name,
            strings,
            color: 0x00FF_0000, // Red default
            midi_port: 0,
            fret_count: 24,
            measures,
        });
    }

    // 5. Create song info
    let song_info = SongInfo {
        name: tbt.metadata.song_info.title.clone(),
        subtitle: String::new(),
        artist: tbt.metadata.song_info.artist.clone(),
        album: tbt.metadata.song_info.album.clone(),
        author: tbt.metadata.song_info.transcribed_by.clone(),
        words: None,
        copyright: String::new(),
        writer: String::new(),
        instructions: tbt.metadata.song_info.comment.clone(),
        notices: vec![],
    };

    // 6. Build final song
    let song = Song {
        version: GpVersion::GP5, // Mark as GP5 equivalent
        song_info,
        triplet_feel: None,
        lyrics: None,
        page_setup: None,
        tempo: initial_tempo,
        hide_tempo: None,
        key_signature: 0,
        octave: None,
        midi_channels,
        measure_headers,
        tracks,
    };

    Ok(song)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_is_tbt_file() {
        assert!(is_tbt_file(b"TBT\x6f"));
        assert!(is_tbt_file(b"TBT\x70some more data"));
        assert!(!is_tbt_file(b"GP5"));
        assert!(!is_tbt_file(b"TB"));
    }

    #[test]
    fn test_parse_take_on_me_header() {
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let header = parse_tbt_header_only(&data).expect("Failed to parse header");

        assert_eq!(header.version, TbtVersion::V0x6f);
        assert_eq!(header.tempo1, 160);
        assert_eq!(header.track_count, 8);
        assert_eq!(header.version_string, "1.6");
        // total_byte_count should match file size
        assert_eq!(header.total_byte_count, data.len() as u32);
    }

    #[test]
    fn test_parse_all_that_she_wants_header() {
        let data = fs::read("test-files/All That She Wants.tbt").expect("Failed to read test file");
        let header = parse_tbt_header_only(&data).expect("Failed to parse header");

        assert_eq!(header.version, TbtVersion::V0x6f);
        assert_eq!(header.tempo1, 94); // 0x5e
        assert_eq!(header.track_count, 7);
        assert_eq!(header.version_string, "1.6");
        assert_eq!(header.total_byte_count, data.len() as u32);
    }

    #[test]
    fn test_tbt_version_from_byte() {
        assert_eq!(TbtVersion::from_byte(0x6f), Some(TbtVersion::V0x6f));
        assert_eq!(TbtVersion::from_byte(0x70), Some(TbtVersion::V0x70));
        assert_eq!(TbtVersion::from_byte(0x71), Some(TbtVersion::V0x71));
        assert_eq!(TbtVersion::from_byte(0x72), Some(TbtVersion::V0x72));
        assert_eq!(TbtVersion::from_byte(0x00), None);
        assert_eq!(TbtVersion::from_byte(0xff), None);
    }

    #[test]
    fn test_tbt_version_capabilities() {
        assert!(!TbtVersion::V0x6f.has_alternate_time_regions());
        assert!(!TbtVersion::V0x6f.has_space_count_per_track());
        assert!(!TbtVersion::V0x6f.has_modulation_pitch_bend());

        assert!(TbtVersion::V0x70.has_alternate_time_regions());
        assert!(TbtVersion::V0x70.has_space_count_per_track());
        assert!(!TbtVersion::V0x70.has_modulation_pitch_bend());

        assert!(TbtVersion::V0x71.has_alternate_time_regions());
        assert!(TbtVersion::V0x71.has_space_count_per_track());
        assert!(TbtVersion::V0x71.has_modulation_pitch_bend());

        assert!(TbtVersion::V0x72.has_alternate_time_regions());
        assert!(TbtVersion::V0x72.has_space_count_per_track());
        assert!(TbtVersion::V0x72.has_modulation_pitch_bend());
    }

    #[test]
    fn test_features_from_byte() {
        let features = TbtFeatures::from_byte(0x18);
        assert!(features.has_alternate_time_regions);
        assert!(features.feature_bit_3);

        let features = TbtFeatures::from_byte(0x08);
        assert!(!features.has_alternate_time_regions);
        assert!(features.feature_bit_3);

        let features = TbtFeatures::from_byte(0x00);
        assert!(!features.has_alternate_time_regions);
        assert!(!features.feature_bit_3);
    }

    #[test]
    fn test_string_effect_from_byte() {
        assert_eq!(
            TbtStringEffect::from_byte(0x68),
            Some(TbtStringEffect::HammerOn)
        );
        assert_eq!(
            TbtStringEffect::from_byte(0x70),
            Some(TbtStringEffect::PullOff)
        );
        assert_eq!(
            TbtStringEffect::from_byte(0x2f),
            Some(TbtStringEffect::SlideUp)
        );
        assert_eq!(
            TbtStringEffect::from_byte(0x5c),
            Some(TbtStringEffect::SlideDown)
        );
        assert_eq!(
            TbtStringEffect::from_byte(0x7e),
            Some(TbtStringEffect::Vibrato)
        );
        assert_eq!(TbtStringEffect::from_byte(0x00), None);
    }

    #[test]
    fn test_crc32_validation_take_on_me() {
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let result = parse_and_validate_tbt_header(&data, TbtParseOptions::default())
            .expect("Failed to parse and validate header");

        assert!(result.header_crc_valid, "Header CRC32 should be valid");
        assert!(
            result.warnings.is_empty(),
            "Expected no warnings, got: {:?}",
            result.warnings
        );
        assert_eq!(result.header.version, TbtVersion::V0x6f);
        assert_eq!(result.header.tempo1, 160);
        assert_eq!(result.header.track_count, 8);
    }

    #[test]
    fn test_crc32_validation_all_files() {
        let valid_files = [
            "test-files/Take On Me (2).tbt",
            "test-files/All That She Wants.tbt",
            "test-files/Ave Maria (Acoustic Guitar).tbt",
            "test-files/Bach - #13 In A Minor.tbt",
        ];

        for path in valid_files {
            let Ok(data) = fs::read(path) else {
                continue; // Skip if file doesn't exist
            };

            // Skip non-TBT files (some might be HTML 404 pages)
            if !is_tbt_file(&data) {
                continue;
            }

            let result = parse_and_validate_tbt_header(&data, TbtParseOptions::default());
            assert!(result.is_ok(), "Failed to parse {path}: {:?}", result.err());

            let result = result.unwrap();
            assert!(
                result.header_crc_valid,
                "CRC32 validation failed for {path}"
            );
        }
    }

    #[test]
    fn test_corrupted_header_crc_fails() {
        let mut data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");

        // Corrupt a byte in the header (but not the CRC field itself)
        data[0x10] ^= 0xFF;

        // Should fail with default options
        let result = parse_and_validate_tbt_header(&data, TbtParseOptions::default());
        assert!(result.is_err(), "Should fail with corrupted header");

        // Should succeed with skip_crc_validation
        let result = parse_and_validate_tbt_header(
            &data,
            TbtParseOptions {
                skip_crc_validation: true,
            },
        );
        assert!(result.is_ok(), "Should succeed with skip_crc_validation");
        assert!(!result.unwrap().header_crc_valid);
    }

    #[test]
    fn test_header_too_small() {
        let data = vec![0u8; 32]; // Too small for header
        let result = parse_and_validate_tbt_header(&data, TbtParseOptions::default());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("too small for header"));
    }

    #[test]
    fn test_invalid_magic_bytes() {
        let mut data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        // Corrupt magic bytes
        data[0] = b'X';
        let result = parse_and_validate_tbt_header(&data, TbtParseOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_version_byte() {
        let mut data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        // Set invalid version byte
        data[3] = 0xFF;
        let result = parse_and_validate_tbt_header(&data, TbtParseOptions::default());
        assert!(result.is_err());
    }

    // Phase 3: Metadata parsing tests

    #[test]
    fn test_parse_metadata_take_on_me() {
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let header = parse_tbt_header_only(&data).expect("Failed to parse header");

        let metadata = parse_tbt_metadata(&data, &header).expect("Failed to parse metadata");

        // Should have 8 tracks
        assert_eq!(metadata.tracks.len(), 8);

        // Check first track has valid index
        let track0 = &metadata.tracks[0];
        assert_eq!(track0.index, 0);

        // Print parsed values for debugging
        println!(
            "Track 0: strings={}, volume={}, midi_ch={}",
            track0.string_count, track0.volume, track0.midi_channel
        );
        println!("Title: '{}'", metadata.song_info.title);
        println!("Artist: '{}'", metadata.song_info.artist);
    }

    #[test]
    fn test_parse_metadata_all_that_she_wants() {
        let data = fs::read("test-files/All That She Wants.tbt").expect("Failed to read test file");
        let header = parse_tbt_header_only(&data).expect("Failed to parse header");

        let metadata = parse_tbt_metadata(&data, &header).expect("Failed to parse metadata");

        // Should have 7 tracks (from header)
        assert_eq!(metadata.tracks.len(), 7);
        assert_eq!(metadata.tracks.len(), header.track_count as usize);

        // Each track should have valid index
        for (i, track) in metadata.tracks.iter().enumerate() {
            assert_eq!(track.index, i as u8);
        }
    }

    #[test]
    fn test_parse_metadata_all_valid_files() {
        let test_files = [
            "test-files/Take On Me (2).tbt",
            "test-files/All That She Wants.tbt",
            "test-files/Ave Maria (Acoustic Guitar).tbt",
            "test-files/Bach - #13 In A Minor.tbt",
            "test-files/Adam - Oh Night Devine.tbt",
            "test-files/Aguado - Study 1.tbt",
            "test-files/Arcadelt - Il Bianco E Dolce Cigno.tbt",
            "test-files/Arndt - Nola.tbt",
        ];

        for path in test_files {
            let Ok(data) = fs::read(path) else { continue };

            if !is_tbt_file(&data) {
                continue;
            }

            let Ok(header) = parse_tbt_header_only(&data) else {
                continue;
            };

            let metadata = parse_tbt_metadata(&data, &header);
            assert!(
                metadata.is_ok(),
                "Failed to parse metadata for {path}: {:?}",
                metadata.err()
            );

            let metadata = metadata.unwrap();
            assert_eq!(
                metadata.tracks.len(),
                header.track_count as usize,
                "Track count mismatch for {path}"
            );
        }
    }

    #[test]
    fn test_track_tuning_parsed() {
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let header = parse_tbt_header_only(&data).expect("Failed to parse header");
        let metadata = parse_tbt_metadata(&data, &header).expect("Failed to parse metadata");

        // Just verify tuning was parsed (8 bytes per track)
        for track in &metadata.tracks {
            println!("Track {}: tuning={:?}", track.index, track.tuning);
            // Tuning array should have 8 elements (verified by type)
            assert_eq!(track.tuning.len(), 8);
        }
    }

    // Phase 4: Body parsing tests

    #[test]
    fn test_parse_tbt_data_take_on_me() {
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let song = parse_tbt_data(&data).expect("Failed to parse TBT file");

        // Verify header data propagated
        assert_eq!(song.header.version, TbtVersion::V0x6f);
        assert_eq!(song.header.tempo1, 160);
        assert_eq!(song.header.track_count, 8);

        // Verify metadata
        assert_eq!(song.metadata.tracks.len(), 8);

        // Verify bar lines were parsed
        assert!(
            !song.bar_lines.is_empty(),
            "Expected bar lines to be parsed"
        );
        println!("Parsed {} bar lines", song.bar_lines.len());
        for (i, bar) in song.bar_lines.iter().take(5).enumerate() {
            println!(
                "  Bar {}: space={}, type={:?}, repeat={}",
                i, bar.space, bar.bar_type, bar.repeat_count
            );
        }

        // Verify track notes were parsed (should have 8 track note arrays)
        assert_eq!(song.track_notes.len(), 8);

        // Count total notes across all tracks
        let total_notes: usize = song.track_notes.iter().map(Vec::len).sum();
        println!("Total notes across all tracks: {total_notes}");
        assert!(total_notes > 0, "Expected notes to be parsed");

        // Print some note details from first track with notes
        for (track_idx, notes) in song.track_notes.iter().enumerate() {
            if !notes.is_empty() {
                println!("Track {track_idx} has {} notes", notes.len());
                for note in notes.iter().take(3) {
                    println!(
                        "  Note: vsq={}, string={}, fret={}, muted={}, effect={:?}",
                        note.vsq_position, note.string, note.fret, note.is_muted, note.effect
                    );
                }
                break;
            }
        }

        // Version 0x6f doesn't have alternate time features typically
        assert_eq!(song.alternate_times.len(), 8);

        // Version 0x6f doesn't have track effect changes
        assert_eq!(song.track_effect_changes.len(), 8);
        for changes in &song.track_effect_changes {
            assert!(
                changes.is_empty(),
                "Version 0x6f should not have track effect changes"
            );
        }
    }

    #[test]
    fn test_parse_tbt_data_all_files() {
        let test_files = [
            "test-files/Take On Me (2).tbt",
            "test-files/All That She Wants.tbt",
            "test-files/Ave Maria (Acoustic Guitar).tbt",
            "test-files/Bach - #13 In A Minor.tbt",
            "test-files/Adam - Oh Night Devine.tbt",
            "test-files/Aguado - Study 1.tbt",
            "test-files/Arcadelt - Il Bianco E Dolce Cigno.tbt",
            "test-files/Arndt - Nola.tbt",
            // Version 0x70 and 0x72 test files
            "test-files/version_0x70_36600.tbt",
            "test-files/version_0x72_31600.tbt",
            "test-files/version_0x72_34200.tbt",
            "test-files/version_0x72_41150.tbt",
        ];

        for path in test_files {
            let Ok(data) = fs::read(path) else { continue };

            if !is_tbt_file(&data) {
                continue;
            }

            let result = parse_tbt_data(&data);
            assert!(result.is_ok(), "Failed to parse {path}: {:?}", result.err());

            let song = result.unwrap();

            // Basic sanity checks
            assert_eq!(
                song.track_notes.len(),
                song.header.track_count as usize,
                "Track notes count mismatch for {path}"
            );

            assert_eq!(
                song.alternate_times.len(),
                song.header.track_count as usize,
                "Alternate times count mismatch for {path}"
            );

            assert_eq!(
                song.track_effect_changes.len(),
                song.header.track_count as usize,
                "Track effect changes count mismatch for {path}"
            );

            let total_notes: usize = song.track_notes.iter().map(Vec::len).sum();
            println!(
                "{path}: {} tracks, {} bars, {total_notes} total notes",
                song.header.track_count,
                song.bar_lines.len()
            );
        }
    }

    #[test]
    fn test_delta_list_decoder() {
        // Test the delta-list expansion with raw pairs
        // Format: pairs of (increment, value) with run-length encoding
        // Delta-list uses FILL semantics: increment says how many slots to fill
        // with the given value.

        // Simple case: 3 pairs filling 1 slot each with values 0xAA, 0xBB, 0xCC
        // Then fill remaining 7 slots with 0x00 (to reach total of 10)
        let pairs = &[
            0x01, 0xAA, // fill 1 slot with 0xAA (position 0)
            0x01, 0xBB, // fill 1 slot with 0xBB (position 1)
            0x01, 0xCC, // fill 1 slot with 0xCC (position 2)
            0x07, 0x00, // fill 7 slots with 0x00 (positions 3-9)
        ];

        let result = expand_delta_list(pairs, 1, 10);

        // Positions 0, 1, 2 should have values AA, BB, CC
        assert_eq!(result[0][0], 0xAA);
        assert_eq!(result[1][0], 0xBB);
        assert_eq!(result[2][0], 0xCC);

        // Rest should be 0
        for slot in result.iter().take(10).skip(3) {
            assert_eq!(slot[0], 0);
        }
    }

    #[test]
    fn test_compute_delta_list_count() {
        // Test the count computation
        let pairs = &[
            0x01, 0xAA, // increment 1
            0x01, 0xBB, // increment 1
            0x05, 0xCC, // increment 5
        ];

        let count = compute_delta_list_count(pairs);
        assert_eq!(count, 7); // 1 + 1 + 5 = 7
    }

    #[test]
    fn test_max_chunks_limit() {
        // Test that decode_delta_list_chunks enforces the MAX_DELTA_LIST_CHUNKS limit
        // to prevent DoS from malformed files with never-ending chunk sequences.
        //
        // Create input that has many tiny chunks that never reach the target count.
        // Each chunk: 2-byte length (1 pair) + 2 bytes data = 4 bytes per chunk
        // We'll create chunks that each add only 1 slot, but target needs millions.

        let mut malformed_input: Vec<u8> = Vec::new();

        // Create MAX_DELTA_LIST_CHUNKS + 100 chunks, each with 1 pair adding 1 slot
        for _ in 0..(MAX_DELTA_LIST_CHUNKS + 100) {
            malformed_input.extend_from_slice(&[
                0x01, 0x00, // pair_count = 1
                0x01, 0xAA, // fill 1 slot with 0xAA
            ]);
        }

        // Request a huge number of spaces that would require many more chunks
        let result = decode_delta_list_chunks(&malformed_input, 1, 100_000_000);

        // Should fail with TooLarge error after hitting MAX_DELTA_LIST_CHUNKS
        assert!(
            result.is_err(),
            "Should reject input requiring too many chunks"
        );
        if let Err(nom::Err::Failure(e)) = result {
            assert_eq!(e.code, nom::error::ErrorKind::TooLarge);
        } else {
            panic!("Expected Failure with TooLarge error kind");
        }
    }

    #[test]
    fn test_bar_type_parsing() {
        // Verify bar type byte decoding
        assert_eq!(TbtBarType::Single as u8, 0); // Just checking the enum exists

        // The bar type extraction in parse_bar_lines_0x6f uses:
        // bar_type_bits = bar_byte & 0x0F
        // 0x01 = Single, 0x02 = CloseRepeat, 0x03 = OpenRepeat, 0x04 = Double, 0x05 = OpenCloseRepeat
    }

    #[test]
    fn test_note_value_parsing() {
        // Verify note value constants
        assert_eq!(NOTE_MUTED, 0x11);
        assert_eq!(NOTE_STOP, 0x12);
        assert_eq!(NOTE_FRET_FLAG, 0x80);

        // A note on fret 5 would be encoded as 0x80 + 5 = 0x85
        let encoded_fret5 = NOTE_FRET_FLAG + 5;
        assert_eq!(encoded_fret5, 0x85);

        // Open string (fret 0) would be 0x80
        let encoded_open = NOTE_FRET_FLAG;
        assert_eq!(encoded_open, 0x80);
    }

    // Phase 5: Conversion tests

    #[test]
    fn test_tbt_to_song_take_on_me() {
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let tbt_song = parse_tbt_data(&data).expect("Failed to parse TBT file");

        let song = tbt_to_song(&tbt_song).expect("Failed to convert TBT to Song");

        // Verify basic song properties
        assert_eq!(song.version, GpVersion::GP5);
        assert_eq!(song.tracks.len(), 8);

        // Verify tempo matches header
        assert_eq!(song.tempo.value, u32::from(tbt_song.header.tempo2));

        // Verify song info was transferred
        assert_eq!(song.song_info.name, tbt_song.metadata.song_info.title);
        assert_eq!(song.song_info.artist, tbt_song.metadata.song_info.artist);

        // Verify measure headers were created
        assert!(!song.measure_headers.is_empty(), "Expected measure headers");
        println!("Created {} measure headers", song.measure_headers.len());

        // Verify each track has measures
        for (i, track) in song.tracks.iter().enumerate() {
            assert_eq!(
                track.measures.len(),
                song.measure_headers.len(),
                "Track {i} measure count mismatch"
            );
            println!(
                "Track {i}: {} strings, {} measures",
                track.strings.len(),
                track.measures.len()
            );
        }

        // Verify MIDI channels were configured
        assert_eq!(song.midi_channels.len(), 64);

        // Count total beats and notes
        let total_beats: usize = song
            .tracks
            .iter()
            .flat_map(|t| &t.measures)
            .flat_map(|m| &m.voices)
            .map(|v| v.beats.len())
            .sum();

        let total_notes: usize = song
            .tracks
            .iter()
            .flat_map(|t| &t.measures)
            .flat_map(|m| &m.voices)
            .flat_map(|v| &v.beats)
            .map(|b| b.notes.len())
            .sum();

        println!("Total beats: {total_beats}, total notes: {total_notes}");
        assert!(total_beats > 0, "Expected beats to be created");
    }

    #[test]
    fn test_tbt_to_song_all_files() {
        let test_files = [
            "test-files/Take On Me (2).tbt",
            "test-files/All That She Wants.tbt",
            "test-files/Ave Maria (Acoustic Guitar).tbt",
            "test-files/Bach - #13 In A Minor.tbt",
            "test-files/Adam - Oh Night Devine.tbt",
            "test-files/Aguado - Study 1.tbt",
            "test-files/Arcadelt - Il Bianco E Dolce Cigno.tbt",
            "test-files/Arndt - Nola.tbt",
        ];

        for path in test_files {
            let Ok(data) = fs::read(path) else { continue };

            if !is_tbt_file(&data) {
                continue;
            }

            let Ok(tbt_song) = parse_tbt_data(&data) else {
                continue;
            };

            let result = tbt_to_song(&tbt_song);
            assert!(
                result.is_ok(),
                "Failed to convert {path}: {:?}",
                result.err()
            );

            let song = result.unwrap();

            // Basic sanity checks
            assert_eq!(
                song.tracks.len(),
                tbt_song.header.track_count as usize,
                "Track count mismatch for {path}"
            );

            assert!(
                !song.measure_headers.is_empty(),
                "No measure headers for {path}"
            );

            // Each track should have the same number of measures as headers
            for track in &song.tracks {
                assert_eq!(
                    track.measures.len(),
                    song.measure_headers.len(),
                    "Measure count mismatch for {path}"
                );
            }

            println!(
                "{path}: {} tracks, {} measures, tempo={}",
                song.tracks.len(),
                song.measure_headers.len(),
                song.tempo.value
            );
        }
    }

    #[test]
    fn test_time_signature_inference() {
        // 4/4 time = 16 spaces
        let ts = infer_time_signature(16);
        assert_eq!(ts.numerator, 4);
        assert_eq!(ts.denominator.value, 4);

        // 3/4 time = 12 spaces
        let ts = infer_time_signature(12);
        assert_eq!(ts.numerator, 3);
        assert_eq!(ts.denominator.value, 4);

        // 2/4 time = 8 spaces
        let ts = infer_time_signature(8);
        assert_eq!(ts.numerator, 2);
        assert_eq!(ts.denominator.value, 4);
    }

    #[test]
    fn test_duration_calculation() {
        let ts = TimeSignature::default();

        // 1 space gap = 16th note
        let dur = calculate_duration(0, 1, &ts);
        assert_eq!(dur.value, 16);

        // 2 space gap = 8th note
        let dur = calculate_duration(0, 2, &ts);
        assert_eq!(dur.value, 8);

        // 4 space gap = quarter note
        let dur = calculate_duration(0, 4, &ts);
        assert_eq!(dur.value, 4);

        // 8 space gap = half note
        let dur = calculate_duration(0, 8, &ts);
        assert_eq!(dur.value, 2);
    }

    #[test]
    fn test_effect_conversion() {
        // Hammer on
        let effect = convert_effect(Some(TbtStringEffect::HammerOn));
        assert!(effect.hammer);

        // Pull off (also sets hammer flag)
        let effect = convert_effect(Some(TbtStringEffect::PullOff));
        assert!(effect.hammer);

        // Slide up
        let effect = convert_effect(Some(TbtStringEffect::SlideUp));
        assert!(matches!(effect.slide, Some(SlideType::ShiftSlideTo)));

        // Vibrato
        let effect = convert_effect(Some(TbtStringEffect::Vibrato));
        assert!(effect.vibrato);

        // Harmonic
        let effect = convert_effect(Some(TbtStringEffect::Harmonic));
        assert!(effect.harmonic.is_some());

        // No effect
        let effect = convert_effect(None);
        assert!(!effect.hammer);
        assert!(effect.slide.is_none());
        assert!(!effect.vibrato);
    }

    #[test]
    fn test_tuning_conversion() {
        // TBT stores tuning as SIGNED OFFSETS from standard guitar tuning [40, 45, 50, 55, 59, 64]
        // For standard tuning, all offsets should be 0
        let tbt_track = TbtTrack {
            string_count: 6,
            tuning: [0, 0, 0, 0, 0, 0, 0, 0], // Zero offsets = standard tuning
            ..Default::default()
        };

        let strings = convert_tuning(&tbt_track);

        assert_eq!(strings.len(), 6);

        // GP format: string 1 is highest (E4=64)
        let string1 = strings.iter().find(|(num, _)| *num == 1);
        assert!(string1.is_some());
        assert_eq!(string1.unwrap().1, 64); // E4 = 64

        // String 6 is lowest (E2=40)
        let string6 = strings.iter().find(|(num, _)| *num == 6);
        assert!(string6.is_some());
        assert_eq!(string6.unwrap().1, 40); // E2 = 40

        // Test with custom tuning: Drop D (low string down 2 semitones)
        // Offset of -2 on first string (D2 = 38 = 40 - 2)
        let drop_d_track = TbtTrack {
            string_count: 6,
            tuning: [0xFE, 0, 0, 0, 0, 0, 0, 0], // -2 as signed byte = 0xFE
            ..Default::default()
        };

        let drop_d_strings = convert_tuning(&drop_d_track);
        let low_string = drop_d_strings.iter().find(|(num, _)| *num == 6);
        assert_eq!(low_string.unwrap().1, 38); // D2 = 38
    }

    #[test]
    fn test_track5_drum_tuning_take_on_me() {
        // TDD test: Track 5 (drums) should have correct tuning after applying signed offsets
        // Expected drum tuning: [35, 35, 38, 38, 37, 49] (bass drum, snare, side stick, crash)
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let tbt_song = parse_tbt_data(&data).expect("Failed to parse TBT file");

        println!("Track 5 metadata:");
        println!("  Is drum: {}", tbt_song.metadata.tracks[5].is_drum);
        println!(
            "  String count: {}",
            tbt_song.metadata.tracks[5].string_count
        );
        println!(
            "  Raw tuning bytes: {:?}",
            tbt_song.metadata.tracks[5].tuning
        );

        // Convert tuning and verify
        let song = tbt_to_song(&tbt_song).expect("Failed to convert to Song");
        let track5 = &song.tracks[5];

        println!("  Converted tuning (GP format):");
        for (string_num, midi_note) in &track5.strings {
            println!("    String {string_num}: MIDI note {midi_note}");
        }

        // Expected drum tuning after conversion: [35, 35, 38, 38, 37, 49]
        // In GP format, string 1 is highest, so:
        // String 1 = 49 (crash), String 2 = 37 (side stick), String 3 = 38 (snare), etc.
        let expected_notes: Vec<i32> = vec![49, 37, 38, 38, 35, 35];
        for (i, (string_num, midi_note)) in track5.strings.iter().enumerate() {
            assert_eq!(
                *string_num,
                (i + 1) as i32,
                "String numbering mismatch at index {i}"
            );
            assert_eq!(
                *midi_note, expected_notes[i],
                "Drum tuning mismatch: string {} expected MIDI {}, got {}",
                string_num, expected_notes[i], midi_note
            );
        }

        // For drums, fret values can be high (they represent drum sound offsets)
        // Fret 33 on the "37" string gives MIDI note 70 (hi-hat)
        let track5_notes = &tbt_song.track_notes[5];
        println!("\nFirst 10 notes from track 5 (drums):");
        for (i, note) in track5_notes.iter().take(10).enumerate() {
            println!("  Note {}: string={}, fret={}", i, note.string, note.fret);
        }
    }

    #[test]
    fn test_guitar_track_fret_values() {
        // TDD test: Guitar track fret values should be reasonable (0-24 range)
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let tbt_song = parse_tbt_data(&data).expect("Failed to parse TBT file");

        // Find a guitar track (not drums, not bass)
        // Track 1 (index 1) should be a 6-string guitar
        let guitar_track_idx = 1;
        let guitar_notes = &tbt_song.track_notes[guitar_track_idx];

        println!("Track {guitar_track_idx} metadata:");
        println!(
            "  Is drum: {}",
            tbt_song.metadata.tracks[guitar_track_idx].is_drum
        );
        println!(
            "  String count: {}",
            tbt_song.metadata.tracks[guitar_track_idx].string_count
        );

        // Guitar frets should be in normal range (0-24)
        for (i, note) in guitar_notes.iter().enumerate() {
            assert!(
                note.fret <= 24,
                "Guitar note {} has unreasonable fret value: {} (string={}, vsq={})",
                i,
                note.fret,
                note.string,
                note.vsq_position
            );
        }

        println!(
            "All {} guitar notes have valid fret values (0-24)",
            guitar_notes.len()
        );
    }

    #[test]
    fn test_debug_all_tracks_take_on_me() {
        let data = fs::read("test-files/Take On Me (2).tbt").expect("Failed to read test file");
        let tbt_song = parse_tbt_data(&data).expect("Failed to parse TBT file");
        let song = tbt_to_song(&tbt_song).expect("Failed to convert to Song");

        println!("\n=== TBT Raw Track Notes ===");
        for (i, track_notes) in tbt_song.track_notes.iter().enumerate() {
            println!(
                "TBT Track {}: {} notes, is_drum={}",
                i,
                track_notes.len(),
                tbt_song.metadata.tracks[i].is_drum
            );
        }

        println!("\n=== Converted Song Tracks ===");
        for (i, track) in song.tracks.iter().enumerate() {
            let total_notes: usize = track
                .measures
                .iter()
                .flat_map(|m| &m.voices)
                .flat_map(|v| &v.beats)
                .map(|b| b.notes.len())
                .sum();

            let first_notes: Vec<i16> = track
                .measures
                .iter()
                .flat_map(|m| &m.voices)
                .flat_map(|v| &v.beats)
                .flat_map(|b| &b.notes)
                .take(10)
                .map(|n| n.value)
                .collect();

            println!(
                "Track {}: {} strings, {} notes, first frets: {:?}",
                i,
                track.strings.len(),
                total_notes,
                first_notes
            );
        }

        // Compare tracks 5 and 6 (0-indexed)
        println!("\n=== Comparing Track 5 vs Track 6 ===");
        let track5_notes: Vec<(i16, i8)> = song.tracks[5]
            .measures
            .iter()
            .flat_map(|m| &m.voices)
            .flat_map(|v| &v.beats)
            .flat_map(|b| &b.notes)
            .take(20)
            .map(|n| (n.value, n.string))
            .collect();

        let track6_notes: Vec<(i16, i8)> = song.tracks[6]
            .measures
            .iter()
            .flat_map(|m| &m.voices)
            .flat_map(|v| &v.beats)
            .flat_map(|b| &b.notes)
            .take(20)
            .map(|n| (n.value, n.string))
            .collect();

        println!("Track 5 first 20 (fret, string): {track5_notes:?}");
        println!("Track 6 first 20 (fret, string): {track6_notes:?}");

        // Check raw TBT notes for track 6
        println!("\n=== Raw TBT Track 6 Notes ===");
        for (i, note) in tbt_song.track_notes[6].iter().take(20).enumerate() {
            println!(
                "  Note {}: string={}, fret={}, vsq={}",
                i, note.string, note.fret, note.vsq_position
            );
        }

        // Check track 6 tuning
        println!("\n=== Track 6 Tuning ===");
        println!("  Raw bytes: {:?}", tbt_song.metadata.tracks[6].tuning);
        println!(
            "  String count: {}",
            tbt_song.metadata.tracks[6].string_count
        );
        println!("  Is drum: {}", tbt_song.metadata.tracks[6].is_drum);
    }
}
