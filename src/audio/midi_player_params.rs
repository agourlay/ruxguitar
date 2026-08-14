use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

const SOLO_NONE: i32 = -1;

/// Playback parameters shared lock-free between UI and audio callback.
pub struct MidiPlayerParams {
    tempo: AtomicU32,
    tempo_percentage: AtomicU32,
    solo_track_id: AtomicI32, // -1 == None
    mute_mask: AtomicU64,     // bit per muted track id
    metronome: AtomicBool,
    count_in: AtomicBool,
    finished: AtomicBool, // the sequence ran past its last event
    // pending count-in request: total ticks (high) | beat ticks (low), 0 = none
    count_in_request: AtomicU64,
    master_volume: AtomicU32, // f32 bits
}

impl MidiPlayerParams {
    pub fn new(tempo: u32, tempo_percentage: u32, solo_track_id: Option<usize>) -> Self {
        Self {
            tempo: AtomicU32::new(tempo),
            tempo_percentage: AtomicU32::new(tempo_percentage),
            solo_track_id: AtomicI32::new(solo_track_id.map_or(SOLO_NONE, |id| id as i32)),
            mute_mask: AtomicU64::new(0),
            metronome: AtomicBool::new(false),
            count_in: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            count_in_request: AtomicU64::new(0),
            master_volume: AtomicU32::new(1.0_f32.to_bits()),
        }
    }

    pub fn metronome_enabled(&self) -> bool {
        self.metronome.load(Ordering::Relaxed)
    }

    pub fn set_metronome(&self, enabled: bool) {
        self.metronome.store(enabled, Ordering::Relaxed);
    }

    /// Whether playback has run past the last event of the sequence.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    pub fn set_finished(&self, finished: bool) {
        self.finished.store(finished, Ordering::Relaxed);
    }

    pub fn count_in_enabled(&self) -> bool {
        self.count_in.load(Ordering::Relaxed)
    }

    pub fn set_count_in(&self, enabled: bool) {
        self.count_in.store(enabled, Ordering::Relaxed);
    }

    /// Ask the audio callback to click through a measure before playing.
    pub fn request_count_in(&self, total_ticks: u32, beat_ticks: u32) {
        let packed = (u64::from(total_ticks) << 32) | u64::from(beat_ticks);
        self.count_in_request.store(packed, Ordering::Relaxed);
    }

    /// Take the pending count-in request, if any: `(total ticks, beat ticks)`.
    pub fn take_count_in_request(&self) -> Option<(u32, u32)> {
        match self.count_in_request.swap(0, Ordering::Relaxed) {
            0 => None,
            packed => Some(((packed >> 32) as u32, packed as u32)),
        }
    }

    pub fn toggle_track_mute(&self, track_id: usize) {
        if track_id < 64 {
            self.mute_mask.fetch_xor(1 << track_id, Ordering::Relaxed);
        }
    }

    pub fn is_track_muted(&self, track_id: usize) -> bool {
        track_id < 64 && self.mute_mask.load(Ordering::Relaxed) & (1 << track_id) != 0
    }

    /// Like TuxGuitar's `shouldSend`: mute wins, then solo excludes the rest.
    pub fn is_track_audible(&self, track_id: usize) -> bool {
        if self.is_track_muted(track_id) {
            return false;
        }
        match self.solo_track_id() {
            Some(solo_track_id) => solo_track_id == track_id,
            None => true,
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
        // clamp to 1 BPM: at tempo 0 the sequencer would never advance again,
        // freezing playback with no way to reach the next tempo change event
        ((tempo as f32 * pct as f32 / 100.0) as u32).max(1)
    }

    pub fn set_tempo(&self, tempo: u32) {
        self.tempo.store(tempo, Ordering::Relaxed);
    }

    pub fn set_tempo_percentage(&self, tempo_percentage: u32) {
        self.tempo_percentage
            .store(tempo_percentage, Ordering::Relaxed);
    }
}
