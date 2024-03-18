/// Hold values changed during playback of a MIDI events.
pub struct MidiPlayerParams {
    tempo: i32,
    solo_track_id: Option<usize>,
}

impl MidiPlayerParams {
    pub fn new(tempo: i32, solo_track_id: Option<usize>) -> Self {
        Self {
            tempo,
            solo_track_id,
        }
    }

    pub fn solo_track_id(&self) -> Option<usize> {
        self.solo_track_id
    }

    pub fn set_solo_track_id(&mut self, solo_track_id: Option<usize>) {
        self.solo_track_id = solo_track_id;
    }

    pub fn tempo(&self) -> i32 {
        self.tempo
    }

    pub fn set_tempo(&mut self, tempo: i32) {
        self.tempo = tempo;
    }
}
