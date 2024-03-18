use uuid::Uuid;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MidiEvent {
    pub id: Uuid, // unique id for testing purpose
    pub tick: usize,
    pub event: MidiEventType,
    pub track: Option<usize>, // None = info event
}

impl MidiEvent {
    pub fn is_midi_message(&self) -> bool {
        matches!(self.event, MidiEventType::MidiMessage(_, _, _, _))
    }

    pub fn is_tempo_change(&self) -> bool {
        matches!(self.event, MidiEventType::TempoChange(_))
    }

    pub fn is_note_event(&self) -> bool {
        !self.is_tempo_change() || !self.is_midi_message()
    }

    pub fn new_note_on(tick: usize, track: usize, key: i32, velocity: i16, channel: i32) -> Self {
        let event = MidiEventType::note_on(channel, key, velocity);
        let id = Uuid::new_v4();
        Self {
            id,
            tick,
            event,
            track: Some(track),
        }
    }

    pub fn new_note_off(tick: usize, track: usize, key: i32, channel: i32) -> Self {
        let event = MidiEventType::note_off(channel, key);
        let id = Uuid::new_v4();
        Self {
            id,
            tick,
            event,
            track: Some(track),
        }
    }

    pub fn new_tempo_change(tick: usize, tempo: i32) -> Self {
        let event = MidiEventType::tempo_change(tempo);
        let id = Uuid::new_v4();
        Self {
            id,
            tick,
            event,
            track: None,
        }
    }

    pub fn new_midi_message(
        tick: usize,
        track: usize,
        channel: i32,
        command: i32,
        data1: i32,
        data2: i32,
    ) -> Self {
        let event = MidiEventType::midi_message(channel, command, data1, data2);
        let id = Uuid::new_v4();
        Self {
            id,
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
    TempoChange(i32),                // tempo in BPM
    MidiMessage(i32, i32, i32, i32), // channel: i32, command: i32, data1: i32, data2: i32
}

impl MidiEventType {
    fn note_on(channel: i32, key: i32, velocity: i16) -> Self {
        Self::NoteOn(channel, key, velocity)
    }

    fn note_off(channel: i32, key: i32) -> Self {
        Self::NoteOff(channel, key)
    }

    fn tempo_change(tempo: i32) -> Self {
        Self::TempoChange(tempo)
    }

    fn midi_message(channel: i32, command: i32, data1: i32, data2: i32) -> Self {
        Self::MidiMessage(channel, command, data1, data2)
    }
}
