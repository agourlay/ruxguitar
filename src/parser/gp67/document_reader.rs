//! GPX XML (`score.gpif`) reader.
//!
//! Parses the GPIF XML into the intermediate [`GpxDocument`] model. Port of the
//! GP6 path of Tuxguitar's `GPXDocumentReader`. The result is consumed by the
//! mapping into the engine `Song` model (Layer 3).

use crate::RuxError;
use crate::parser::gp67::document::{
    DEFAULT_PERCUSSION_CHANNEL, GpxAutomation, GpxBar, GpxBeat, GpxChord, GpxDocument,
    GpxMasterBar, GpxNote, GpxRhythm, GpxTrack, GpxVoice,
};
use roxmltree::{Document, Node};

/// GPIF schema flavour. GP6 (`.gpx`) and GP7 (`.gp`) share most of the document
/// but differ in how a track's MIDI assignment and properties are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpifVersion {
    Gp6,
    Gp7,
}

/// Parse a `score.gpif` XML document into the intermediate model.
pub fn read_document(xml: &str, version: GpifVersion) -> Result<GpxDocument, RuxError> {
    let doc = Document::parse(xml)
        .map_err(|e| RuxError::ParsingError(format!("invalid GPIF XML: {e}")))?;
    let root = doc.root_element();

    let mut gpx = GpxDocument::default();
    read_score(root, &mut gpx);
    read_automations(root, &mut gpx);
    read_tracks(root, &mut gpx, version);
    read_master_bars(root, &mut gpx);
    read_bars(root, &mut gpx);
    read_voices(root, &mut gpx);
    read_beats(root, &mut gpx);
    read_notes(root, &mut gpx);
    read_rhythms(root, &mut gpx);
    Ok(gpx)
}

fn read_score(root: Node, gpx: &mut GpxDocument) {
    let Some(score_node) = child(root, "Score") else {
        return;
    };
    let s = &mut gpx.score;
    s.title = child_text(score_node, "Title");
    s.sub_title = child_text(score_node, "SubTitle");
    s.artist = child_text(score_node, "Artist");
    s.album = child_text(score_node, "Album");
    s.words = child_text(score_node, "Words");
    s.music = child_text(score_node, "Music");
    s.words_and_music = child_text(score_node, "WordsAndMusic");
    s.copyright = child_text(score_node, "Copyright");
    s.tabber = child_text(score_node, "Tabber");
    s.instructions = child_text(score_node, "Instructions");
    s.notices = child_text(score_node, "Notices");
}

fn read_automations(root: Node, gpx: &mut GpxDocument) {
    let Some(master_track) = child(root, "MasterTrack") else {
        return;
    };
    let Some(automations) = child(master_track, "Automations") else {
        return;
    };
    for node in element_children(automations, "Automation") {
        gpx.automations.push(GpxAutomation {
            kind: child_text(node, "Type"),
            bar_id: child_int(node, "Bar", 0),
            value: child_int_array(node, "Value", ' ').unwrap_or_default(),
            linear: child_bool(node, "Linear"),
            position: child_int(node, "Position", 0),
            visible: child_bool(node, "Visible"),
        });
    }
}

fn read_tracks(root: Node, gpx: &mut GpxDocument, version: GpifVersion) {
    let Some(tracks) = child(root, "Tracks") else {
        return;
    };
    for node in element_children(tracks, "Track") {
        let mut track = GpxTrack {
            id: attr_int(node, "id"),
            name: child_text(node, "Name")
                .unwrap_or_default()
                .replace('\n', "")
                .trim()
                .to_string(),
            color: child_int_array(node, "Color", ' '),
            ..Default::default()
        };

        match version {
            GpifVersion::Gp6 => read_track_midi_gp6(node, &mut track, gpx),
            GpifVersion::Gp7 => read_track_midi_gp7(node, &mut track, gpx),
        }

        // GP6 keeps track properties directly under <Properties>; GP7 nests them
        // under <Staves><Staff><Properties>.
        let properties = match version {
            GpifVersion::Gp6 => child(node, "Properties"),
            GpifVersion::Gp7 => child(node, "Staves")
                .and_then(|staves| child(staves, "Staff"))
                .and_then(|staff| child(staff, "Properties")),
        };

        if let Some(properties) = properties {
            for property in element_children(properties, "Property") {
                match attr(property, "name") {
                    Some("Tuning") => {
                        track.tuning_pitches = child_int_array(property, "Pitches", ' ');
                    }
                    Some("CapoFret") => {
                        track.capo = child_int(property, "Fret", 0);
                    }
                    _ => {}
                }
            }
            read_chords(properties, gpx);
        }

        gpx.tracks.push(track);
    }
}

/// GP6 stores the MIDI assignment under `<GeneralMidi>`.
fn read_track_midi_gp6(node: Node, track: &mut GpxTrack, gpx: &GpxDocument) {
    if let Some(gm) = child(node, "GeneralMidi") {
        track.gm_program = child_int(gm, "Program", 0);
        track.gm_channel_1 = child_int_opt(gm, "PrimaryChannel")
            .unwrap_or_else(|| free_gm_channel(&gpx.tracks, None));
        track.gm_channel_2 = child_int_opt(gm, "SecondaryChannel")
            .unwrap_or_else(|| free_gm_channel(&gpx.tracks, Some(track)));
    }
}

/// GP7 stores the program under `<Sounds><Sound><MIDI>` and the channels under
/// `<MidiConnection>`; when channels are missing it guesses a percussion track.
fn read_track_midi_gp7(node: Node, track: &mut GpxTrack, gpx: &GpxDocument) {
    let (mut primary, mut secondary) = (-1, -1);
    if let Some(connection) = child(node, "MidiConnection") {
        primary = child_int(connection, "PrimaryChannel", -1);
        secondary = child_int(connection, "SecondaryChannel", -1);
    }
    if let Some(sounds) = child(node, "Sounds") {
        for sound in element_children(sounds, "Sound") {
            if let Some(midi) = child(sound, "MIDI") {
                track.gm_program = child_int(midi, "Program", 0);
            }
        }
    }

    if primary >= 0 && secondary >= 0 {
        track.gm_channel_1 = primary;
        track.gm_channel_2 = secondary;
        return;
    }

    // Unusual .gp file: channels are not defined, so guess percussion.
    let mut is_percussion = false;
    if let Some(instrument_set) = child(node, "InstrumentSet") {
        is_percussion |=
            child_text(instrument_set, "Name").is_some_and(|s| s.to_lowercase().contains("drum"));
        is_percussion |=
            child_text(instrument_set, "Type").is_some_and(|s| s.to_lowercase().contains("drum"));
    }
    is_percussion &= track.gm_program == 0;

    if is_percussion {
        track.gm_channel_1 = DEFAULT_PERCUSSION_CHANNEL;
        track.gm_channel_2 = DEFAULT_PERCUSSION_CHANNEL;
    } else {
        track.gm_channel_1 = free_gm_channel(&gpx.tracks, None);
        track.gm_channel_2 = free_gm_channel(&gpx.tracks, Some(track));
    }
}

fn read_chords(properties: Node, gpx: &mut GpxDocument) {
    for property in element_children(properties, "Property") {
        if attr(property, "name") != Some("DiagramCollection") {
            continue;
        }
        let Some(items) = child(property, "Items") else {
            continue;
        };
        for item in element_children(items, "Item") {
            let Some(diagram) = child(item, "Diagram") else {
                continue;
            };
            let Some(fret_count) = attr_int_opt(diagram, "fretCount") else {
                continue;
            };
            let mut chord = GpxChord {
                id: attr_int(item, "id"),
                name: attr(item, "name").map(str::to_string),
                string_count: attr_int_opt(diagram, "stringCount"),
                fret_count: Some(fret_count),
                base_fret: attr_int_opt(diagram, "baseFret"),
                frets: vec![None; fret_count.max(0) as usize],
            };
            for fret in element_children(diagram, "Fret") {
                if let Some(string) = attr_int_opt(fret, "string")
                    && string > 0
                    && string <= fret_count
                {
                    chord.frets[(string - 1) as usize] = attr_int_opt(fret, "fret");
                }
            }
            gpx.chords.push(chord);
        }
    }
}

fn read_master_bars(root: Node, gpx: &mut GpxDocument) {
    let Some(master_bars) = child(root, "MasterBars") else {
        return;
    };
    for node in element_children(master_bars, "MasterBar") {
        let mut mbar = GpxMasterBar {
            bar_ids: child_int_array(node, "Bars", ' ').unwrap_or_default(),
            time: child_int_array(node, "Time", '/'),
            triplet_feel: child_text(node, "TripletFeel"),
            alternate_endings: child_int_array(node, "AlternateEndings", ' '),
            ..Default::default()
        };

        if let Some(repeat) = child(node, "Repeat") {
            mbar.repeat_start = attr_bool(repeat, "start");
            if attr_bool(repeat, "end") {
                mbar.repeat_count = attr_int(repeat, "count") - 1;
            }
        }
        if let Some(key) = child(node, "Key") {
            mbar.accidental_count = child_int(key, "AccidentalCount", 0);
            mbar.mode = child_text(key, "Mode");
        }
        if let Some(section) = child(node, "Section") {
            mbar.marker_text =
                child_text(section, "Text").map(|t| t.replace('\n', "").trim().to_string());
        }

        gpx.master_bars.push(mbar);
    }
}

fn read_bars(root: Node, gpx: &mut GpxDocument) {
    let Some(bars) = child(root, "Bars") else {
        return;
    };
    for node in element_children(bars, "Bar") {
        gpx.bars.push(GpxBar {
            id: attr_int(node, "id"),
            voice_ids: child_int_array(node, "Voices", ' ').unwrap_or_default(),
            clef: child_text(node, "Clef"),
            simile_mark: child_text(node, "SimileMark"),
        });
    }
}

fn read_voices(root: Node, gpx: &mut GpxDocument) {
    let Some(voices) = child(root, "Voices") else {
        return;
    };
    for node in element_children(voices, "Voice") {
        gpx.voices.push(GpxVoice {
            id: attr_int(node, "id"),
            beat_ids: child_int_array(node, "Beats", ' ').unwrap_or_default(),
        });
    }
}

fn read_beats(root: Node, gpx: &mut GpxDocument) {
    let Some(beats) = child(root, "Beats") else {
        return;
    };
    for node in element_children(beats, "Beat") {
        let mut beat = GpxBeat {
            id: attr_int(node, "id"),
            dynamic: child_text(node, "Dynamic"),
            text: child_text(node, "FreeText").unwrap_or_default(),
            rhythm_id: child(node, "Rhythm")
                .and_then(|r| attr_int_opt(r, "ref"))
                .unwrap_or(0),
            tremolo: child_int_array(node, "Tremolo", '/'),
            note_ids: child_int_array(node, "Notes", ' '),
            chord_id: child_int_opt(node, "Chord"),
            fadding: child_text(node, "Fadding"),
            grace_notes: child_text(node, "GraceNotes"),
            ..Default::default()
        };

        if let Some(properties) = child(node, "Properties") {
            for property in element_children(properties, "Property") {
                match attr(property, "name") {
                    Some("PickStroke") => {
                        beat.pick_stroke = child_text(property, "Direction").unwrap_or_default();
                    }
                    Some("Brush") => {
                        beat.brush = child_text(property, "Direction").unwrap_or_default();
                    }
                    Some("Slapped") => beat.slapped = child(property, "Enable").is_some(),
                    Some("Popped") => beat.popped = child(property, "Enable").is_some(),
                    Some("WhammyBar") => {
                        beat.whammy_bar_enabled = child(property, "Enable").is_some();
                    }
                    Some("WhammyBarOriginValue") => {
                        beat.whammy_bar_origin_value = child_int_opt(property, "Float");
                    }
                    Some("WhammyBarMiddleValue") => {
                        beat.whammy_bar_middle_value = child_int_opt(property, "Float");
                    }
                    Some("WhammyBarDestinationValue") => {
                        beat.whammy_bar_destination_value = child_int_opt(property, "Float");
                    }
                    Some("WhammyBarOriginOffset") => {
                        beat.whammy_bar_origin_offset = child_int_opt(property, "Float");
                    }
                    Some("WhammyBarMiddleOffset1") => {
                        beat.whammy_bar_middle_offset_1 = child_int_opt(property, "Float");
                    }
                    Some("WhammyBarMiddleOffset2") => {
                        beat.whammy_bar_middle_offset_2 = child_int_opt(property, "Float");
                    }
                    Some("WhammyBarDestinationOffset") => {
                        beat.whammy_bar_destination_offset = child_int_opt(property, "Float");
                    }
                    _ => {}
                }
            }
        }

        gpx.beats.push(beat);
    }
}

fn read_notes(root: Node, gpx: &mut GpxDocument) {
    let Some(notes) = child(root, "Notes") else {
        return;
    };
    for node in element_children(notes, "Note") {
        let mut note = GpxNote {
            id: attr_int(node, "id"),
            tie_destination: child(node, "Tie")
                .is_some_and(|t| attr(t, "destination") == Some("true")),
            accent: child_int(node, "Accent", 0),
            trill: child_int(node, "Trill", 0),
            let_ring: child(node, "LetRing").is_some(),
            vibrato: child(node, "Vibrato").is_some(),
            ..Default::default()
        };

        if let Some(anti_accent) = child_text(node, "AntiAccent") {
            note.ghost = anti_accent == "Normal";
        }

        if let Some(properties) = child(node, "Properties") {
            for property in element_children(properties, "Property") {
                match attr(property, "name") {
                    Some("String") => note.string = child_int(property, "String", 0),
                    Some("Fret") => note.fret = child_int(property, "Fret", 0),
                    Some("Midi") => note.midi_number = child_int(property, "Number", 0),
                    Some("Tone") => note.tone = child_int(property, "Step", 0),
                    Some("Octave") => note.octave = child_int(property, "Number", 0),
                    Some("Element") => note.element = child_int(property, "Element", 0),
                    Some("Variation") => note.variation = child_int(property, "Variation", 0),
                    Some("Muted") => note.muted_enabled = child(property, "Enable").is_some(),
                    Some("PalmMuted") => {
                        note.palm_muted_enabled = child(property, "Enable").is_some();
                    }
                    Some("Slide") => {
                        note.slide = true;
                        note.slide_flags = child_int(property, "Flags", 0);
                    }
                    Some("Tapped") => note.tapped = child(property, "Enable").is_some(),
                    Some("Bended") => note.bend_enabled = child(property, "Enable").is_some(),
                    Some("BendOriginValue") => {
                        note.bend_origin_value = child_int_opt(property, "Float");
                    }
                    Some("BendMiddleValue") => {
                        note.bend_middle_value = child_int_opt(property, "Float");
                    }
                    Some("BendDestinationValue") => {
                        note.bend_destination_value = child_int_opt(property, "Float");
                    }
                    Some("BendOriginOffset") => {
                        note.bend_origin_offset = child_int_opt(property, "Float");
                    }
                    Some("BendMiddleOffset1") => {
                        note.bend_middle_offset_1 = child_int_opt(property, "Float");
                    }
                    Some("BendMiddleOffset2") => {
                        note.bend_middle_offset_2 = child_int_opt(property, "Float");
                    }
                    Some("BendDestinationOffset") => {
                        note.bend_destination_offset = child_int_opt(property, "Float");
                    }
                    Some("HopoOrigin") => note.hammer = true,
                    Some("HarmonicFret") => note.harmonic_fret = child_int(property, "HFret", 0),
                    Some("HarmonicType") => {
                        note.harmonic_type = child_text(property, "HType").unwrap_or_default();
                    }
                    _ => {}
                }
            }
        }

        // XProperty id 688062467 carries the trill duration (in divisions).
        if let Some(x_properties) = child(node, "XProperties") {
            for x_property in element_children(x_properties, "XProperty") {
                if attr(x_property, "id") == Some("688062467") {
                    note.trill_duration = child_int(x_property, "Int", 0);
                }
            }
        }

        gpx.notes.push(note);
    }
}

fn read_rhythms(root: Node, gpx: &mut GpxDocument) {
    let Some(rhythms) = child(root, "Rhythms") else {
        return;
    };
    for node in element_children(rhythms, "Rhythm") {
        let primary_tuplet = child(node, "PrimaryTuplet");
        let augmentation_dot = child(node, "AugmentationDot");
        gpx.rhythms.push(GpxRhythm {
            id: attr_int(node, "id"),
            note_value: child_text(node, "NoteValue"),
            primary_tuplet_den: primary_tuplet.map_or(1, |n| attr_int(n, "den")),
            primary_tuplet_num: primary_tuplet.map_or(1, |n| attr_int(n, "num")),
            augmentation_dot_count: augmentation_dot.map_or(0, |n| attr_int(n, "count")),
        });
    }
}

/// Find the lowest non-percussion MIDI channel not yet used by any track.
fn free_gm_channel(tracks: &[GpxTrack], track_to_check: Option<&GpxTrack>) -> i32 {
    let mut channel = 0;
    loop {
        channel += 1;
        let used_by_check =
            track_to_check.is_some_and(|t| t.gm_channel_1 == channel || t.gm_channel_2 == channel);
        let used_by_any = tracks
            .iter()
            .any(|t| t.gm_channel_1 == channel || t.gm_channel_2 == channel);
        if !used_by_check && !used_by_any && channel != DEFAULT_PERCUSSION_CHANNEL {
            return channel;
        }
    }
}

// --- XML navigation helpers (mirroring GPXDocumentReader's getters) ---

/// First direct element child of `node` with the given tag name.
fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|n| n.has_tag_name(name))
}

/// All direct element children of `node` with the given tag name.
fn element_children<'a>(node: Node<'a, 'a>, name: &'a str) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children().filter(move |n| n.has_tag_name(name))
}

/// Concatenated text content of an element (including CDATA).
fn element_text(node: Node) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}

/// Text content of the first child element named `name`.
fn child_text(node: Node, name: &str) -> Option<String> {
    child(node, name).map(element_text)
}

fn child_bool(node: Node, name: &str) -> bool {
    child_text(node, name).as_deref() == Some("true")
}

/// Integer content of the first child element named `name`, or `default`.
fn child_int(node: Node, name: &str, default: i32) -> i32 {
    child_int_opt(node, name).unwrap_or(default)
}

fn child_int_opt(node: Node, name: &str) -> Option<i32> {
    child_text(node, name).as_deref().and_then(to_int)
}

/// Split the child element's text on `sep` and parse each piece as an integer
/// (unparseable pieces become 0). Returns `None` when the child is absent.
fn child_int_array(node: Node, name: &str, sep: char) -> Option<Vec<i32>> {
    let text = child_text(node, name)?;
    Some(
        text.trim()
            .split(sep)
            .map(|p| to_int(p.trim()).unwrap_or(0))
            .collect(),
    )
}

fn attr<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

fn attr_int(node: Node, name: &str) -> i32 {
    attr_int_opt(node, name).unwrap_or(0)
}

fn attr_int_opt(node: Node, name: &str) -> Option<i32> {
    node.attribute(name).and_then(to_int)
}

fn attr_bool(node: Node, name: &str) -> bool {
    node.attribute(name) == Some("true")
}

/// Parse an integer the way Tuxguitar's `BigDecimal(..).intValue()` does:
/// accept plain integers and truncate decimal values (e.g. "100.0" -> 100).
fn to_int(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Ok(v) = s.parse::<i32>() {
        return Some(v);
    }
    s.parse::<f64>().ok().map(|f| f as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::gp67::file_system::GpxFileSystem;

    // Only fixtures committed to the repo (test-files/ is otherwise gitignored).
    const FIXTURES: &[&str] = &["test-files/Tyr - Evening Star.gpx"];

    fn read_fixture(path: &str) -> GpxDocument {
        let data = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let fs = GpxFileSystem::load(&data).unwrap_or_else(|e| panic!("load {path}: {e}"));
        let xml = fs.file_contents("score.gpif").expect("score.gpif");
        let xml = std::str::from_utf8(xml).expect("utf8 gpif");
        read_document(xml, GpifVersion::Gp6).unwrap_or_else(|e| panic!("read_document {path}: {e}"))
    }

    #[test]
    fn reads_all_fixtures_with_consistent_references() {
        for path in FIXTURES {
            let doc = read_fixture(path);

            assert!(!doc.tracks.is_empty(), "{path}: no tracks");
            assert!(!doc.master_bars.is_empty(), "{path}: no master bars");
            assert!(!doc.bars.is_empty(), "{path}: no bars");
            assert!(!doc.voices.is_empty(), "{path}: no voices");
            assert!(!doc.beats.is_empty(), "{path}: no beats");
            assert!(!doc.rhythms.is_empty(), "{path}: no rhythms");

            // Every id reference in the tree must resolve in the flat lists.
            for mbar in &doc.master_bars {
                for &bar_id in &mbar.bar_ids {
                    assert!(doc.bar(bar_id).is_some(), "{path}: dangling bar {bar_id}");
                }
            }
            for bar in &doc.bars {
                for &voice_id in &bar.voice_ids {
                    if voice_id >= 0 {
                        assert!(
                            doc.voice(voice_id).is_some(),
                            "{path}: dangling voice {voice_id}"
                        );
                    }
                }
            }
            for voice in &doc.voices {
                for &beat_id in &voice.beat_ids {
                    assert!(
                        doc.beat(beat_id).is_some(),
                        "{path}: dangling beat {beat_id}"
                    );
                }
            }
            for beat in &doc.beats {
                assert!(
                    doc.rhythm(beat.rhythm_id).is_some(),
                    "{path}: dangling rhythm"
                );
                if let Some(note_ids) = &beat.note_ids {
                    for &note_id in note_ids {
                        assert!(
                            doc.note(note_id).is_some(),
                            "{path}: dangling note {note_id}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn reads_tyr_evening_star_metadata() {
        let doc = read_fixture("test-files/Tyr - Evening Star.gpx");
        assert_eq!(doc.score.title.as_deref(), Some("Evening Star"));
        assert_eq!(doc.score.artist.as_deref(), Some("TYR"));
        assert_eq!(doc.score.album.as_deref(), Some("The Lay Of Thrym"));
        assert_eq!(doc.notes.len(), 298);

        // Tracks carry tuning + MIDI assignment.
        let track = &doc.tracks[0];
        assert!(!track.name.is_empty());
        assert!(track.tuning_pitches.as_ref().is_some_and(|t| !t.is_empty()));
    }
}
