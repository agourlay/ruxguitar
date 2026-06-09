//! Intermediate data model for a parsed GPX (`score.gpif`) document.
//!
//! Mirrors Tuxguitar's `io.gpx.score.GPX*` classes: a set of flat, id-referenced
//! lists. The XML reader (Layer 2) fills these in; the mapping into the engine
//! `Song` model (Layer 3) walks them by id.

/// MIDI channel reserved for percussion in GPX.
pub const DEFAULT_PERCUSSION_CHANNEL: i32 = 9;

#[derive(Debug, Default)]
pub struct GpxScore {
    pub title: Option<String>,
    pub sub_title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub words: Option<String>,
    pub music: Option<String>,
    pub words_and_music: Option<String>,
    pub copyright: Option<String>,
    pub tabber: Option<String>,
    pub instructions: Option<String>,
    pub notices: Option<String>,
}

#[derive(Debug, Default)]
pub struct GpxAutomation {
    pub kind: Option<String>,
    pub bar_id: i32,
    pub position: i32,
    pub linear: bool,
    pub visible: bool,
    pub value: Vec<i32>,
}

#[derive(Debug, Default)]
pub struct GpxTrack {
    pub id: i32,
    pub name: String,
    pub tuning_pitches: Option<Vec<i32>>,
    pub color: Option<Vec<i32>>,
    pub capo: i32,
    pub gm_program: i32,
    pub gm_channel_1: i32,
    pub gm_channel_2: i32,
}

#[derive(Debug, Default)]
pub struct GpxChord {
    pub id: i32,
    pub name: Option<String>,
    pub string_count: Option<i32>,
    pub fret_count: Option<i32>,
    pub base_fret: Option<i32>,
    pub frets: Vec<Option<i32>>,
}

#[derive(Debug, Default)]
pub struct GpxMasterBar {
    pub bar_ids: Vec<i32>,
    pub time: Option<Vec<i32>>,
    pub repeat_count: i32,
    pub repeat_start: bool,
    pub accidental_count: i32,
    pub mode: Option<String>,
    pub triplet_feel: Option<String>,
    pub alternate_endings: Option<Vec<i32>>,
    pub marker_text: Option<String>,
}

#[derive(Debug, Default)]
pub struct GpxBar {
    pub id: i32,
    pub voice_ids: Vec<i32>,
    pub clef: Option<String>,
    pub simile_mark: Option<String>,
}

#[derive(Debug, Default)]
pub struct GpxVoice {
    pub id: i32,
    pub beat_ids: Vec<i32>,
}

#[derive(Debug, Default)]
pub struct GpxBeat {
    pub id: i32,
    pub rhythm_id: i32,
    pub note_ids: Option<Vec<i32>>,
    pub dynamic: Option<String>,
    pub slapped: bool,
    pub popped: bool,
    /// Brush (strum) direction: "Up" / "Down" / "" (none).
    pub brush: String,
    /// Pick-stroke direction: "Up" / "Down" / "" (none).
    pub pick_stroke: String,
    pub tremolo: Option<Vec<i32>>,
    pub fadding: Option<String>,
    pub text: String,
    pub chord_id: Option<i32>,
    pub grace_notes: Option<String>,
    pub whammy_bar_enabled: bool,
    pub whammy_bar_origin_value: Option<i32>,
    pub whammy_bar_middle_value: Option<i32>,
    pub whammy_bar_destination_value: Option<i32>,
    pub whammy_bar_origin_offset: Option<i32>,
    pub whammy_bar_middle_offset_1: Option<i32>,
    pub whammy_bar_middle_offset_2: Option<i32>,
    pub whammy_bar_destination_offset: Option<i32>,
}

#[derive(Debug)]
pub struct GpxNote {
    pub id: i32,
    pub fret: i32,
    pub string: i32,
    pub tone: i32,
    pub octave: i32,
    pub element: i32,
    pub variation: i32,
    pub midi_number: i32,
    pub trill: i32,
    pub trill_duration: i32,
    pub accent: i32,
    pub slide_flags: i32,
    pub harmonic_fret: i32,
    pub harmonic_type: String,
    pub bend_enabled: bool,
    pub bend_origin_value: Option<i32>,
    pub bend_middle_value: Option<i32>,
    pub bend_destination_value: Option<i32>,
    pub bend_origin_offset: Option<i32>,
    pub bend_middle_offset_1: Option<i32>,
    pub bend_middle_offset_2: Option<i32>,
    pub bend_destination_offset: Option<i32>,
    pub hammer: bool,
    pub ghost: bool,
    pub slide: bool,
    pub vibrato: bool,
    pub let_ring: bool,
    pub tapped: bool,
    pub tie_destination: bool,
    pub muted_enabled: bool,
    pub palm_muted_enabled: bool,
}

impl Default for GpxNote {
    fn default() -> Self {
        GpxNote {
            id: -1,
            fret: -1,
            string: -1,
            tone: -1,
            octave: -1,
            element: -1,
            variation: -1,
            midi_number: -1,
            trill: 0,
            trill_duration: 0,
            accent: 0,
            slide_flags: 0,
            harmonic_fret: -1,
            harmonic_type: String::new(),
            bend_enabled: false,
            bend_origin_value: None,
            bend_middle_value: None,
            bend_destination_value: None,
            bend_origin_offset: None,
            bend_middle_offset_1: None,
            bend_middle_offset_2: None,
            bend_destination_offset: None,
            hammer: false,
            ghost: false,
            slide: false,
            vibrato: false,
            let_ring: false,
            tapped: false,
            tie_destination: false,
            muted_enabled: false,
            palm_muted_enabled: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct GpxRhythm {
    pub id: i32,
    pub note_value: Option<String>,
    pub augmentation_dot_count: i32,
    pub primary_tuplet_num: i32,
    pub primary_tuplet_den: i32,
}

#[derive(Debug, Default)]
pub struct GpxDocument {
    pub score: GpxScore,
    pub tracks: Vec<GpxTrack>,
    pub master_bars: Vec<GpxMasterBar>,
    pub bars: Vec<GpxBar>,
    pub voices: Vec<GpxVoice>,
    pub beats: Vec<GpxBeat>,
    pub notes: Vec<GpxNote>,
    pub chords: Vec<GpxChord>,
    pub rhythms: Vec<GpxRhythm>,
    pub automations: Vec<GpxAutomation>,
}

impl GpxDocument {
    pub fn bar(&self, id: i32) -> Option<&GpxBar> {
        self.bars.iter().find(|b| b.id == id)
    }

    pub fn voice(&self, id: i32) -> Option<&GpxVoice> {
        self.voices.iter().find(|v| v.id == id)
    }

    pub fn beat(&self, id: i32) -> Option<&GpxBeat> {
        self.beats.iter().find(|b| b.id == id)
    }

    pub fn note(&self, id: i32) -> Option<&GpxNote> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn chord(&self, id: i32) -> Option<&GpxChord> {
        self.chords.iter().find(|c| c.id == id)
    }

    pub fn rhythm(&self, id: i32) -> Option<&GpxRhythm> {
        self.rhythms.iter().find(|r| r.id == id)
    }

    /// Most recent automation of the given type at or before `until_bar_id`.
    pub fn automation(&self, kind: &str, until_bar_id: i32) -> Option<&GpxAutomation> {
        let mut result: Option<&GpxAutomation> = None;
        for automation in &self.automations {
            if automation.kind.as_deref() == Some(kind)
                && automation.bar_id <= until_bar_id
                && result.is_none_or(|r| automation.bar_id > r.bar_id)
            {
                result = Some(automation);
            }
        }
        result
    }
}
