/// First tick of a song.
pub const FIRST_TICK: u32 = 1;

/// A MIDI event.
/// Try to keep this struct as small as possible because there will be a lot of them.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MidiEvent {
    pub tick: u32,
    pub event: MidiEventType,
    /// `None` for info events (tempo changes), which belong to no track.
    pub track: Option<u8>,
}

impl MidiEvent {
    pub const fn is_midi_message(&self) -> bool {
        matches!(self.event, MidiEventType::MidiMessage(_, _, _, _))
    }

    pub const fn is_note_event(&self) -> bool {
        matches!(
            self.event,
            MidiEventType::NoteOn(_, _, _) | MidiEventType::NoteOff(_, _)
        )
    }

    pub const fn new_note_on(
        tick: u32,
        track: usize,
        key: i32,
        velocity: i16,
        channel: i32,
    ) -> Self {
        Self {
            tick,
            event: MidiEventType::NoteOn(channel, key, velocity),
            track: Some(track as u8),
        }
    }

    pub const fn new_note_off(tick: u32, track: usize, key: i32, channel: i32) -> Self {
        Self {
            tick,
            event: MidiEventType::NoteOff(channel, key),
            track: Some(track as u8),
        }
    }

    pub const fn new_tempo_change(tick: u32, tempo: u32) -> Self {
        Self {
            tick,
            event: MidiEventType::TempoChange(tempo),
            track: None,
        }
    }

    pub const fn new_midi_message(
        tick: u32,
        track: usize,
        channel: i32,
        command: i32,
        data1: i32,
        data2: i32,
    ) -> Self {
        Self {
            tick,
            event: MidiEventType::MidiMessage(channel, command, data1, data2),
            track: Some(track as u8),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum MidiEventType {
    NoteOn(i32, i32, i16),           // channel, note, velocity
    NoteOff(i32, i32),               // channel, note
    TempoChange(u32),                // tempo in BPM
    MidiMessage(i32, i32, i32, i32), // channel, command, data1, data2
}
