/// Hold values changed during playback of a MIDI events.
pub struct MidiPlayerParams {
    tempo: u32,
    tempo_percentage: u32,
    solo_track_id: Option<usize>,
    repeat: Option<Repeat>, // current repeat
}

impl MidiPlayerParams {
    pub const fn new(tempo: u32, tempo_percentage: u32, solo_track_id: Option<usize>) -> Self {
        Self {
            tempo,
            tempo_percentage,
            solo_track_id,
            repeat: None,
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

    #[allow(clippy::missing_const_for_fn)]
    pub fn get_repeat(&self) -> Option<&Repeat> {
        self.repeat.as_ref()
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = Some(repeat);
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn decrease_play_count(&mut self) {
        let mut purge_repeat = false;
        if let Some(repeat) = self.repeat.as_mut() {
            // check if the repeat needs to be removed
            if repeat.play_count == 1 {
                purge_repeat = true;
            }
            repeat.decrease_play_count();
        }
        if purge_repeat {
            self.repeat = None;
        }
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn unset_repeat(&mut self) {
        self.repeat = None;
    }
}

// Holds data describing a measure repeation sequence
#[derive(Default, Debug, Clone)]
pub struct Repeat {
    pub back_to: u32,                 // time to get back to
    pub play_count: u8,               // how many times to play the sequence
    pub end_time: u32,                // the end time of the repeated measure
    pub alternative_repeat: Vec<u32>, // time to use for the last measure
}

impl Repeat {
    #[allow(clippy::missing_const_for_fn)]
    pub fn decrease_play_count(&mut self) {
        if self.play_count > 0 {
            self.play_count -= 1;
        }
    }
}
