/// Hold values changed during playback of a MIDI events.
pub struct MidiPlayerParams {
    tempo: u32,
    tempo_percentage: u32,
    solo_track_id: Option<usize>,
}

impl MidiPlayerParams {
    pub const fn new(tempo: u32, tempo_percentage: u32, solo_track_id: Option<usize>) -> Self {
        Self {
            tempo,
            tempo_percentage,
            solo_track_id,
        }
    }

    pub const fn solo_track_id(&self) -> Option<usize> {
        self.solo_track_id
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn set_solo_track_id(&mut self, solo_track_id: Option<usize>) {
        self.solo_track_id = solo_track_id;
    }

    pub fn adjusted_tempo(&self) -> u32 {
        (self.tempo as f32 * self.tempo_percentage as f32 / 100.0) as u32
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn set_tempo(&mut self, tempo: u32) {
        self.tempo = tempo;
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn set_tempo_percentage(&mut self, tempo_percentage: u32) {
        self.tempo_percentage = tempo_percentage;
    }
}
