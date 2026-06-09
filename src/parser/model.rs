//! Data model for parsed Guitar Pro songs (version-agnostic).

pub const MAX_VOICES: u32 = 2;

pub const QUARTER_TIME: u32 = 960;
pub const QUARTER: u16 = 4;

pub const DURATION_EIGHTH: u8 = 8;
pub const DURATION_SIXTEENTH: u8 = 16;
pub const DURATION_THIRTY_SECOND: u8 = 32;
pub const DURATION_SIXTY_FOURTH: u8 = 64;

pub const BEND_EFFECT_MAX_POSITION_LENGTH: f32 = 12.0;

pub const SEMITONE_LENGTH: f32 = 1.0;
pub const GP_BEND_SEMITONE: f32 = 25.0;
pub const GP_BEND_POSITION: f32 = 60.0;

pub const SHARP_NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub const DEFAULT_PERCUSSION_BANK: u8 = 128;

pub const DEFAULT_BANK: u8 = 0;

pub const MIN_VELOCITY: i16 = 15;
pub const VELOCITY_INCREMENT: i16 = 16;
pub const DEFAULT_VELOCITY: i16 = MIN_VELOCITY + VELOCITY_INCREMENT * 5; // FORTE

/// Convert Guitar Pro dynamic value to raw MIDI velocity
pub const fn convert_velocity(v: i16) -> i16 {
    MIN_VELOCITY + (VELOCITY_INCREMENT * v) - VELOCITY_INCREMENT
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Default)]
pub enum GpVersion {
    #[default]
    GP3,
    GP4,
    GP4_06,
    GP5,
    GP5_10,
    GP6,
    GP7,
}

#[derive(Debug, PartialEq, Default)]
pub struct Song {
    pub version: GpVersion,
    pub song_info: SongInfo,
    pub triplet_feel: Option<bool>, // only < GP5
    pub lyrics: Option<Lyrics>,
    pub page_setup: Option<PageSetup>,
    pub tempo: Tempo,
    pub hide_tempo: Option<bool>,
    pub key_signature: i8,
    pub octave: Option<i8>,
    pub midi_channels: Vec<MidiChannel>,
    pub measure_headers: Vec<MeasureHeader>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MidiChannel {
    pub channel_id: u8,
    pub effect_channel_id: u8,
    pub instrument: i32,
    pub volume: i8,
    pub balance: i8,
    pub chorus: i8,
    pub reverb: i8,
    pub phaser: i8,
    pub tremolo: i8,
    pub bank: u8,
}

impl MidiChannel {
    pub const fn is_percussion(&self) -> bool {
        self.bank == DEFAULT_PERCUSSION_BANK
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Padding {
    pub right: i32,
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, PartialEq)]
pub struct PageSetup {
    pub page_size: Point,
    pub page_margin: Padding,
    pub score_size_proportion: f32,
    pub header_and_footer: i16,
    pub title: String,
    pub subtitle: String,
    pub artist: String,
    pub album: String,
    pub words: String,
    pub music: String,
    pub word_and_music: String,
    pub copyright: String,
    pub page_number: String,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Lyrics {
    pub track_choice: i32,
    pub lines: Vec<(i32, String)>,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub struct SongInfo {
    pub name: String,
    pub subtitle: String,
    pub artist: String,
    pub album: String,
    pub author: String,
    pub words: Option<String>,
    pub copyright: String,
    pub writer: String,
    pub instructions: String,
    pub notices: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Marker {
    pub title: String,
    pub color: i32,
}

pub const KEY_SIGNATURES: [&str; 34] = [
    "F♭ major",
    "C♭ major",
    "G♭ major",
    "D♭ major",
    "A♭ major",
    "E♭ major",
    "B♭ major",
    "F major",
    "C major",
    "G major",
    "D major",
    "A major",
    "E major",
    "B major",
    "F# major",
    "C# major",
    "G# major",
    "D♭ minor",
    "A♭ minor",
    "E♭ minor",
    "B♭ minor",
    "F minor",
    "C minor",
    "G minor",
    "D minor",
    "A minor",
    "E minor",
    "B minor",
    "F# minor",
    "C# minor",
    "G# minor",
    "D# minor",
    "A# minor",
    "E# minor",
];

#[derive(Debug, PartialEq, Eq)]
pub struct KeySignature {
    pub key: i8,
    pub is_minor: bool,
}

impl KeySignature {
    pub const fn new(key: i8, is_minor: bool) -> Self {
        KeySignature { key, is_minor }
    }
}

impl std::fmt::Display for KeySignature {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let index: usize = if self.is_minor {
            (23i8 + self.key) as usize
        } else {
            (8i8 + self.key) as usize
        };
        write!(f, "{}", KEY_SIGNATURES[index])
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TripletFeel {
    None,
    Eighth,
    Sixteenth,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tempo {
    pub value: u32,
    pub name: Option<String>,
}

impl Tempo {
    pub const fn new(value: u32, name: Option<String>) -> Self {
        Tempo { value, name }
    }
}

impl Default for Tempo {
    fn default() -> Self {
        Tempo {
            value: 120,
            name: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct MeasureHeader {
    pub start: u32,
    pub time_signature: TimeSignature,
    pub tempo: Tempo,
    pub marker: Option<Marker>,
    pub repeat_open: bool,
    pub repeat_alternative: u8,
    pub repeat_close: i8,
    pub triplet_feel: TripletFeel,
    pub key_signature: KeySignature,
}

impl Default for MeasureHeader {
    fn default() -> Self {
        MeasureHeader {
            start: QUARTER_TIME,
            time_signature: TimeSignature::default(),
            tempo: Tempo::default(),
            marker: None,
            repeat_open: false,
            repeat_alternative: 0,
            repeat_close: 0,
            triplet_feel: TripletFeel::None,
            key_signature: KeySignature::new(0, false),
        }
    }
}

impl MeasureHeader {
    pub fn length(&self) -> u32 {
        let numerator = u32::from(self.time_signature.numerator);
        let denominator = self.time_signature.denominator.time();
        numerator * denominator
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: Duration,
}

impl Default for TimeSignature {
    fn default() -> Self {
        TimeSignature {
            numerator: 4,
            denominator: Duration::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duration {
    pub value: u16,
    pub dotted: bool,
    pub double_dotted: bool,
    pub tuplet_enters: u8,
    pub tuplet_times: u8,
}

impl Default for Duration {
    fn default() -> Self {
        Duration {
            value: QUARTER,
            dotted: false,
            double_dotted: false,
            tuplet_enters: 1,
            tuplet_times: 1,
        }
    }
}

impl Duration {
    pub fn convert_time(&self, time: u32) -> u32 {
        log::debug!(
            "time:{} tuplet_times:{} tuplet_enters:{}",
            time,
            self.tuplet_times,
            self.tuplet_enters
        );
        time * u32::from(self.tuplet_times) / u32::from(self.tuplet_enters)
    }

    pub fn time(&self) -> u32 {
        let mut time = QUARTER_TIME as f32 * (4.0 / f32::from(self.value));
        if self.dotted {
            time += time / 2.0;
        } else if self.double_dotted {
            time += (time / 4.0) * 3.0;
        }
        self.convert_time(time as u32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BendPoint {
    pub position: u8,
    pub value: i8,
}

impl BendPoint {
    pub fn get_time(&self, duration: u32) -> u32 {
        let time = duration as f32 * f32::from(self.position) / BEND_EFFECT_MAX_POSITION_LENGTH;
        time as u32
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BendEffect {
    pub points: Vec<BendPoint>,
}

impl BendEffect {
    pub fn direction(&self) -> isize {
        if self.points.len() < 2 {
            return 0;
        }
        let first = self.points[0].value;
        for p in &self.points[1..] {
            match first.cmp(&p.value) {
                std::cmp::Ordering::Greater => return -1,
                std::cmp::Ordering::Less => return 1,
                std::cmp::Ordering::Equal => (),
            }
        }
        0
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TremoloBarEffect {
    pub points: Vec<BendPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraceEffect {
    pub duration: u8,
    pub fret: i8,
    pub is_dead: bool,
    pub is_on_beat: bool,
    pub transition: GraceEffectTransition,
    pub velocity: i16,
}

impl GraceEffect {
    pub fn duration_time(&self) -> f32 {
        (QUARTER_TIME as f32 / 16.00) * f32::from(self.duration)
    }
}

impl Default for GraceEffect {
    fn default() -> Self {
        GraceEffect {
            duration: 0,
            fret: 0,
            is_dead: false,
            is_on_beat: false,
            transition: GraceEffectTransition::None,
            velocity: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraceEffectTransition {
    /// No transition
    None = 0,
    /// Slide from the grace note to the real one.
    Slide,
    /// Perform a bend from the grace note to the real one.
    Bend,
    /// Perform a hammer on.
    Hammer,
}

impl GraceEffectTransition {
    pub fn get_grace_effect_transition(value: i8) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Slide,
            2 => Self::Bend,
            3 => Self::Hammer,
            _ => panic!("Cannot get transition for the grace effect"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PitchClass {
    pub note: String,
    pub just: i8,
    /// flat (-1), none (0) or sharp (1).
    pub accidental: i8,
    pub value: i8,
    pub sharp: bool,
}

impl PitchClass {
    pub fn from(just: i8, accidental: Option<i8>, sharp: Option<bool>) -> PitchClass {
        let mut p = PitchClass {
            just,
            accidental: 0,
            value: -1,
            sharp: true,
            note: String::with_capacity(2),
        };
        let pitch: i8;
        let accidental2: i8;
        if let Some(a) = accidental {
            pitch = p.just;
            accidental2 = a;
        } else {
            let value = p.just % 12;
            p.note = if value >= 0 {
                String::from(SHARP_NOTES[value as usize])
            } else {
                String::from(SHARP_NOTES[(12 + value) as usize])
            };
            if p.note.ends_with('b') {
                accidental2 = -1;
                p.sharp = false;
            } else if p.note.ends_with('#') {
                accidental2 = 1;
            } else {
                accidental2 = 0;
            }
            pitch = value - accidental2;
        }
        p.just = pitch % 12;
        p.accidental = accidental2;
        p.value = p.just + accidental2;
        if sharp.is_none() {
            p.sharp = p.accidental >= 0;
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarmonicType {
    Natural,
    Artificial,
    Tapped,
    Pinch,
    Semi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Octave {
    None,
    Ottava,
    Quindicesima,
    OttavaBassa,
    QuindicesimaBassa,
}

impl Octave {
    pub fn get_octave(value: u8) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Ottava,
            2 => Self::Quindicesima,
            3 => Self::OttavaBassa,
            4 => Self::QuindicesimaBassa,
            _ => panic!("Cannot get octave value"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarmonicEffect {
    pub kind: HarmonicType,
    // artificial harmonic
    pub pitch: Option<PitchClass>,
    pub octave: Option<Octave>,
    // tapped harmonic
    pub right_hand_fret: Option<i8>,
}

impl Default for HarmonicEffect {
    fn default() -> Self {
        HarmonicEffect {
            kind: HarmonicType::Natural,
            pitch: None,
            octave: None,
            right_hand_fret: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlideType {
    IntoFromAbove,
    IntoFromBelow,
    ShiftSlideTo,
    LegatoSlideTo,
    OutDownwards,
    OutUpWards,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrillEffect {
    pub fret: i8,
    pub duration: Duration,
}

impl TrillEffect {
    pub fn from_trill_period(period: i8) -> u16 {
        match period {
            1 => u16::from(DURATION_SIXTEENTH),
            2 => u16::from(DURATION_THIRTY_SECOND),
            3 => u16::from(DURATION_SIXTY_FOURTH),
            other => panic!("Cannot get trill period - got {other}"),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TremoloPickingEffect {
    pub duration: Duration,
}

impl TremoloPickingEffect {
    pub fn from_tremolo_value(value: i8) -> u16 {
        match value {
            1 => u16::from(DURATION_EIGHTH),
            2 => u16::from(DURATION_SIXTEENTH),
            3 => u16::from(DURATION_THIRTY_SECOND),
            other => panic!("Cannot get tremolo value - got {other}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum NoteType {
    Rest,
    Normal,
    Tie,
    Dead,
    Unknown(u8),
}

impl NoteType {
    pub const fn get_note_type(value: u8) -> Self {
        match value {
            0 => Self::Rest,
            1 => Self::Normal,
            2 => Self::Tie,
            3 => Self::Dead,
            _ => Self::Unknown(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEffect {
    pub accentuated_note: bool,
    pub bend: Option<BendEffect>,
    pub ghost_note: bool,
    pub grace: Option<GraceEffect>,
    pub hammer: bool,
    pub harmonic: Option<HarmonicEffect>,
    pub heavy_accentuated_note: bool,
    pub let_ring: bool,
    pub palm_mute: bool,
    pub slide: Option<SlideType>,
    pub staccato: bool,
    pub tremolo_picking: Option<TremoloPickingEffect>,
    pub trill: Option<TrillEffect>,
    pub fade_in: bool,
    pub vibrato: bool,
    pub slap: SlapEffect,
    pub tremolo_bar: Option<TremoloBarEffect>,
}

impl Default for NoteEffect {
    fn default() -> Self {
        NoteEffect {
            accentuated_note: false,
            bend: None,
            ghost_note: false,
            grace: None,
            hammer: false,
            harmonic: None,
            heavy_accentuated_note: false,
            let_ring: false,
            palm_mute: false,
            slide: None,
            staccato: false,
            tremolo_picking: None,
            trill: None,
            fade_in: false,
            vibrato: false,
            slap: SlapEffect::None,
            tremolo_bar: None,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Chord {
    pub length: u8,
    pub sharp: Option<bool>,
    pub root: Option<PitchClass>,
    pub bass: Option<PitchClass>,
    pub add: Option<bool>,
    pub name: String,
    pub first_fret: Option<u32>,
    pub strings: Vec<i8>,
    pub omissions: Vec<bool>,
    pub show: Option<bool>,
    pub new_format: Option<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BeatStrokeDirection {
    None,
    Up,
    Down,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BeatStroke {
    pub direction: BeatStrokeDirection,
    pub value: u16,
}

impl BeatStroke {
    pub fn is_empty(&self) -> bool {
        self.direction == BeatStrokeDirection::None || self.value == 0
    }

    // A small time increment that depends on note duration and stroke intensity.
    pub fn increment_for_duration(&self, beat_duration: u32) -> u32 {
        if self.value == 0 {
            return 0;
        }
        // stroke speed is based on the smallest rhythmic value
        let duration = beat_duration.min(QUARTER_TIME);
        ((duration as f32 / 8.0) * (4.0 / f32::from(self.value))).round() as u32
    }
}

impl Default for BeatStroke {
    fn default() -> Self {
        BeatStroke {
            direction: BeatStrokeDirection::None,
            value: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlapEffect {
    None,
    Tapping,
    Slapping,
    Popping,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BeatEffects {
    pub stroke: BeatStroke,
    pub chord: Option<Chord>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Note {
    pub value: i16,
    pub velocity: i16,
    pub string: i8,
    pub effect: NoteEffect,
    pub swap_accidentals: bool,
    pub kind: NoteType,
    tuplet: Option<i8>,
}

impl Note {
    pub const fn new(note_effect: NoteEffect) -> Self {
        Note {
            value: 0,
            velocity: DEFAULT_VELOCITY,
            string: 1,
            effect: note_effect,
            swap_accidentals: false,
            kind: NoteType::Rest,
            tuplet: None,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Beat {
    pub notes: Vec<Note>,
    pub duration: Duration,
    pub empty: bool,
    pub text: String,
    pub start: u32,
    pub effect: BeatEffects,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Voice {
    pub measure_index: i16,
    pub beats: Vec<Beat>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Measure {
    pub key_signature: KeySignature,
    pub time_signature: TimeSignature,
    pub track_index: usize,
    pub header_index: usize,
    pub voices: Vec<Voice>,
}

impl Default for Measure {
    fn default() -> Self {
        Measure {
            key_signature: KeySignature::new(0, false),
            time_signature: TimeSignature::default(),
            track_index: 0,
            header_index: 0,
            voices: vec![],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Track {
    pub number: i32,
    pub offset: i32,
    pub channel_id: u8,
    pub solo: bool,
    pub mute: bool,
    pub visible: bool,
    pub name: String,
    pub strings: Vec<(i32, i32)>,
    pub color: i32,
    pub midi_port: u8,
    pub fret_count: u8,
    pub measures: Vec<Measure>,
}

impl Default for Track {
    fn default() -> Self {
        Track {
            number: 1,
            offset: 0,
            channel_id: 0,
            solo: false,
            mute: false,
            visible: true,
            name: String::new(),
            strings: vec![],
            color: 0,
            midi_port: 0,
            fret_count: 24,
            measures: vec![],
        }
    }
}
