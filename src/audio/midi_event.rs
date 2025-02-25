#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MidiEvent {
    /// The tick at which the event occurs.
    pub tick: u32,
    /// The type of the event.
    pub event: MidiEventType,
    /// The track number of the event. None = info event. (TODO make it u8)
    pub track: Option<usize>,
}

impl MidiEvent {
    pub const fn is_midi_message(&self) -> bool {
        matches!(self.event, MidiEventType::MidiMessage(_, _, _, _))
    }

    pub const fn is_tempo_change(&self) -> bool {
        matches!(self.event, MidiEventType::TempoChange(_))
    }

    pub const fn is_note_event(&self) -> bool {
        !self.is_tempo_change() || !self.is_midi_message()
    }

    pub const fn new_note_on(
        tick: u32,
        track: usize,
        key: i32,
        velocity: i16,
        channel: i32,
    ) -> Self {
        let event = MidiEventType::note_on(channel, key, velocity);
        Self {
            tick,
            event,
            track: Some(track),
        }
    }

    pub const fn new_note_off(tick: u32, track: usize, key: i32, channel: i32) -> Self {
        let event = MidiEventType::note_off(channel, key);
        Self {
            tick,
            event,
            track: Some(track),
        }
    }

    pub const fn new_tempo_change(tick: u32, tempo: u32) -> Self {
        let event = MidiEventType::tempo_change(tempo);
        Self {
            tick,
            event,
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
        let event = MidiEventType::midi_message(channel, command, data1, data2);
        Self {
            tick,
            event,
            track: Some(track),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum MidiEventType {
    NoteOn(i32, i32, i16),           // channel, note, velocity
    NoteOff(i32, i32),               // channel, note
    TempoChange(u32),                // tempo in BPM
    MidiMessage(i32, i32, i32, i32), // channel: i32, command: i32, data1: i32, data2: i32
}

impl MidiEventType {
    const fn note_on(channel: i32, key: i32, velocity: i16) -> Self {
        Self::NoteOn(channel, key, velocity)
    }

    const fn note_off(channel: i32, key: i32) -> Self {
        Self::NoteOff(channel, key)
    }

    const fn tempo_change(tempo: u32) -> Self {
        Self::TempoChange(tempo)
    }

    const fn midi_message(channel: i32, command: i32, data1: i32, data2: i32) -> Self {
        Self::MidiMessage(channel, command, data1, data2)
    }
}
