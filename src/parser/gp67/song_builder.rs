//! Map a parsed GPX document into the engine `Song` model (Layer 3).
//!
//! Port of Tuxguitar's `GPXDocumentParser`, adapted to ruxguitar's model where
//! each `Measure` holds its voices directly and each `Beat` carries a single
//! duration (rather than Tuxguitar's beat-with-multiple-voices layout).

use crate::RuxError;
use crate::parser::gp67::archive::read_gp7_gpif;
use crate::parser::gp67::document::{
    DEFAULT_PERCUSSION_CHANNEL, GpxBar, GpxBeat, GpxDocument, GpxMasterBar, GpxNote, GpxRhythm,
};
use crate::parser::gp67::document_reader::{GpifVersion, read_document};
use crate::parser::gp67::file_system::GpxFileSystem;
use crate::parser::song_parser::{
    BEND_EFFECT_MAX_POSITION_LENGTH, Beat, BeatEffects, BeatStroke, BeatStrokeDirection,
    BendEffect, BendPoint, Chord, DEFAULT_BANK, DEFAULT_PERCUSSION_BANK, DEFAULT_VELOCITY,
    DURATION_SIXTEENTH, DURATION_SIXTY_FOURTH, DURATION_THIRTY_SECOND, Duration, GP_BEND_SEMITONE,
    GpVersion, GraceEffect, GraceEffectTransition, HarmonicEffect, HarmonicType, KeySignature,
    MAX_VOICES, Marker, Measure, MeasureHeader, MidiChannel, Note, NoteEffect, NoteType, QUARTER,
    QUARTER_TIME, SEMITONE_LENGTH, SlapEffect, SlideType, Song, SongInfo, Tempo, TimeSignature,
    Track, TremoloBarEffect, TremoloPickingEffect, TrillEffect, TripletFeel, Voice,
    convert_velocity,
};

/// Position units used by GPX bend/whammy offsets (a full bar = 100%).
const GP_POSITION_GPX: f32 = 100.0;
/// Whammy-bar value units in GPX (a full step = 50).
const GP_WHAMMY_SEMITONE: f32 = 50.0;
/// Clamp for tremolo-bar point values.
const TREMOLO_BAR_MAX_VALUE: i8 = 12;
/// Stroke speed assigned to GPX brushes (the score stores only a direction).
const BRUSH_STROKE_VALUE: u16 = DURATION_THIRTY_SECOND as u16;

/// `(midi_value, element, variation)` drum-kit lookup, from Tuxguitar's `GPXDrumkit`.
const DRUMKITS: [(i32, i32, i32); 21] = [
    (36, 0, 0),
    (36, 0, 0),
    (37, 1, 2),
    (38, 1, 0),
    (41, 5, 0),
    (42, 10, 0),
    (43, 6, 0),
    (44, 11, 0),
    (45, 7, 0),
    (46, 10, 2),
    (47, 8, 0),
    (48, 9, 0),
    (49, 12, 0),
    (50, 9, 0),
    (51, 15, 0),
    (52, 16, 0),
    (53, 15, 2),
    (55, 14, 0),
    (56, 3, 0),
    (57, 13, 0),
    (59, 15, 1),
];

/// Decode a `.gpx` file (Guitar Pro 6) all the way to a `Song`.
pub fn parse_gpx_data(data: &[u8]) -> Result<Song, RuxError> {
    let fs = GpxFileSystem::load(data)?;
    let xml = fs
        .file_contents("score.gpif")
        .ok_or_else(|| RuxError::ParsingError("no score.gpif in GPX file".to_string()))?;
    build_from_gpif(xml, GpifVersion::Gp6, GpVersion::GP6)
}

/// Decode a `.gp` file (Guitar Pro 7) all the way to a `Song`.
pub fn parse_gp7_data(data: &[u8]) -> Result<Song, RuxError> {
    let xml = read_gp7_gpif(data)?;
    build_from_gpif(&xml, GpifVersion::Gp7, GpVersion::GP7)
}

fn build_from_gpif(
    xml: &[u8],
    gpif_version: GpifVersion,
    song_version: GpVersion,
) -> Result<Song, RuxError> {
    let xml = std::str::from_utf8(xml)
        .map_err(|e| RuxError::ParsingError(format!("score.gpif is not UTF-8: {e}")))?;
    let document = read_document(xml, gpif_version)?;
    Ok(build_song(&document, song_version))
}

/// Map the intermediate document into a `Song`.
pub fn build_song(doc: &GpxDocument, version: GpVersion) -> Song {
    let mut song = Song {
        version,
        song_info: build_song_info(doc),
        ..Default::default()
    };

    let (mut tracks, channels) = build_tracks(doc);
    song.midi_channels = channels;

    build_measures(doc, &mut song, &mut tracks);
    song.tracks = tracks;

    if let Some(first) = song.measure_headers.first() {
        song.tempo = Tempo::new(first.tempo.value, first.tempo.name.clone());
    }
    song
}

fn build_song_info(doc: &GpxDocument) -> SongInfo {
    let s = &doc.score;
    SongInfo {
        name: s.title.clone().unwrap_or_default(),
        subtitle: s.sub_title.clone().unwrap_or_default(),
        artist: s.artist.clone().unwrap_or_default(),
        album: s.album.clone().unwrap_or_default(),
        author: s.words_and_music.clone().unwrap_or_default(),
        words: s.words.clone(),
        copyright: s.copyright.clone().unwrap_or_default(),
        writer: s.tabber.clone().unwrap_or_default(),
        instructions: s.instructions.clone().unwrap_or_default(),
        notices: s.notices.iter().cloned().collect(),
    }
}

/// Build the track shells (without measures) and their MIDI channels.
fn build_tracks(doc: &GpxDocument) -> (Vec<Track>, Vec<MidiChannel>) {
    let mut tracks = Vec::with_capacity(doc.tracks.len());
    let mut channels = Vec::with_capacity(doc.tracks.len());
    let mut next_channel: u8 = 0;

    for (index, gp_track) in doc.tracks.iter().enumerate() {
        let is_percussion = gp_track.gm_channel_1 == DEFAULT_PERCUSSION_CHANNEL;
        let channel_id = if is_percussion {
            9
        } else {
            let c = next_channel;
            next_channel += 1;
            if next_channel == 9 {
                next_channel += 1; // reserve channel 9 for percussion
            }
            c.min(15)
        };

        channels.push(MidiChannel {
            channel_id,
            effect_channel_id: channel_id,
            instrument: if is_percussion {
                0
            } else {
                gp_track.gm_program
            },
            // Raw GP channel scale (0-16), scaled to MIDI 0-127 at emit time.
            volume: 16, // full
            balance: 8, // center
            chorus: 0,
            reverb: 0,
            phaser: 0,
            tremolo: 0,
            bank: if is_percussion {
                DEFAULT_PERCUSSION_BANK
            } else {
                DEFAULT_BANK
            },
        });

        let strings = build_strings(gp_track.tuning_pitches.as_deref(), is_percussion);
        let color = match gp_track.color.as_deref() {
            Some([r, g, b]) => (r << 16) | (g << 8) | b,
            _ => 0,
        };

        tracks.push(Track {
            number: index as i32 + 1,
            offset: gp_track.capo,
            channel_id,
            name: gp_track.name.clone(),
            strings,
            color,
            ..Default::default()
        });
    }

    (tracks, channels)
}

/// Build the `(string_number, tuning)` list for a track.
fn build_strings(tuning_pitches: Option<&[i32]>, is_percussion: bool) -> Vec<(i32, i32)> {
    if let Some(pitches) = tuning_pitches
        && !pitches.is_empty()
    {
        // String 1 is the highest pitch; GPX stores them lowest-first.
        return (1..=pitches.len())
            .map(|s| (s as i32, pitches[pitches.len() - s]))
            .collect();
    }
    if is_percussion {
        return (1..=6).map(|s| (s, 0)).collect();
    }
    // Standard 6-string guitar tuning.
    [64, 59, 55, 50, 45, 40]
        .into_iter()
        .enumerate()
        .map(|(i, v)| (i as i32 + 1, v))
        .collect()
}

/// Walk the master bars, appending a measure header and one measure per track.
fn build_measures(doc: &GpxDocument, song: &mut Song, tracks: &mut [Track]) {
    let mut start = QUARTER_TIME;

    for (index, mbar) in doc.master_bars.iter().enumerate() {
        let header = build_measure_header(doc, mbar, index, start);
        let length = header.length();
        let time_signature = header.time_signature.clone();
        let key_signature = clone_key_signature(&header.key_signature);
        song.measure_headers.push(header);

        for (track_index, track) in tracks.iter_mut().enumerate() {
            let mut measure = Measure {
                header_index: index,
                track_index,
                time_signature: time_signature.clone(),
                key_signature: clone_key_signature(&key_signature),
                voices: Vec::with_capacity(MAX_VOICES as usize),
            };

            let gp_bar = resolve_bar(doc, index, track_index);
            for voice_slot in 0..MAX_VOICES as usize {
                let voice = build_voice(doc, gp_bar, voice_slot, index, start, &track.strings);
                measure.voices.push(voice);
            }

            if index == 0 {
                fix_first_measure_start_positions(&mut measure, start, length);
            }

            track.measures.push(measure);
        }

        start += length;
    }
}

fn build_measure_header(
    doc: &GpxDocument,
    mbar: &GpxMasterBar,
    index: usize,
    start: u32,
) -> MeasureHeader {
    let mut header = MeasureHeader {
        start,
        repeat_open: mbar.repeat_start,
        repeat_close: mbar.repeat_count as i8,
        triplet_feel: triplet_feel_of(mbar),
        key_signature: key_signature_of(mbar),
        ..Default::default()
    };

    if let Some(endings) = &mbar.alternate_endings {
        let mut mask = 0u8;
        for &ending in endings {
            if ending >= 1 {
                mask |= 1 << (ending - 1);
            }
        }
        header.repeat_alternative = mask;
    }

    if let Some(time) = &mbar.time
        && time.len() == 2
    {
        header.time_signature = TimeSignature {
            numerator: time[0] as u8,
            denominator: Duration {
                value: time[1] as u16,
                ..Default::default()
            },
        };
    }

    if let Some(automation) = doc.automation("Tempo", index as i32)
        && automation.value.len() == 2
    {
        let mut tempo = automation.value[0];
        match automation.value[1] {
            1 => tempo /= 2,
            3 => tempo += tempo / 2,
            4 => tempo *= 2,
            5 => tempo += tempo * 2,
            _ => {}
        }
        header.tempo.value = tempo.max(1) as u32;
    }

    if let Some(text) = &mbar.marker_text
        && !text.is_empty()
    {
        header.marker = Some(Marker {
            title: text.clone(),
            color: 0,
        });
    }

    header
}

fn key_signature_of(mbar: &GpxMasterBar) -> KeySignature {
    let key = mbar.accidental_count.clamp(-7, 7) as i8;
    let is_minor = mbar
        .mode
        .as_deref()
        .is_some_and(|m| m.eq_ignore_ascii_case("minor"));
    KeySignature::new(key, is_minor)
}

const fn clone_key_signature(k: &KeySignature) -> KeySignature {
    KeySignature::new(k.key, k.is_minor)
}

fn triplet_feel_of(mbar: &GpxMasterBar) -> TripletFeel {
    match mbar.triplet_feel.as_deref() {
        Some("Triplet8th") => TripletFeel::Eighth,
        Some("Triplet16th") => TripletFeel::Sixteenth,
        _ => TripletFeel::None,
    }
}

/// Resolve the GPX bar for a track at a master-bar index, following simile marks
/// that point back at an earlier bar to be repeated.
fn resolve_bar(doc: &GpxDocument, master_index: usize, track_index: usize) -> Option<&GpxBar> {
    let bar_of = |mi: usize| -> Option<&GpxBar> {
        let mbar = doc.master_bars.get(mi)?;
        let bar_id = mbar.bar_ids.get(track_index)?;
        doc.bar(*bar_id)
    };

    let mut current_index = master_index as isize;
    let mut bar = bar_of(master_index);
    while let Some(b) = bar {
        match b.simile_mark.as_deref() {
            Some("Simple") => current_index -= 1,
            Some("FirstOfDouble" | "SecondOfDouble") => current_index -= 2,
            _ => break,
        }
        if current_index >= 0 {
            bar = bar_of(current_index as usize);
        } else {
            return None;
        }
    }
    bar
}

/// Build one voice (`voice_slot`) of a measure.
fn build_voice(
    doc: &GpxDocument,
    gp_bar: Option<&GpxBar>,
    voice_slot: usize,
    measure_index: usize,
    measure_start: u32,
    strings: &[(i32, i32)],
) -> Voice {
    let mut voice = Voice {
        measure_index: measure_index as i16,
        beats: Vec::new(),
    };

    let gp_voice = gp_bar
        .and_then(|bar| bar.voice_ids.get(voice_slot))
        .filter(|&&id| id >= 0)
        .and_then(|&id| doc.voice(id));
    let Some(gp_voice) = gp_voice else {
        return voice;
    };

    let mut start = measure_start;
    let mut previous_beat: Option<&GpxBeat> = None;
    let mut previous_grace_notes: Vec<&GpxNote> = Vec::new();
    let mut previous_duration = Duration::default();

    for &beat_id in &gp_voice.beat_ids {
        let Some(gp_beat) = doc.beat(beat_id) else {
            continue;
        };
        let duration = rhythm_to_duration(doc.rhythm(gp_beat.rhythm_id));

        // GPX puts grace notes on a separate preceding beat; we attach them to
        // the following beat's notes, so buffer them here without advancing time.
        if gp_beat.grace_notes.is_some() {
            previous_grace_notes.clear();
            if let Some(note_ids) = &gp_beat.note_ids {
                for &id in note_ids {
                    if let Some(note) = doc.note(id) {
                        previous_grace_notes.push(note);
                    }
                }
            }
            previous_beat = Some(gp_beat);
            previous_duration = duration;
            continue;
        }

        let mut beat = Beat {
            start,
            duration: duration.clone(),
            empty: false,
            text: gp_beat.text.trim().to_string(),
            effect: BeatEffects {
                stroke: stroke_of(gp_beat),
                chord: chord_of(doc, gp_beat),
            },
            notes: Vec::new(),
        };

        if let Some(note_ids) = &gp_beat.note_ids {
            let velocity = dynamic_velocity(gp_beat.dynamic.as_deref());
            for &id in note_ids {
                if let Some(gp_note) = doc.note(id)
                    && let Some(note) = build_note(
                        gp_note,
                        strings,
                        &beat.notes,
                        velocity,
                        gp_beat,
                        previous_beat,
                        &previous_grace_notes,
                        &previous_duration,
                    )
                {
                    beat.notes.push(note);
                }
            }
        }

        start += beat.duration.time();
        voice.beats.push(beat);
        previous_beat = Some(gp_beat);
        previous_grace_notes.clear();
        previous_duration = duration;
    }

    voice
}

fn rhythm_to_duration(rhythm: Option<&GpxRhythm>) -> Duration {
    let mut duration = Duration::default();
    if let Some(rhythm) = rhythm {
        duration.dotted = rhythm.augmentation_dot_count == 1;
        duration.double_dotted = rhythm.augmentation_dot_count == 2;
        duration.tuplet_enters = rhythm.primary_tuplet_num.max(1) as u8;
        duration.tuplet_times = rhythm.primary_tuplet_den.max(1) as u8;
        duration.value = match rhythm.note_value.as_deref() {
            Some("Whole") => 1,
            Some("Half") => 2,
            Some("Quarter") => QUARTER,
            Some("Eighth") => 8,
            Some("16th") => 16,
            Some("32nd") => 32,
            Some("64th") => 64,
            _ => QUARTER,
        };
    }
    duration
}

fn stroke_of(gp_beat: &GpxBeat) -> BeatStroke {
    match gp_beat.brush.as_str() {
        "Down" => BeatStroke {
            direction: BeatStrokeDirection::Down,
            value: BRUSH_STROKE_VALUE,
        },
        "Up" => BeatStroke {
            direction: BeatStrokeDirection::Up,
            value: BRUSH_STROKE_VALUE,
        },
        _ => BeatStroke::default(),
    }
}

fn chord_of(doc: &GpxDocument, gp_beat: &GpxBeat) -> Option<Chord> {
    let gp_chord = doc.chord(gp_beat.chord_id?)?;
    let mut chord = Chord {
        length: gp_chord.frets.len() as u8,
        name: gp_chord.name.clone().unwrap_or_default(),
        first_fret: gp_chord.base_fret.map(|f| f as u32),
        strings: vec![-1; gp_chord.frets.len()],
        ..Default::default()
    };
    for (i, fret) in gp_chord.frets.iter().enumerate() {
        if let Some(value) = fret {
            chord.strings[i] = *value as i8;
        }
    }
    Some(chord)
}

fn dynamic_velocity(dynamic: Option<&str>) -> i16 {
    let level = match dynamic {
        Some("PPP") => 1,
        Some("PP") => 2,
        Some("P") => 3,
        Some("MP") => 4,
        Some("MF") => 5,
        Some("F") => 6,
        Some("FF") => 7,
        Some("FFF") => 8,
        _ => return DEFAULT_VELOCITY,
    };
    convert_velocity(level)
}

#[allow(clippy::too_many_arguments)]
fn build_note(
    gp_note: &GpxNote,
    strings: &[(i32, i32)],
    beat_notes: &[Note],
    velocity: i16,
    gp_beat: &GpxBeat,
    previous_beat: Option<&GpxBeat>,
    previous_grace_notes: &[&GpxNote],
    previous_duration: &Duration,
) -> Option<Note> {
    let string_count = strings.len() as i32;
    let (value, string) = if gp_note.string >= 0 && gp_note.fret >= 0 {
        (gp_note.fret, string_count - gp_note.string)
    } else {
        let gm_value = midi_value_of(gp_note)?;
        let (string_number, string_tuning) = string_for(strings, beat_notes, gm_value)?;
        (gm_value - string_tuning, string_number)
    };

    if value < 0 || string <= 0 || string > string_count {
        return None;
    }

    let mut note = Note::new(NoteEffect::default());
    note.value = value as i16;
    note.string = string as i8;
    note.velocity = velocity;
    note.kind = if gp_note.tie_destination {
        NoteType::Tie
    } else if gp_note.muted_enabled {
        NoteType::Dead
    } else {
        NoteType::Normal
    };

    let effect = &mut note.effect;
    effect.let_ring = gp_note.let_ring;
    effect.vibrato = gp_note.vibrato;
    effect.palm_mute = gp_note.palm_muted_enabled;
    effect.hammer = gp_note.hammer;
    effect.ghost_note = gp_note.ghost;
    effect.staccato = gp_note.accent == 1;
    effect.heavy_accentuated_note = gp_note.accent == 4;
    effect.accentuated_note = gp_note.accent == 8;
    effect.fade_in = gp_beat.fadding.as_deref() == Some("FadeIn");
    effect.slap = if gp_beat.slapped {
        SlapEffect::Slapping
    } else if gp_beat.popped {
        SlapEffect::Popping
    } else if gp_note.tapped {
        SlapEffect::Tapping
    } else {
        SlapEffect::None
    };
    if gp_note.slide {
        effect.slide = Some(slide_type_of(gp_note.slide_flags));
    }
    effect.trill = trill_of(gp_note, value);
    effect.tremolo_picking = tremolo_picking_of(gp_beat);
    effect.harmonic = harmonic_of(gp_note);
    effect.bend = bend_of(gp_note);
    effect.tremolo_bar = tremolo_bar_of(gp_beat);
    effect.grace = grace_of(
        previous_beat,
        previous_grace_notes,
        previous_duration,
        gp_note,
    );

    Some(note)
}

fn midi_value_of(gp_note: &GpxNote) -> Option<i32> {
    if gp_note.midi_number >= 0 {
        Some(gp_note.midi_number)
    } else if gp_note.tone >= 0 && gp_note.octave >= 0 {
        Some(gp_note.tone + (12 * gp_note.octave - 12))
    } else if gp_note.element >= 0 {
        DRUMKITS
            .iter()
            .find(|(_, element, variation)| {
                *element == gp_note.element && *variation == gp_note.variation
            })
            .map(|(midi, _, _)| *midi)
    } else {
        None
    }
}

/// Pick the lowest string able to play `value` that is not already used in the beat.
fn string_for(strings: &[(i32, i32)], beat_notes: &[Note], value: i32) -> Option<(i32, i32)> {
    for &(number, tuning) in strings {
        if value >= tuning && !beat_notes.iter().any(|n| i32::from(n.string) == number) {
            return Some((number, tuning));
        }
    }
    None
}

const fn slide_type_of(flags: i32) -> SlideType {
    if flags & 0x02 != 0 {
        SlideType::LegatoSlideTo
    } else if flags & 0x04 != 0 {
        SlideType::OutDownwards
    } else if flags & 0x08 != 0 {
        SlideType::OutUpWards
    } else if flags & 0x10 != 0 {
        SlideType::IntoFromBelow
    } else if flags & 0x20 != 0 {
        SlideType::IntoFromAbove
    } else {
        SlideType::ShiftSlideTo
    }
}

fn trill_of(gp_note: &GpxNote, initial_fret: i32) -> Option<TrillEffect> {
    if gp_note.trill <= 0 {
        return None;
    }
    let diff = gp_note.trill - gp_note.midi_number;
    // GPX uses 480 ticks per quarter; ruxguitar uses QUARTER_TIME (960).
    let ticks = (gp_note.trill_duration * 2).max(1) as u32;
    let value = (QUARTER_TIME * 4 / ticks).clamp(1, 64) as u16;
    Some(TrillEffect {
        fret: (initial_fret + diff) as i8,
        duration: Duration {
            value,
            ..Default::default()
        },
    })
}

fn tremolo_picking_of(gp_beat: &GpxBeat) -> Option<TremoloPickingEffect> {
    let tremolo = gp_beat.tremolo.as_ref()?;
    if tremolo.len() != 2 {
        return None;
    }
    Some(TremoloPickingEffect {
        duration: Duration {
            value: (i32::from(QUARTER) * tremolo[1]).clamp(1, 64) as u16,
            ..Default::default()
        },
    })
}

fn harmonic_of(gp_note: &GpxNote) -> Option<HarmonicEffect> {
    if gp_note.harmonic_type.is_empty() {
        return None;
    }
    let kind = match gp_note.harmonic_type.as_str() {
        "Artificial" => HarmonicType::Artificial,
        "Pinch" => HarmonicType::Pinch,
        _ => HarmonicType::Natural,
    };
    Some(HarmonicEffect {
        kind,
        ..Default::default()
    })
}

fn bend_value(gp_value: i32) -> i8 {
    (gp_value as f32 * SEMITONE_LENGTH / GP_BEND_SEMITONE).round() as i8
}

fn bend_position(gp_offset: i32) -> u8 {
    (gp_offset as f32 * BEND_EFFECT_MAX_POSITION_LENGTH / GP_POSITION_GPX).round() as u8
}

fn bend_of(gp_note: &GpxNote) -> Option<BendEffect> {
    if !gp_note.bend_enabled {
        return None;
    }
    let origin = gp_note.bend_origin_value?;
    let destination = gp_note.bend_destination_value?;

    let mut bend = BendEffect::default();
    bend.points.push(BendPoint {
        position: 0,
        value: bend_value(origin),
    });
    if let Some(offset) = gp_note.bend_origin_offset {
        bend.points.push(BendPoint {
            position: bend_position(offset),
            value: bend_value(origin),
        });
    }
    if let Some(middle) = gp_note.bend_middle_value {
        let default_offset = (GP_POSITION_GPX / 2.0).round() as i32;
        if gp_note.bend_middle_offset_1 != Some(12) {
            let offset = gp_note.bend_middle_offset_1.unwrap_or(default_offset);
            bend.points.push(BendPoint {
                position: bend_position(offset),
                value: bend_value(middle),
            });
        }
        if gp_note.bend_middle_offset_2 != Some(12) {
            let offset = gp_note.bend_middle_offset_2.unwrap_or(default_offset);
            bend.points.push(BendPoint {
                position: bend_position(offset),
                value: bend_value(middle),
            });
        }
    }
    if let Some(offset) = gp_note.bend_destination_offset
        && (offset as f32) < GP_POSITION_GPX
    {
        bend.points.push(BendPoint {
            position: bend_position(offset),
            value: bend_value(destination),
        });
    }
    bend.points.push(BendPoint {
        position: BEND_EFFECT_MAX_POSITION_LENGTH as u8,
        value: bend_value(destination),
    });
    // The MIDI builder walks points in order and assumes monotonic positions.
    bend.points.sort_by_key(|p| p.position);
    Some(bend)
}

fn tremolo_bar_value(gp_value: i32) -> i8 {
    let value = (gp_value as f32 / GP_WHAMMY_SEMITONE).round() as i32;
    value.clamp(
        -i32::from(TREMOLO_BAR_MAX_VALUE),
        i32::from(TREMOLO_BAR_MAX_VALUE),
    ) as i8
}

fn tremolo_bar_of(gp_beat: &GpxBeat) -> Option<TremoloBarEffect> {
    if !gp_beat.whammy_bar_enabled {
        return None;
    }
    let origin = gp_beat.whammy_bar_origin_value?;
    let destination = gp_beat.whammy_bar_destination_value?;

    let mut tremolo = TremoloBarEffect::default();
    tremolo.points.push(BendPoint {
        position: 0,
        value: tremolo_bar_value(origin),
    });
    if let Some(offset) = gp_beat.whammy_bar_origin_offset {
        tremolo.points.push(BendPoint {
            position: bend_position(offset),
            value: tremolo_bar_value(origin),
        });
    }
    if let Some(middle) = gp_beat.whammy_bar_middle_value {
        let default_offset = (GP_POSITION_GPX / 2.0).round() as i32;
        let origin_offset = gp_beat.whammy_bar_origin_offset;
        let offset1 = gp_beat.whammy_bar_middle_offset_1.unwrap_or(default_offset);
        if origin_offset.is_none_or(|o| offset1 >= o) {
            tremolo.points.push(BendPoint {
                position: bend_position(offset1),
                value: tremolo_bar_value(middle),
            });
        }
        let offset2 = gp_beat.whammy_bar_middle_offset_2.unwrap_or(default_offset);
        if origin_offset.is_none_or(|o| offset1 >= o && offset2 > offset1) {
            tremolo.points.push(BendPoint {
                position: bend_position(offset2),
                value: tremolo_bar_value(middle),
            });
        }
    }
    if let Some(offset) = gp_beat.whammy_bar_destination_offset
        && (offset as f32) < GP_POSITION_GPX
    {
        tremolo.points.push(BendPoint {
            position: bend_position(offset),
            value: tremolo_bar_value(destination),
        });
    }
    tremolo.points.push(BendPoint {
        position: BEND_EFFECT_MAX_POSITION_LENGTH as u8,
        value: tremolo_bar_value(destination),
    });
    tremolo.points.sort_by_key(|p| p.position);
    Some(tremolo)
}

fn grace_of(
    previous_beat: Option<&GpxBeat>,
    previous_grace_notes: &[&GpxNote],
    previous_duration: &Duration,
    current_note: &GpxNote,
) -> Option<GraceEffect> {
    let previous_beat = previous_beat?;
    let grace_type = previous_beat.grace_notes.as_deref()?;
    let grace_note = previous_grace_notes
        .iter()
        .find(|n| n.string == current_note.string)?;

    let duration = match previous_duration.value {
        v if v == u16::from(DURATION_SIXTEENTH) => 4,
        v if v == u16::from(DURATION_THIRTY_SECOND) => 2,
        v if v == u16::from(DURATION_SIXTY_FOURTH) => 1,
        _ => 1,
    };

    let transition = if grace_note.bend_enabled {
        GraceEffectTransition::Bend
    } else if grace_note.slide {
        GraceEffectTransition::Slide
    } else if grace_note.hammer {
        GraceEffectTransition::Hammer
    } else {
        GraceEffectTransition::None
    };

    Some(GraceEffect {
        duration,
        fret: grace_note.fret.max(0) as i8,
        is_dead: grace_note.muted_enabled,
        is_on_beat: grace_type == "OnBeat",
        transition,
        velocity: dynamic_velocity(previous_beat.dynamic.as_deref()),
    })
}

/// Right-align an incomplete (anacrusis) first measure so its notes end on the
/// measure boundary, matching Tuxguitar's `fixFirstMeasureStartPositions`.
fn fix_first_measure_start_positions(measure: &mut Measure, measure_start: u32, length: u32) {
    let measure_end = measure_start + length;
    // Tuxguitar measures the extent of every real beat of a non-empty voice
    // (rests included), not just beats carrying notes.
    let mut max_note_end = 0;
    for voice in &measure.voices {
        for beat in &voice.beats {
            max_note_end = max_note_end.max(beat.start + beat.duration.time());
        }
    }
    if max_note_end == 0 || max_note_end >= measure_end {
        return;
    }
    let movement = measure_end - max_note_end;
    for voice in &mut measure.voices {
        for beat in &mut voice.beats {
            beat.start += movement;
        }
    }
}
