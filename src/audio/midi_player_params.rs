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
    loop_enabled: AtomicBool,
    loop_start_tick: AtomicU32,
    loop_end_tick: AtomicU32,
    loop_start_tempo: AtomicU32, // song tempo at the loop start, restored on each pass
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
            loop_enabled: AtomicBool::new(false),
            loop_start_tick: AtomicU32::new(0),
            loop_end_tick: AtomicU32::new(0),
            loop_start_tempo: AtomicU32::new(tempo),
            count_in_request: AtomicU64::new(0),
            master_volume: AtomicU32::new(1.0_f32.to_bits()),
        }
    }

    pub fn loop_enabled(&self) -> bool {
        self.loop_enabled.load(Ordering::Relaxed)
    }

    pub fn set_loop_region(&self, start_tick: u32, end_tick: u32, start_tempo: u32) {
        self.loop_start_tick.store(start_tick, Ordering::Relaxed);
        self.loop_end_tick.store(end_tick, Ordering::Relaxed);
        self.loop_start_tempo.store(start_tempo, Ordering::Relaxed);
        self.loop_enabled.store(true, Ordering::Relaxed);
    }

    pub fn clear_loop(&self) {
        self.loop_enabled.store(false, Ordering::Relaxed);
    }

    pub fn loop_start_tick(&self) -> u32 {
        self.loop_start_tick.load(Ordering::Relaxed)
    }

    pub fn loop_end_tick(&self) -> u32 {
        self.loop_end_tick.load(Ordering::Relaxed)
    }

    pub fn loop_start_tempo(&self) -> u32 {
        self.loop_start_tempo.load(Ordering::Relaxed)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> MidiPlayerParams {
        MidiPlayerParams::new(120, 100, None)
    }

    #[test]
    fn the_tempo_follows_its_percentage() {
        let params = params();
        assert_eq!(params.adjusted_tempo(), 120);
        params.set_tempo_percentage(50);
        assert_eq!(params.adjusted_tempo(), 60);
        params.set_tempo(200);
        assert_eq!(params.adjusted_tempo(), 100);
    }

    #[test]
    fn the_tempo_never_reaches_zero() {
        let params = params();
        params.set_tempo(0);
        assert_eq!(params.adjusted_tempo(), 1);
    }

    #[test]
    fn mute_wins_over_solo() {
        let params = params();
        assert!(params.is_track_audible(0) && params.is_track_audible(1));
        params.set_solo_track_id(Some(1));
        assert!(!params.is_track_audible(0));
        assert!(params.is_track_audible(1));
        params.toggle_track_mute(1);
        assert!(!params.is_track_audible(1));
        params.toggle_track_mute(1);
        params.set_solo_track_id(None);
        assert!(params.is_track_audible(0) && params.is_track_audible(1));
    }

    #[test]
    fn tracks_past_the_mute_mask_cannot_be_muted() {
        let params = params();
        params.toggle_track_mute(64);
        assert!(!params.is_track_muted(64));
        assert!(params.is_track_audible(64));
    }

    #[test]
    fn a_count_in_request_is_taken_once() {
        let params = params();
        assert_eq!(params.take_count_in_request(), None);
        params.request_count_in(3840, 960);
        assert_eq!(params.take_count_in_request(), Some((3840, 960)));
        assert_eq!(params.take_count_in_request(), None);
    }

    #[test]
    fn the_master_volume_is_clamped() {
        let params = params();
        params.set_master_volume(1.5);
        assert_eq!(params.master_volume(), 1.0);
        params.set_master_volume(-1.0);
        assert_eq!(params.master_volume(), 0.0);
        params.set_master_volume(0.25);
        assert_eq!(params.master_volume(), 0.25);
    }

    #[test]
    fn a_loop_region_can_be_set_and_cleared() {
        let params = params();
        assert!(!params.loop_enabled());
        params.set_loop_region(960, 4800, 90);
        assert!(params.loop_enabled());
        assert_eq!(
            (
                params.loop_start_tick(),
                params.loop_end_tick(),
                params.loop_start_tempo()
            ),
            (960, 4800, 90)
        );
        params.clear_loop();
        assert!(!params.loop_enabled());
    }
}
