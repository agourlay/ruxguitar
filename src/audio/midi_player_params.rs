use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

const SOLO_NONE: i32 = -1;

/// Playback parameters shared lock-free between UI and audio callback.
pub struct MidiPlayerParams {
    tempo: AtomicU32,
    tempo_percentage: AtomicU32,
    solo_track_id: AtomicI32, // -1 == None
    master_volume: AtomicU32, // f32 bits
}

impl MidiPlayerParams {
    pub fn new(tempo: u32, tempo_percentage: u32, solo_track_id: Option<usize>) -> Self {
        Self {
            tempo: AtomicU32::new(tempo),
            tempo_percentage: AtomicU32::new(tempo_percentage),
            solo_track_id: AtomicI32::new(solo_track_id.map_or(SOLO_NONE, |id| id as i32)),
            master_volume: AtomicU32::new(1.0_f32.to_bits()),
        }
    }

    pub fn master_volume(&self) -> f32 {
        f32::from_bits(self.master_volume.load(Ordering::Relaxed))
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.master_volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn solo_track_id(&self) -> Option<usize> {
        match self.solo_track_id.load(Ordering::Relaxed) {
            SOLO_NONE => None,
            id => Some(id as usize),
        }
    }

    pub fn set_solo_track_id(&self, solo_track_id: Option<usize>) {
        self.solo_track_id.store(
            solo_track_id.map_or(SOLO_NONE, |id| id as i32),
            Ordering::Relaxed,
        );
    }

    pub fn adjusted_tempo(&self) -> u32 {
        let tempo = self.tempo.load(Ordering::Relaxed);
        let pct = self.tempo_percentage.load(Ordering::Relaxed);
        (tempo as f32 * pct as f32 / 100.0) as u32
    }

    pub fn set_tempo(&self, tempo: u32) {
        self.tempo.store(tempo, Ordering::Relaxed);
    }

    pub fn set_tempo_percentage(&self, tempo_percentage: u32) {
        self.tempo_percentage
            .store(tempo_percentage, Ordering::Relaxed);
    }
}
