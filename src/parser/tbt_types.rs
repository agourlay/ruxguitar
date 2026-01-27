//! TBT (TabIt) file format data structures
//!
//! Reference: <https://github.com/bostick/tabit-file-format>

/// TBT file format versions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbtVersion {
    /// Version 0x6f (111) - earliest supported version
    V0x6f,
    /// Version 0x70 (112) - adds alternate time regions, space count per track
    V0x70,
    /// Version 0x71 (113) - adds modulation and pitch bend blocks
    V0x71,
    /// Version 0x72 (114) - latest version
    V0x72,
}

impl TbtVersion {
    /// Parse version byte into enum variant
    pub const fn from_byte(byte: u8) -> Option<TbtVersion> {
        match byte {
            0x6f => Some(TbtVersion::V0x6f),
            0x70 => Some(TbtVersion::V0x70),
            0x71 => Some(TbtVersion::V0x71),
            0x72 => Some(TbtVersion::V0x72),
            _ => None,
        }
    }

    /// Check if version supports alternate time regions
    #[allow(dead_code)]
    pub const fn has_alternate_time_regions(&self) -> bool {
        matches!(self, TbtVersion::V0x70 | TbtVersion::V0x71 | TbtVersion::V0x72)
    }

    /// Check if version has per-track space count
    pub const fn has_space_count_per_track(&self) -> bool {
        matches!(self, TbtVersion::V0x70 | TbtVersion::V0x71 | TbtVersion::V0x72)
    }

    /// Check if version has modulation and pitch bend blocks
    pub const fn has_modulation_pitch_bend(&self) -> bool {
        matches!(self, TbtVersion::V0x71 | TbtVersion::V0x72)
    }

    /// Check if version has track effect changes in separate chunk
    pub const fn has_track_effect_changes_chunk(&self) -> bool {
        matches!(self, TbtVersion::V0x71 | TbtVersion::V0x72)
    }
}

/// Feature flags parsed from the feature bitfield in header
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TbtFeatures {
    /// Bit 4: Has alternate time regions (triplets, etc.)
    pub has_alternate_time_regions: bool,
    /// Bit 3: Always set in 0x6e+
    pub feature_bit_3: bool,
}

impl TbtFeatures {
    /// Parse feature bitfield byte
    pub const fn from_byte(byte: u8) -> Self {
        TbtFeatures {
            has_alternate_time_regions: (byte & 0x10) != 0,
            feature_bit_3: (byte & 0x08) != 0,
        }
    }
}

/// 64-byte TBT file header
#[derive(Debug, Clone, PartialEq)]
pub struct TbtHeader {
    /// File version (0x6f, 0x70, 0x71, or 0x72)
    pub version: TbtVersion,
    /// Primary tempo value (BPM)
    pub tempo1: u8,
    /// Number of tracks in the file
    pub track_count: u8,
    /// Version string (e.g., "2.03")
    pub version_string: String,
    /// Feature flags
    pub features: TbtFeatures,
    /// Number of bars (measures)
    pub bar_count: u16,
    /// Total space count (1 space = 1/16th note)
    pub space_count: u16,
    /// Index of last non-empty space
    pub last_non_empty_space: u16,
    /// Secondary tempo value (may differ from tempo1)
    pub tempo2: u16,
    /// Length of compressed metadata section
    pub compressed_metadata_len: u32,
    /// CRC32 of body section
    pub crc32_body: u32,
    /// Total byte count of file
    pub total_byte_count: u32,
    /// CRC32 of header (first 60 bytes)
    pub crc32_header: u32,
}

/// Track configuration data from metadata section
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TbtTrack {
    /// Track index (0-based)
    pub index: u8,
    /// Space count for this track (version >= 0x70, 4 bytes)
    pub space_count: Option<u32>,
    /// Number of strings (typically 6 for guitar)
    pub string_count: u8,
    /// MIDI program number for clean guitar sound
    pub clean_guitar: u8,
    /// MIDI program number for muted guitar sound
    pub muted_guitar: u8,
    /// Track volume (0-127)
    pub volume: u8,
    /// Modulation (version >= 0x71)
    pub modulation: Option<u8>,
    /// Pitch bend (version >= 0x71, 2 bytes)
    pub pitch_bend: Option<u16>,
    /// Transpose in half steps
    pub transpose_half_steps: i8,
    /// MIDI bank number
    pub midi_bank: u8,
    /// Reverb level
    pub reverb: u8,
    /// Chorus level
    pub chorus: u8,
    /// Pan position
    pub pan: u8,
    /// Highest note value
    pub highest_note: u8,
    /// Display MIDI note numbers flag
    pub display_midi_note_numbers: bool,
    /// MIDI channel (0-15)
    pub midi_channel: u8,
    /// Top line text enabled
    pub top_line_text: bool,
    /// Bottom line text enabled
    pub bottom_line_text: bool,
    /// String tunings (8 bytes, from low to high string)
    pub tuning: [u8; 8],
    /// Is this a drum track
    pub is_drum: bool,
}

/// Song information strings from metadata section
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TbtSongInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub transcribed_by: String,
    pub comment: String,
}

/// Parsed metadata section
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TbtMetadata {
    /// Track configuration data
    pub tracks: Vec<TbtTrack>,
    /// Song information
    pub song_info: TbtSongInfo,
}

/// Bar line type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbtBarType {
    /// Single bar line
    Single,
    /// Double bar line
    Double,
    /// Open repeat (|:)
    OpenRepeat,
    /// Close repeat (:|)
    CloseRepeat,
    /// Both open and close repeat (:|:)
    OpenCloseRepeat,
}

/// Bar line information
#[derive(Debug, Clone, PartialEq)]
pub struct TbtBarLine {
    /// Space position of the bar line
    pub space: u16,
    /// Type of bar line
    pub bar_type: TbtBarType,
    /// Repeat count (for close repeats)
    pub repeat_count: u8,
}

/// String effect for a note
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbtStringEffect {
    /// '/' - Slide up
    SlideUp,
    /// '\' - Slide down
    SlideDown,
    /// '^' - Bend up
    BendUp,
    /// 'b' - Bend
    Bend,
    /// 'h' - Hammer on
    HammerOn,
    /// 'p' - Pull off
    PullOff,
    /// 'r' - Release bend
    ReleaseBend,
    /// '~' - Vibrato
    Vibrato,
    /// '<' - Natural harmonic
    Harmonic,
    /// '{' - Tremolo picking
    Tremolo,
    /// '(' - Ghost note / soft
    GhostNote,
    /// 't' - Tap
    Tap,
    /// 's' - Slap
    Slap,
    /// 'w' - Whammy bar
    Whammy,
}

impl TbtStringEffect {
    /// Parse effect byte into enum variant
    pub const fn from_byte(byte: u8) -> Option<TbtStringEffect> {
        match byte {
            0x2f => Some(TbtStringEffect::SlideUp),    // '/'
            0x5c => Some(TbtStringEffect::SlideDown),  // '\'
            0x5e => Some(TbtStringEffect::BendUp),     // '^'
            0x62 => Some(TbtStringEffect::Bend),       // 'b'
            0x68 => Some(TbtStringEffect::HammerOn),   // 'h'
            0x70 => Some(TbtStringEffect::PullOff),    // 'p'
            0x72 => Some(TbtStringEffect::ReleaseBend), // 'r'
            0x7e => Some(TbtStringEffect::Vibrato),    // '~'
            0x3c => Some(TbtStringEffect::Harmonic),   // '<'
            0x7b => Some(TbtStringEffect::Tremolo),    // '{'
            0x28 => Some(TbtStringEffect::GhostNote),  // '('
            0x74 => Some(TbtStringEffect::Tap),        // 't'
            0x73 => Some(TbtStringEffect::Slap),       // 's'
            0x77 => Some(TbtStringEffect::Whammy),     // 'w'
            _ => None,
        }
    }
}

/// A note in the TBT format
#[derive(Debug, Clone, PartialEq)]
pub struct TbtNote {
    /// Position in vsq units (1 vsq = 1/20 of a space)
    pub vsq_position: u32,
    /// String number (0 = lowest/bass string)
    pub string: u8,
    /// Fret number (after removing 0x80 flag)
    pub fret: u8,
    /// Is this a muted note (0x11)
    pub is_muted: bool,
    /// Is this a stop/rest (0x12)
    pub is_stop: bool,
    /// Effect applied to this note
    pub effect: Option<TbtStringEffect>,
}

/// Note value constants
pub const NOTE_MUTED: u8 = 0x11;
pub const NOTE_STOP: u8 = 0x12;
pub const NOTE_FRET_FLAG: u8 = 0x80;

/// Alternate time region (e.g., triplets)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TbtAlternateTime {
    /// Position in dsq units (1 dsq = 1/2 of a space)
    pub dsq_position: u32,
    /// Denominator (e.g., 2 in 3:2)
    pub denominator: u8,
    /// Numerator (e.g., 3 in 3:2)
    pub numerator: u8,
}

/// Track effect change types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbtEffectChangeType {
    Stroke = 1,
    Tempo = 2,
    Instrument = 3,
    Volume = 4,
    Pan = 5,
    Chorus = 6,
    Reverb = 7,
    Modulation = 8,
    PitchBend = 9,
}

impl TbtEffectChangeType {
    /// Parse effect type from byte
    pub const fn from_byte(byte: u8) -> Option<TbtEffectChangeType> {
        match byte {
            1 => Some(TbtEffectChangeType::Stroke),
            2 => Some(TbtEffectChangeType::Tempo),
            3 => Some(TbtEffectChangeType::Instrument),
            4 => Some(TbtEffectChangeType::Volume),
            5 => Some(TbtEffectChangeType::Pan),
            6 => Some(TbtEffectChangeType::Chorus),
            7 => Some(TbtEffectChangeType::Reverb),
            8 => Some(TbtEffectChangeType::Modulation),
            9 => Some(TbtEffectChangeType::PitchBend),
            _ => None,
        }
    }
}

/// Track effect change event
#[derive(Debug, Clone, PartialEq)]
pub struct TbtEffectChange {
    /// Space position
    pub space: u32,
    /// Effect type
    pub effect_type: TbtEffectChangeType,
    /// Effect value (interpretation depends on effect_type)
    pub value: u16,
}

/// Fully parsed TBT song before conversion to GP format
#[derive(Debug, Clone, PartialEq)]
pub struct TbtSong {
    /// Header information
    pub header: TbtHeader,
    /// Metadata (track settings, song info)
    pub metadata: TbtMetadata,
    /// Bar lines with positions
    pub bar_lines: Vec<TbtBarLine>,
    /// Notes organized by track, each track contains notes with vsq positions
    pub track_notes: Vec<Vec<TbtNote>>,
    /// Alternate time regions per track
    pub alternate_times: Vec<Vec<TbtAlternateTime>>,
    /// Track effect changes per track (version >= 0x71)
    pub track_effect_changes: Vec<Vec<TbtEffectChange>>,
}
