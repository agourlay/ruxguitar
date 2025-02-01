use crate::parser::music_parser::MusicParser;
use crate::parser::primitive_parser::{
    parse_bool, parse_byte, parse_byte_size_string, parse_int, parse_int_byte_sized_string,
    parse_int_sized_string, parse_short, parse_signed_byte, skip,
};
use crate::RuxError;
use nom::bytes::complete::take;
use nom::combinator::{cond, flat_map, map};
use nom::multi::count;
use nom::sequence::preceded;
use nom::IResult;
use nom::Parser;
use std::fmt::Debug;

// GP4 docs at <https://dguitar.sourceforge.net/GP4format.html>
// GP5 docs thanks to Tuxguitar and <https://github.com/slundi/guitarpro> for the help

pub const MAX_VOICES: usize = 2;

pub const QUARTER_TIME: i64 = 960;
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
pub fn convert_velocity(v: i16) -> i16 {
    MIN_VELOCITY + (VELOCITY_INCREMENT * v) - VELOCITY_INCREMENT
}

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub enum GpVersion {
    #[default]
    GP3,
    GP4,
    GP4_06,
    GP5,
    GP5_10,
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
    pub octave: Option<i32>,
    pub midi_channels: Vec<MidiChannel>,
    pub measure_headers: Vec<MeasureHeader>,
    pub tracks: Vec<Track>,
}

impl Song {
    pub fn get_measure_beat_for_tick(&self, track_id: usize, tick: usize) -> (usize, usize) {
        let mut measure_index = 0;
        let mut beat_index = 0;
        // TODO could pre-compute boundaries with btree map
        for (i, measure) in self.measure_headers.iter().enumerate() {
            if measure.start > tick as i64 {
                break;
            } else {
                measure_index = i;
            }
        }
        let voice = &self.tracks[track_id].measures[measure_index].voices[0];
        for (j, beat) in voice.beats.iter().enumerate() {
            if beat.start > tick as i64 {
                break;
            } else {
                beat_index = j;
            }
        }
        (measure_index, beat_index)
    }
}

#[derive(Debug, PartialEq)]
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
    pub fn is_percussion(&self) -> bool {
        self.bank == DEFAULT_PERCUSSION_BANK
    }
}

#[derive(Debug, PartialEq)]
pub struct Padding {
    pub right: i32,
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
}
#[derive(Debug, PartialEq)]
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
#[derive(Debug, PartialEq)]
pub struct Lyrics {
    pub track_choice: i32,
    pub lines: Vec<(i32, String)>,
}

#[derive(Debug, PartialEq, Default)]
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

#[derive(Debug, PartialEq)]
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
    pub fn new(key: i8, is_minor: bool) -> Self {
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

#[derive(Debug, PartialEq, Eq)]
pub enum TripletFeel {
    None,
    Eighth,
    Sixteenth,
}

#[derive(Debug, PartialEq)]
pub struct Tempo {
    pub value: i32,
    pub name: Option<String>,
}

impl Tempo {
    fn new(value: i32, name: Option<String>) -> Self {
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

#[derive(Debug, PartialEq)]
pub struct MeasureHeader {
    pub start: i64,
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
    pub fn length(&self) -> i64 {
        let numerator = self.time_signature.numerator as i64;
        let denominator = self.time_signature.denominator.time() as i64;
        numerator * denominator
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TimeSignature {
    pub numerator: i8,
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
        time * self.tuplet_times as u32 / self.tuplet_enters as u32
    }

    pub fn time(&self) -> u32 {
        let mut time = QUARTER_TIME as f64 * (4.0 / self.value as f64);
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
    pub fn get_time(&self, duration: usize) -> usize {
        let time = duration as f32 * self.position as f32 / BEND_EFFECT_MAX_POSITION_LENGTH;
        time as usize
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BendEffect {
    pub points: Vec<BendPoint>,
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
        (QUARTER_TIME as f32 / 16.00) * self.duration as f32
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
    pub fn get_grace_effect_transition(value: i8) -> GraceEffectTransition {
        match value {
            0 => GraceEffectTransition::None,
            1 => GraceEffectTransition::Slide,
            2 => GraceEffectTransition::Bend,
            3 => GraceEffectTransition::Hammer,
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
    pub fn get_octave(value: u8) -> Octave {
        match value {
            0 => Octave::None,
            1 => Octave::Ottava,
            2 => Octave::Quindicesima,
            3 => Octave::OttavaBassa,
            4 => Octave::QuindicesimaBassa,
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
    fn from_trill_period(period: i8) -> u16 {
        match period {
            1 => DURATION_SIXTEENTH as u16,
            2 => DURATION_THIRTY_SECOND as u16,
            3 => DURATION_SIXTY_FOURTH as u16,
            _ => panic!("Cannot get trill period"),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TremoloPickingEffect {
    pub duration: Duration,
}

impl TremoloPickingEffect {
    fn from_tremolo_value(value: i8) -> u16 {
        match value {
            1 => DURATION_EIGHTH as u16,
            3 => DURATION_SIXTEENTH as u16,
            2 => DURATION_THIRTY_SECOND as u16,
            _ => panic!("Cannot get tremolo value"),
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
    pub fn get_note_type(value: u8) -> NoteType {
        match value {
            0 => NoteType::Rest,
            1 => NoteType::Normal,
            2 => NoteType::Tie,
            3 => NoteType::Dead,
            _ => NoteType::Unknown(value),
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
    direction: BeatStrokeDirection,
    value: u16,
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

#[derive(Debug, PartialEq)]
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
    pub fn new(note_effect: NoteEffect) -> Self {
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

#[derive(Debug, Default, PartialEq)]
pub struct Beat {
    pub notes: Vec<Note>,
    pub duration: Duration,
    pub empty: bool,
    pub text: String,
    pub start: i64,
    pub effect: BeatEffects,
}

#[derive(Debug, Default, PartialEq)]
pub struct Voice {
    pub measure_index: i16,
    pub beats: Vec<Beat>,
}

#[derive(Debug, PartialEq)]
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

#[derive(Debug, PartialEq)]
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

pub fn parse_chord(
    string_count: u8,
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], Chord> {
    move |i| {
        log::debug!("Parsing chords for {} strings", string_count);
        let mut i = i;
        let mut chord = Chord {
            strings: vec![-1; string_count.into()],
            ..Default::default()
        };
        let (inner, chord_gp4_header) = parse_byte(i)?;
        i = inner;

        // chord header defines the version as well
        if (chord_gp4_header & 0x01) == 0 {
            debug_assert!(
                version < GpVersion::GP5,
                "Chord header is GP4 but version is {:?}",
                version
            );
            log::debug!("Parsing GP4 chord");
            let (inner, chord_name) = parse_int_byte_sized_string(i)?;
            i = inner;
            chord.name = chord_name;
            let (inner, first_fret) = parse_int(i)?;
            i = inner;
            chord.first_fret = Some(first_fret as u32);
            if first_fret != 0 {
                for c in 0..6 {
                    let (inner, fret) = parse_int(i)?;
                    if c < string_count {
                        chord.strings[c as usize] = fret as i8;
                    }
                    i = inner;
                }
            }
        } else {
            i = skip(i, 16);
            let (inner, chord_name) = parse_byte_size_string(21)(i)?;
            i = inner;
            chord.name = chord_name;
            i = skip(i, 4);
            let (inner, first_fret) = parse_int(i)?;
            i = inner;
            chord.first_fret = Some(first_fret as u32);
            for c in 0..7 {
                let (inner, fret) = parse_int(i)?;
                if c < string_count {
                    chord.strings[c as usize] = fret as i8;
                }
                i = inner;
            }
            i = skip(i, 32);
        }
        Ok((i, chord))
    }
}

pub fn parse_note_effects(
    note: &mut Note,
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], ()> + '_ {
    move |i| {
        log::debug!("Parsing note effects");
        let mut i = i;
        let (inner, (flags1, flags2)) = (parse_byte, parse_byte).parse(i)?;
        i = inner;
        note.effect.hammer = (flags1 & 0x02) == 0x02;
        note.effect.let_ring = (flags1 & 0x08) == 0x08;

        note.effect.staccato = (flags2 & 0x01) == 0x01;
        note.effect.palm_mute = (flags2 & 0x02) == 0x02;
        note.effect.vibrato = (flags2 & 0x40) == 0x40 || note.effect.vibrato;

        if (flags1 & 0x01) == 0x01 {
            let (inner, bend_effect) = parse_bend_effect(i)?;
            i = inner;
            note.effect.bend = Some(bend_effect);
        }

        if (flags1 & 0x10) == 0x10 {
            let (inner, grace_effect) = parse_grace_effect(version)(i)?;
            i = inner;
            note.effect.grace = Some(grace_effect);
        }

        if (flags2 & 0x04) == 0x04 {
            let (inner, tremolo_picking) = parse_tremolo_picking(i)?;
            i = inner;
            note.effect.tremolo_picking = Some(tremolo_picking);
        }

        if (flags2 & 0x08) == 0x08 {
            let (inner, slide_type) = parse_slide_type(i)?;
            i = inner;
            note.effect.slide = slide_type;
        }

        if (flags2 & 0x10) == 0x10 {
            let (inner, harmonic_effect) = parse_harmonic_effect(version)(i)?;
            i = inner;
            note.effect.harmonic = Some(harmonic_effect);
        }

        if (flags2 & 0x20) == 0x20 {
            let (inner, trill_effect) = parse_trill_effect(i)?;
            i = inner;
            note.effect.trill = Some(trill_effect);
        }

        Ok((i, ()))
    }
}

pub fn parse_trill_effect(i: &[u8]) -> IResult<&[u8], TrillEffect> {
    log::debug!("Parsing trill effect");
    let mut i = i;
    let mut trill_effect = TrillEffect::default();
    let (inner, (fret, period)) = (parse_signed_byte, parse_signed_byte).parse(i)?;
    i = inner;
    trill_effect.fret = fret;
    trill_effect.duration.value = TrillEffect::from_trill_period(period);
    Ok((i, trill_effect))
}

pub fn parse_harmonic_effect(
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], HarmonicEffect> {
    move |i| {
        let mut i = i;
        let mut he = HarmonicEffect::default();
        let (inner, harmonic_type) = parse_signed_byte(i)?;
        i = inner;
        log::debug!("Parsing harmonic effect {}", harmonic_type);
        match harmonic_type {
            1 => he.kind = HarmonicType::Natural,
            2 => {
                he.kind = HarmonicType::Artificial;
                if version >= GpVersion::GP5 {
                    let (inner, (semitone, accidental, octave)) =
                        (parse_byte, parse_signed_byte, parse_byte).parse(i)?;
                    i = inner;
                    he.pitch = Some(PitchClass::from(semitone as i8, Some(accidental), None));
                    he.octave = Some(Octave::get_octave(octave));
                }
            }
            3 => {
                he.kind = HarmonicType::Tapped;
                if version >= GpVersion::GP5 {
                    let (inner, fret) = parse_byte(i)?;
                    i = inner;
                    he.right_hand_fret = Some(fret as i8);
                }
            }
            4 => he.kind = HarmonicType::Pinch,
            5 => he.kind = HarmonicType::Semi,
            15 => {
                assert!(
                    version < GpVersion::GP5,
                    "Cannot read artificial harmonic type for GP4"
                );
                he.kind = HarmonicType::Artificial
            }
            17 => {
                assert!(
                    version < GpVersion::GP5,
                    "Cannot read artificial harmonic type for GP4"
                );
                he.kind = HarmonicType::Artificial
            }
            22 => {
                assert!(
                    version < GpVersion::GP5,
                    "Cannot read artificial harmonic type for GP4"
                );
                he.kind = HarmonicType::Artificial
            }
            x => panic!("Cannot read harmonic type {}", x),
        };
        Ok((i, he))
    }
}

pub fn parse_slide_type(i: &[u8]) -> IResult<&[u8], Option<SlideType>> {
    log::debug!("Parsing slide type");
    map(parse_byte, |t| {
        if (t & 0x01) == 0x01 {
            Some(SlideType::ShiftSlideTo)
        } else if (t & 0x02) == 0x02 {
            Some(SlideType::LegatoSlideTo)
        } else if (t & 0x04) == 0x04 {
            Some(SlideType::OutDownwards)
        } else if (t & 0x08) == 0x08 {
            Some(SlideType::OutUpWards)
        } else if (t & 0x10) == 0x10 {
            Some(SlideType::IntoFromBelow)
        } else if (t & 0x20) == 0x20 {
            Some(SlideType::IntoFromAbove)
        } else {
            None
        }
    })
    .parse(i)
}

pub fn parse_tremolo_picking(i: &[u8]) -> IResult<&[u8], TremoloPickingEffect> {
    log::debug!("Parsing tremolo picking");
    map(parse_byte, |tp| {
        let value = TremoloPickingEffect::from_tremolo_value(tp as i8);
        let mut tremolo_picking_effect = TremoloPickingEffect::default();
        tremolo_picking_effect.duration.value = value;
        tremolo_picking_effect
    })
    .parse(i)
}

pub fn parse_grace_effect(version: GpVersion) -> impl FnMut(&[u8]) -> IResult<&[u8], GraceEffect> {
    move |i| {
        log::debug!("Parsing grace effect");
        let mut i = i;
        let mut grace_effect = GraceEffect::default();

        // fret
        let (inner, fret) = parse_byte(i)?;
        i = inner;
        grace_effect.fret = fret as i8;

        // velocity
        let (inner, velocity) = parse_byte(i)?;
        i = inner;
        grace_effect.velocity = convert_velocity(velocity as i16);

        // transition
        let (inner, transition) = parse_signed_byte(i)?;
        i = inner;
        grace_effect.transition = GraceEffectTransition::get_grace_effect_transition(transition);

        // duration
        let (inner, duration) = parse_byte(i)?;
        i = inner;
        grace_effect.duration = duration;

        if version >= GpVersion::GP5 {
            // flags
            let (inner, flags) = parse_byte(i)?;
            i = inner;
            grace_effect.is_dead = (flags & 0x01) == 0x01;
            grace_effect.is_on_beat = (flags & 0x02) == 0x02;
        }

        Ok((i, grace_effect))
    }
}

pub fn parse_beat_effects<'a>(
    beat: &'a mut Beat,
    note_effect: &'a mut NoteEffect,
) -> impl FnMut(&[u8]) -> IResult<&[u8], ()> + 'a {
    move |i| {
        log::debug!("Parsing beat effects");
        let mut i = i;
        let (inner, (flags1, flags2)) = (parse_byte, parse_byte).parse(i)?;
        i = inner;

        note_effect.fade_in = flags1 & 0x10 != 0;
        note_effect.vibrato = flags1 & 0x02 != 0;

        if flags1 & 0x20 != 0 {
            let (inner, effect) = parse_byte(i)?;
            i = inner;
            note_effect.slap = match effect {
                1 => SlapEffect::Slapping,
                2 => SlapEffect::Popping,
                3 => SlapEffect::Tapping,
                _ => SlapEffect::None,
            };
        }

        if flags2 & 0x04 != 0 {
            let (inner, effect) = parse_tremolo_bar(i)?;
            i = inner;
            note_effect.tremolo_bar = Some(effect);
        }

        if flags1 & 0x40 != 0 {
            let (inner, (stroke_up, stroke_down)) =
                (parse_signed_byte, parse_signed_byte).parse(i)?;
            i = inner;
            if stroke_up > 0 {
                beat.effect.stroke.value = stroke_up as u16;
                beat.effect.stroke.direction = BeatStrokeDirection::Up;
            }
            if stroke_down > 0 {
                beat.effect.stroke.value = stroke_down as u16;
                beat.effect.stroke.direction = BeatStrokeDirection::Down;
            }
        }

        if flags2 & 0x02 != 0 {
            i = skip(i, 1);
        }

        Ok((i, ()))
    }
}

pub fn parse_bend_effect(i: &[u8]) -> IResult<&[u8], BendEffect> {
    log::debug!("Parsing bend effect");
    let mut i = skip(i, 5);
    let mut bend_effect = BendEffect::default();
    let (inner, num_points) = parse_int(i)?;
    i = inner;
    for _ in 0..num_points {
        let (inner, (bend_position, bend_value, _vibrato)) =
            (parse_int, parse_int, parse_byte).parse(i)?;
        i = inner;

        let point_position =
            bend_position as f32 * BEND_EFFECT_MAX_POSITION_LENGTH / GP_BEND_POSITION;
        let point_value = bend_value as f32 * SEMITONE_LENGTH / GP_BEND_SEMITONE;
        bend_effect.points.push(BendPoint {
            position: point_position.round() as u8,
            value: point_value.round() as i8,
        });
    }
    Ok((i, bend_effect))
}

pub fn parse_tremolo_bar(i: &[u8]) -> IResult<&[u8], TremoloBarEffect> {
    log::debug!("Parsing tremolo bar");
    let mut i = skip(i, 5);
    let mut tremolo_bar_effect = TremoloBarEffect::default();
    let (inner, num_points) = parse_int(i)?;
    i = inner;
    for _ in 0..num_points {
        let (inner, (position, value, _vibrato)) = (parse_int, parse_int, parse_byte).parse(i)?;
        i = inner;

        let point_position = position as f32 * BEND_EFFECT_MAX_POSITION_LENGTH / GP_BEND_POSITION;
        let point_value = value as f32 / GP_BEND_SEMITONE * 2.0f32;
        tremolo_bar_effect.points.push(BendPoint {
            position: point_position.round() as u8,
            value: point_value.round() as i8,
        });
    }
    Ok((i, tremolo_bar_effect))
}

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
        let (inner, value) = parse_signed_byte(i)?;
        i = inner;
        d.value = (2_u32.pow((value + 4) as u32) / 4) as u16;
        log::debug!("Duration value: {}", d.value);
        d.dotted = flags & 0x01 != 0;

        if (flags & 0x20) == 0x20 {
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
                x => panic!("Unknown tuplet: {}", x),
            }
        }

        Ok((i, d))
    }
}

pub fn parse_color(i: &[u8]) -> IResult<&[u8], i32> {
    log::debug!("Parsing RGB color");
    map(
        (parse_byte, parse_byte, parse_byte, parse_byte),
        |(r, g, b, _ignore)| (r as i32) << 16 | (g as i32) << 8 | b as i32,
    )
    .parse(i)
}

pub fn parse_marker(i: &[u8]) -> IResult<&[u8], Marker> {
    log::debug!("Parsing marker");
    map((parse_int_sized_string, parse_color), |(title, color)| {
        Marker { title, color }
    })
    .parse(i)
}

pub fn parse_triplet_feel(i: &[u8]) -> IResult<&[u8], TripletFeel> {
    log::debug!("Parsing triplet feel");
    map(parse_signed_byte, |triplet_feel| match triplet_feel {
        0 => TripletFeel::None,
        1 => TripletFeel::Eighth,
        2 => TripletFeel::Sixteenth,
        x => panic!("Unknown triplet feel: {}", x),
    })
    .parse(i)
}

/// Parse measure header.
/// the time signature is propagated to the next measure
pub fn parse_measure_header(
    previous_time_signature: TimeSignature,
    song_tempo: i32,
    song_version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], MeasureHeader> {
    move |i: &[u8]| {
        log::debug!("Parsing measure header");
        let (mut i, flags) = parse_byte(i)?;
        log::debug!("Flags: {:08b}", flags);
        let mut mh = MeasureHeader::default();
        mh.tempo.value = song_tempo; // value updated later when parsing beats
        mh.repeat_open = (flags & 0x04) == 0x04;
        // propagate time signature
        mh.time_signature = previous_time_signature.clone();

        // Numerator of the (key) signature
        if (flags & 0x01) != 0 {
            log::debug!("Parsing numerator");
            let (inner, numerator) = parse_signed_byte(i)?;
            i = inner;
            mh.time_signature.numerator = numerator;
        }

        // Denominator of the (key) signature
        if (flags & 0x02) != 0 {
            log::debug!("Parsing denominator");
            let (inner, denominator_value) = parse_signed_byte(i)?;
            i = inner;
            let denominator = Duration {
                value: denominator_value as u16,
                ..Default::default()
            };
            mh.time_signature.denominator = denominator;
        }

        // Beginning of repeat
        if (flags & 0x08) != 0 {
            log::debug!("Parsing repeat close");
            let (inner, repeat_close) = parse_signed_byte(i)?;
            i = inner;
            mh.repeat_close = repeat_close;
        }

        // Presence of a marker
        if (flags & 0x20) != 0 {
            let (inner, marker) = parse_marker(i)?;
            i = inner;
            mh.marker = Some(marker);
        }

        // Number of alternate ending
        if (flags & 0x10) != 0 {
            log::debug!("Parsing repeat alternative");
            let (inner, alternative) = parse_byte(i)?;
            i = inner;
            mh.repeat_alternative = alternative;
        }

        // Tonality of the measure
        if (flags & 0x40) != 0 {
            log::debug!("Parsing key signature");
            let (inner, key_signature) = parse_signed_byte(i)?;
            mh.key_signature.key = key_signature;
            i = inner;
            let (inner, is_minor) = parse_signed_byte(i)?;
            i = inner;
            mh.key_signature.is_minor = is_minor != 0;
        }

        if song_version >= GpVersion::GP5 {
            if (flags & 0x01) != 0 || (flags & 0x02) != 0 {
                log::debug!("Skip 4");
                i = skip(i, 4);
            }

            if (flags & 0x10) == 0 {
                log::debug!("Skip one");
                i = skip(i, 1);
            }

            let (inner, triplet_feel) = parse_triplet_feel(i)?;
            i = inner;
            mh.triplet_feel = triplet_feel;
        }
        log::debug!("{:?}", mh);

        Ok((i, mh))
    }
}

pub fn parse_measure_headers(
    measure_count: i32,
    song_tempo: i32,
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], Vec<MeasureHeader>> {
    move |i: &[u8]| {
        log::debug!("Parsing {} measure headers", measure_count);
        // parse first header to account for the byte in between each header
        let (mut i, first_header) =
            parse_measure_header(TimeSignature::default(), song_tempo, version)(i)?;
        let mut previous_time_signature = first_header.time_signature.clone();
        let mut headers = vec![first_header];
        for _ in 1..measure_count {
            let (rest, header) = preceded(
                cond(version >= GpVersion::GP5, parse_byte),
                parse_measure_header(previous_time_signature, song_tempo, version),
            )
            .parse(i)?;
            // propagate time signature
            previous_time_signature = header.time_signature.clone();
            i = rest;
            headers.push(header);
        }
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
                parse_int,
                parse_signed_byte,
                parse_signed_byte,
                parse_signed_byte,
                parse_signed_byte,
                parse_signed_byte,
                parse_signed_byte,
                parse_byte,
                parse_byte,
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
            copyright: format!("{}\n{}", copyright_1, copyright_2),
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
    parse_byte_size_string(30)(i).map(|(i, version_string)| match version_string.as_str() {
        "FICHIER GUITAR PRO v3.00" => (i, GpVersion::GP3),
        "FICHIER GUITAR PRO v4.00" => (i, GpVersion::GP4),
        "FICHIER GUITAR PRO v4.06" => (i, GpVersion::GP4_06),
        "FICHIER GUITAR PRO v5.00" => (i, GpVersion::GP5),
        "FICHIER GUITAR PRO v5.10" => (i, GpVersion::GP5_10),
        _ => panic!("Unsupported GP version: {}", version_string),
    })
}

fn parse_notices(i: &[u8]) -> IResult<&[u8], Vec<String>> {
    flat_map(parse_int, |notice_count| {
        log::debug!("Notice count: {}", notice_count);
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

pub fn parse_gp_data(file_data: &[u8]) -> Result<Song, RuxError> {
    let (rest, base_song) = flat_map(parse_gp_version, |version| {
        map(
            (
                parse_info(version),                                     // Song info
                cond(version < GpVersion::GP5, parse_bool),              // Triplet feel
                cond(version >= GpVersion::GP4, parse_lyrics),           // Lyrics
                cond(version >= GpVersion::GP5_10, take(19usize)),       // Skip RSE master effect
                cond(version >= GpVersion::GP5, parse_page_setup),       // Page setup
                cond(version >= GpVersion::GP5, parse_int_sized_string), // Tempo name
                parse_int,                                               // Tempo
                cond(version > GpVersion::GP5, parse_bool),              // Tempo hide
                parse_signed_byte,                                       // Key signature
                cond(version > GpVersion::GP3, parse_int),               // Octave
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
                octave,
                midi_channels,
            )| {
                // init base song
                let tempo = Tempo::new(tempo, tempo_name);
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
    let (_rest, _unit) = parser.parse_music_data(rest).map_err(|e| {
        log::error!("Failed to parse music data: {:?}", e);
        RuxError::ParsingError("Failed to parse music data".to_string())
    })?;
    let song = parser.take_song();
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
}
