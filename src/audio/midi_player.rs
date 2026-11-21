use crate::audio::midi_builder::{
    METRONOME_KEYS, METRONOME_TRACK, METRONOME_VELOCITY, MidiBuilder,
};
use crate::audio::midi_event::{FIRST_TICK, MidiEvent, MidiEventType};
use crate::audio::midi_player_params::MidiPlayerParams;
use crate::audio::midi_sequencer::{MidiSequencer, TickRate};
use crate::audio::playback_order::{
    MeasureLoop, first_occurrence_ticks, first_playback_ticks, measure_loop_tick_range,
};
use crate::parser::song_parser::Song;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::fs::File;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

const DEFAULT_SAMPLE_RATE: u32 = 44100; // number of samples per second

/// Default sound font file is embedded in the binary (6MB)
const TIMIDITY_SOUND_FONT: &[u8] = include_bytes!("../../resources/TimGM6mb.sf2");

pub struct AudioPlayer {
    is_playing: bool,
    /// Song to play, shared with the app.
    song: Rc<Song>,
    /// `cpal::Stream` is neither `Send` nor `Sync`.
    stream: Option<Rc<cpal::Stream>>,
    sequencer: Arc<Mutex<MidiSequencer>>,
    /// Lock-free playback parameters.
    player_params: Arc<MidiPlayerParams>,
    synthesizer: Arc<Mutex<Synthesizer>>,
    sound_font: Arc<SoundFont>,
    /// Latest tick reached by the audio callback, and the UI wake-up signal.
    current_tick: Arc<AtomicU32>,
    beat_notify: Arc<Notify>,
    /// First playback tick per measure, for seeking.
    measure_playback_ticks: Vec<u32>,
    /// First playback tick per measure, `None` when it is never played.
    measure_first_occurrences: Vec<Option<u32>>,
}

impl AudioPlayer {
    pub fn new(
        song: Rc<Song>,
        song_tempo: u32,
        tempo_percentage: u32,
        sound_font_file: Option<PathBuf>,
        current_tick: Arc<AtomicU32>,
        beat_notify: Arc<Notify>,
        playback_order: &[(usize, i64)],
    ) -> Result<Self, AudioPlayerError> {
        let player_params = Arc::new(MidiPlayerParams::new(song_tempo, tempo_percentage));

        let events = MidiBuilder::new().build_for_song_with_order(&song, playback_order);
        let measure_playback_ticks = first_playback_ticks(&song.measure_headers, playback_order);
        let measure_first_occurrences =
            first_occurrence_ticks(&song.measure_headers, playback_order);

        let sound_font = if let Some(ref sound_font_file) = sound_font_file {
            let mut sf2 = File::open(sound_font_file).map_err(|e| {
                AudioPlayerError::SoundFontFileError(format!("{}: {e}", sound_font_file.display()))
            })?;
            SoundFont::new(&mut sf2).map_err(|e| {
                AudioPlayerError::SoundFontLoadError(format!("{}: {e}", sound_font_file.display()))
            })?
        } else {
            let mut sf2 = TIMIDITY_SOUND_FONT;
            SoundFont::new(&mut sf2)
                .map_err(|e| AudioPlayerError::SoundFontLoadError(format!("embedded: {e}")))?
        };
        let sound_font = Arc::new(sound_font);
        let synthesizer = Self::make_synthesizer(sound_font.clone(), DEFAULT_SAMPLE_RATE)?;

        let synthesizer = Arc::new(Mutex::new(synthesizer));
        let sequencer = Arc::new(Mutex::new(MidiSequencer::new(events)));
        Ok(Self {
            is_playing: false,
            song,
            stream: None,
            sequencer,
            player_params,
            synthesizer,
            sound_font,
            current_tick,
            beat_notify,
            measure_playback_ticks,
            measure_first_occurrences,
        })
    }

    fn make_synthesizer(
        sound_font: Arc<SoundFont>,
        sample_rate: u32,
    ) -> Result<Synthesizer, AudioPlayerError> {
        let mut synthesizer_settings = SynthesizerSettings::new(sample_rate as i32);
        // MIDI events take effect on the next synth block. Eight frames is
        // RustySynth's smallest supported block.
        synthesizer_settings.block_size = 8;
        Synthesizer::new(&sound_font, &synthesizer_settings)
            .map_err(|e| AudioPlayerError::SynthesizerError(e.to_string()))
    }

    pub const fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn is_track_solo(&self, track_id: usize) -> bool {
        self.player_params.is_track_solo(track_id)
    }

    /// Whether playback reached the end of the song.
    pub fn is_finished(&self) -> bool {
        self.player_params.is_finished()
    }

    pub fn set_metronome(&self, enabled: bool) {
        log::info!("Metronome enabled: {enabled}");
        self.player_params.set_metronome(enabled);
    }

    pub fn set_count_in(&self, enabled: bool) {
        log::info!("Count-in enabled: {enabled}");
        self.player_params.set_count_in(enabled);
    }

    /// Ask for a count-in over the measure currently under the cursor.
    fn request_count_in(&self) {
        let current_tick = self.current_tick.load(Ordering::Relaxed);
        // the measure whose first playback tick the cursor has reached
        let measure_id = self
            .measure_playback_ticks
            .partition_point(|&t| t <= current_tick)
            .saturating_sub(1);
        let Some(header) = self.song.measure_headers.get(measure_id) else {
            return;
        };
        let beat_ticks = header.time_signature.denominator.time();
        let total_ticks = u32::from(header.time_signature.numerator) * beat_ticks;
        self.player_params.request_count_in(total_ticks, beat_ticks);
    }

    pub fn is_track_muted(&self, track_id: usize) -> bool {
        self.player_params.is_track_muted(track_id)
    }

    pub fn toggle_track_mute(&self, track_id: usize) {
        self.player_params.toggle_track_mute(track_id);
    }

    pub fn toggle_track_solo(&self, track_id: usize) {
        let enabled = !self.player_params.is_track_solo(track_id);
        log::info!(
            "{} solo on track {track_id}",
            if enabled { "Enable" } else { "Disable" }
        );
        self.player_params.toggle_track_solo(track_id);
    }

    pub fn set_tempo_percentage(&self, new_tempo_percentage: u32) {
        self.player_params
            .set_tempo_percentage(new_tempo_percentage);
    }

    /// Loop the given measures, returning the first one that plays, where
    /// each pass starts. Clears the loop when none of them ever plays.
    pub fn set_measure_loop(&self, start_measure: usize, end_measure: usize) -> Option<usize> {
        let Some(measure_loop) = measure_loop_tick_range(
            &self.song.measure_headers,
            &self.measure_first_occurrences,
            start_measure,
            end_measure,
        ) else {
            log::warn!("Cannot loop measures {start_measure}..={end_measure}: never played");
            self.player_params.clear_loop();
            return None;
        };
        let MeasureLoop {
            first_measure,
            start_tick,
            end_tick,
        } = measure_loop;
        log::info!(
            "Loop measures {start_measure}..={end_measure} (ticks {start_tick}..{end_tick})"
        );
        let start_tempo = self.song.measure_headers[first_measure].tempo.value;
        self.player_params
            .set_loop_region(start_tick, end_tick, start_tempo);
        Some(first_measure)
    }

    pub fn clear_measure_loop(&self) {
        self.player_params.clear_loop();
    }

    pub fn master_volume(&self) -> f32 {
        self.player_params.master_volume()
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.player_params.set_master_volume(volume);
    }

    /// Cut all sounding notes and recenter the pitch wheel: leaving it mid-bend
    /// would keep later notes out of tune (the bend range RPN is preserved).
    fn silence_synthesizer(&self) {
        let mut synthesizer_guard = self.synthesizer.lock().unwrap();
        synthesizer_guard.note_off_all(false);
        synthesizer_guard.reset_all_controllers();
    }

    pub fn stop(&mut self) {
        if let Some(stream) = &self.stream {
            log::debug!("Stopping audio stream");
            if let Err(err) = stream.pause() {
                // e.g. audio device disappeared; the stream is dropped below anyway
                log::warn!("Failed to pause audio stream: {err}");
            }
        }
        self.is_playing = false;

        self.sequencer.lock().unwrap().reset_ticks();
        self.player_params.set_finished(false);
        self.silence_synthesizer();

        // reset the UI cursor to the first playable tick so the measure lookup resolves cleanly
        self.current_tick.store(FIRST_TICK, Ordering::Relaxed);
        self.beat_notify.notify_one();

        self.stream.take();
    }

    /// Toggle play/pause. Returns an error message if playback fails.
    pub fn toggle_play(&mut self) -> Option<String> {
        log::debug!("Toggle audio stream");
        if let Some(ref stream) = self.stream {
            if self.is_playing {
                self.is_playing = false;
                if let Err(err) = stream.pause() {
                    return Some(format!("Failed to pause audio stream: {err}"));
                }
            } else {
                self.is_playing = true;
                if self.player_params.count_in_enabled() {
                    self.request_count_in();
                }
                if let Err(err) = stream.play() {
                    return Some(format!("Failed to resume audio stream: {err}"));
                }
            }
        } else {
            self.is_playing = true;
            if self.player_params.count_in_enabled() {
                self.request_count_in();
            }

            // Initialize audio output stream
            let stream = new_output_stream(
                self.sequencer.clone(),
                self.player_params.clone(),
                self.synthesizer.clone(),
                self.sound_font.clone(),
                self.current_tick.clone(),
                self.beat_notify.clone(),
            );

            match stream {
                Ok(stream) => {
                    self.stream = Some(Rc::new(stream));
                }
                Err(err) => {
                    self.is_playing = false;
                    self.stream = None;
                    return Some(format!("Failed to create audio stream: {err}"));
                }
            }
        }
        None
    }

    /// Seek to a measure, offset by `beat_tick_offset` ticks into it.
    pub fn focus_measure_at(&self, measure_id: usize, beat_tick_offset: u32) {
        log::debug!("Focus audio player on measure:{measure_id} (+{beat_tick_offset} ticks)");
        let measure = &self.song.measure_headers[measure_id];
        let measure_start_tick = self.measure_playback_ticks[measure_id] + beat_tick_offset;

        self.sequencer.lock().unwrap().set_tick(measure_start_tick);
        self.player_params.set_finished(false);

        // keep the cursor tick in sync: the count-in looks up the measure
        // (and its time signature) through it
        self.current_tick
            .store(measure_start_tick, Ordering::Relaxed);

        self.silence_synthesizer();
        self.player_params.set_tempo(measure.tempo.value);
    }
}

/// Send the events of one playback step to the synthesizer.
fn play_events(
    events: &[MidiEvent],
    tick: u32,
    synthesizer_guard: &mut Synthesizer,
    player_params: &MidiPlayerParams,
    current_tick: &AtomicU32,
    beat_notify: &Notify,
) {
    if !events.is_empty() {
        // debug level: runs on the real-time audio thread
        log::debug!("---> tick {tick} ({} events)", events.len());
    }
    if events.iter().any(MidiEvent::is_note_event) {
        current_tick.store(tick, Ordering::Release);
        beat_notify.notify_one();
    }
    for midi_event in events {
        // mute/solo filtering, like TuxGuitar's shouldSend: new
        // notes and channel messages of inaudible tracks are
        // skipped, note-offs always pass so nothing gets stuck,
        // and the setup events at FIRST_TICK are never filtered;
        // metronome clicks are gated solely on their toggle
        let audible = if midi_event.track == Some(METRONOME_TRACK) {
            player_params.metronome_enabled()
        } else {
            midi_event.tick == FIRST_TICK
                || midi_event
                    .track
                    .is_none_or(|t| player_params.is_track_audible(usize::from(t)))
        };
        match midi_event.event {
            MidiEventType::NoteOn(channel, key, velocity) => {
                if !audible {
                    continue;
                }
                log::debug!(
                    "[{}] Note on: channel={}, key={}, velocity={}",
                    midi_event.tick,
                    channel,
                    key,
                    velocity
                );
                synthesizer_guard.note_on(channel, key, i32::from(velocity));
            }
            MidiEventType::NoteOff(channel, key) => {
                log::debug!(
                    "[{}] Note off: channel={}, key={}",
                    midi_event.tick,
                    channel,
                    key
                );
                synthesizer_guard.note_off(channel, key);
            }
            MidiEventType::TempoChange(tempo) => {
                // debug level: runs on the real-time audio thread
                log::debug!("Tempo changed to {tempo}");
                player_params.set_tempo(tempo);
            }
            MidiEventType::MidiMessage(channel, command, data1, data2) => {
                if !audible {
                    continue;
                }
                log::debug!(
                    "[{}] Midi message: channel={}, command={}, data1={}, data2={}",
                    midi_event.tick,
                    channel,
                    command,
                    data1,
                    data2
                );
                synthesizer_guard.process_midi_message(channel, command, data1, data2);
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioPlayerError {
    #[error("audio device not found")]
    CpalDeviceNotFound,
    #[error("no output configuration found: {0}")]
    CpalOutputConfigNotFound(cpal::Error),
    #[error("failed to open sound font file: {0}")]
    SoundFontFileError(String),
    #[error("failed to load sound font: {0}")]
    SoundFontLoadError(String),
    #[error("failed to create synthesizer: {0}")]
    SynthesizerError(String),
    #[error("failed to create audio stream: {0}")]
    StreamError(String),
}

fn new_output_stream(
    sequencer: Arc<Mutex<MidiSequencer>>,
    player_params: Arc<MidiPlayerParams>,
    synthesizer: Arc<Mutex<Synthesizer>>,
    sound_font: Arc<SoundFont>,
    current_tick: Arc<AtomicU32>,
    beat_notify: Arc<Notify>,
) -> Result<cpal::Stream, AudioPlayerError> {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        return Err(AudioPlayerError::CpalDeviceNotFound);
    };

    let config = device
        .default_output_config()
        .map_err(AudioPlayerError::CpalOutputConfigNotFound)?;

    if !config.sample_format().is_float() {
        return Err(AudioPlayerError::StreamError(format!(
            "Unsupported sample format {}",
            config.sample_format()
        )));
    }
    let stream_config: cpal::StreamConfig = config.into();
    let sample_rate = stream_config.sample_rate;
    let channel_count = usize::from(stream_config.channels).max(1);

    log::info!("Audio output stream config: {stream_config:?}");

    {
        let mut synthesizer_guard = synthesizer.lock().unwrap();
        if synthesizer_guard.get_sample_rate() != sample_rate as i32 {
            // A newly opened device can use a different sample rate.
            *synthesizer_guard = AudioPlayer::make_synthesizer(sound_font, sample_rate)?;
        }
        prime_synthesizer(&sequencer.lock().unwrap(), &mut synthesizer_guard);
    }

    let mut renderer = StreamRenderer::new(
        sequencer,
        player_params,
        synthesizer,
        current_tick,
        beat_notify,
        sample_rate,
        channel_count,
    );
    let err_fn = |err| log::error!("an error occurred on stream: {err}");
    let stream = device.build_output_stream(
        stream_config,
        move |output: &mut [f32], info: &cpal::OutputCallbackInfo| {
            renderer.render(output, info.timestamp());
        },
        err_fn,
        None, // blocking stream
    );
    let stream = stream.map_err(|e| AudioPlayerError::StreamError(e.to_string()))?;
    stream
        .play()
        .map_err(|e| AudioPlayerError::StreamError(e.to_string()))?;
    Ok(stream)
}

/// Apply the channel setup events at `FIRST_TICK`, so that seeking to a
/// measure before playing already sounds with the right instruments.
fn prime_synthesizer(sequencer: &MidiSequencer, synthesizer: &mut Synthesizer) {
    sequencer
        .events()
        .iter()
        .take_while(|event| event.tick == FIRST_TICK)
        .filter(|event| event.is_midi_message())
        .for_each(|event| {
            if let MidiEventType::MidiMessage(channel, command, data1, data2) = event.event {
                synthesizer.process_midi_message(channel, command, data1, data2);
            }
        });
}

/// A one measure count-in, clicking every beat before playback resumes.
#[derive(Debug, Default)]
struct CountIn {
    /// Ticks left to count.
    left: f64,
    /// Ticks per beat.
    beat: f64,
    /// Ticks until the next click.
    next_click: f64,
}

impl CountIn {
    fn start(&mut self, total_ticks: u32, beat_ticks: u32) {
        self.left = f64::from(total_ticks);
        self.beat = f64::from(beat_ticks);
        self.next_click = 0.0;
    }

    fn is_active(&self) -> bool {
        self.left > 0.0
    }

    /// Advance by `ticks`: whether a click is due at the start of them.
    fn advance(&mut self, ticks: f64) -> bool {
        let click = self.next_click <= 0.0;
        if click {
            self.next_click += self.beat;
        }
        self.left -= ticks;
        self.next_click -= ticks;
        click
    }
}

/// The audio callback's state: steps the sequencer, plays its events and
/// renders the synthesizer into the device buffer.
struct StreamRenderer {
    sequencer: Arc<Mutex<MidiSequencer>>,
    player_params: Arc<MidiPlayerParams>,
    synthesizer: Arc<Mutex<Synthesizer>>,
    current_tick: Arc<AtomicU32>,
    beat_notify: Arc<Notify>,
    /// Render buffers, reused across calls.
    left: Vec<f32>,
    right: Vec<f32>,
    channel_count: usize,
    sample_rate: u32,
    count_in: CountIn,
    /// When the device will have played the callback buffer containing the end.
    finish_after: Option<cpal::StreamInstant>,
}

impl StreamRenderer {
    fn new(
        sequencer: Arc<Mutex<MidiSequencer>>,
        player_params: Arc<MidiPlayerParams>,
        synthesizer: Arc<Mutex<Synthesizer>>,
        current_tick: Arc<AtomicU32>,
        beat_notify: Arc<Notify>,
        sample_rate: u32,
        channel_count: usize,
    ) -> Self {
        // The buffers hold 0.1 second of audio,
        // e.g. 4410 samples at 44100 Hz
        let channel_sample_count = (sample_rate / 10) as usize;
        Self {
            sequencer,
            player_params,
            synthesizer,
            current_tick,
            beat_notify,
            left: vec![0_f32; channel_sample_count],
            right: vec![0_f32; channel_sample_count],
            channel_count,
            sample_rate,
            count_in: CountIn::default(),
            finish_after: None,
        }
    }

    fn render(&mut self, output: &mut [f32], timestamp: cpal::OutputStreamTimestamp) {
        let capacity = self.left.len() * self.channel_count;
        let frames = output.len() / self.channel_count;
        let duration = Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate));
        let buffer_end = timestamp.playback + duration;
        for chunk in output.chunks_mut(capacity) {
            self.render_chunk(chunk, timestamp.callback, buffer_end);
        }
    }

    fn render_chunk(
        &mut self,
        output: &mut [f32],
        now: cpal::StreamInstant,
        buffer_end: cpal::StreamInstant,
    ) {
        let player_params = &self.player_params;
        let frames = output.len() / self.channel_count;
        let mut sequencer_guard = self.sequencer.lock().unwrap();
        let mut synthesizer_guard = self.synthesizer.lock().unwrap();

        // a pending count-in clicks through one measure before playback
        if let Some((total_ticks, beat_ticks)) = player_params.take_count_in_request() {
            self.count_in.start(total_ticks, beat_ticks);
        }
        if self.count_in.is_active() {
            self.finish_after = None;
            let rate = TickRate::new(player_params.adjusted_tempo(), self.sample_rate);
            let ticks = rate.ticks_for_frames(frames);
            if self.count_in.advance(ticks) {
                // same layered click as the metronome
                for key in METRONOME_KEYS {
                    synthesizer_guard.note_on(9, key, i32::from(METRONOME_VELOCITY));
                }
            }
            synthesizer_guard.render(&mut self.left[..frames], &mut self.right[..frames]);
        } else {
            let loop_range = player_params
                .loop_enabled()
                .then(|| {
                    (
                        player_params.loop_start_tick(),
                        player_params.loop_end_tick(),
                    )
                })
                .filter(|&(start, end)| start < end);
            let mut rendered = 0;
            while rendered < frames {
                if loop_range.is_some() || !sequencer_guard.is_exhausted() {
                    self.finish_after = None;
                } else if !player_params.is_finished() {
                    match self.finish_after {
                        Some(deadline) if now >= deadline => {
                            player_params.set_finished(true);
                            self.beat_notify.notify_one();
                        }
                        None => self.finish_after = Some(buffer_end),
                        _ => {}
                    }
                }
                let rate = TickRate::new(player_params.adjusted_tempo(), self.sample_rate);
                let count = sequencer_guard.frames_until_event(frames - rendered, rate, loop_range);
                let end = rendered + count;
                synthesizer_guard.render(
                    &mut self.left[rendered..end],
                    &mut self.right[rendered..end],
                );
                rendered = end;

                let step = sequencer_guard.step(rate, count, loop_range);
                let events = sequencer_guard.events();
                if let Some(range) = step.events {
                    play_events(
                        &events[range],
                        step.tick,
                        &mut synthesizer_guard,
                        player_params,
                        &self.current_tick,
                        &self.beat_notify,
                    );
                }
                if let Some(range) = step.restart {
                    synthesizer_guard.note_off_all(false);
                    synthesizer_guard.reset_all_controllers();
                    player_params.set_tempo(player_params.loop_start_tempo());
                    player_params.set_finished(false);
                    play_events(
                        &events[range],
                        sequencer_guard.get_tick(),
                        &mut synthesizer_guard,
                        player_params,
                        &self.current_tick,
                        &self.beat_notify,
                    );
                }
            }
        }

        let master_volume = player_params.master_volume();

        // Drop locks
        drop(sequencer_guard);
        drop(synthesizer_guard);

        write_frames(
            output,
            &self.left[..frames],
            &self.right[..frames],
            self.channel_count,
            master_volume,
        );
    }
}

/// Interleave rendered stereo samples into the device's frame layout.
///
/// Mono devices get a downmix, channels beyond stereo are zeroed.
/// Frames past the rendered samples are silenced explicitly: the output
/// buffer is not guaranteed to be zeroed and would replay stale samples.
fn write_frames(
    output: &mut [f32],
    left: &[f32],
    right: &[f32],
    channel_count: usize,
    master_volume: f32,
) {
    let rendered = left.len().min(right.len());
    let mut frames = output.chunks_exact_mut(channel_count);
    for (i, frame) in frames.by_ref().enumerate() {
        let (l, r) = if i < rendered {
            (left[i] * master_volume, right[i] * master_volume)
        } else {
            (0.0, 0.0)
        };
        match frame {
            [mono] => *mono = (l + r) / 2.0,
            [first, second, rest @ ..] => {
                *first = l;
                *second = r;
                rest.fill(0.0);
            }
            [] => {}
        }
    }
    // leftover samples when the buffer is not a whole number of frames
    frames.into_remainder().fill(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Advance a count-in in steps of `ticks` until it ends, returning the
    /// tick of each click.
    fn count_in_clicks(total: u32, beat: u32, ticks: f64) -> Vec<f64> {
        let mut count_in = CountIn::default();
        count_in.start(total, beat);
        let mut clicks = Vec::new();
        let mut elapsed = 0.0;
        while count_in.is_active() {
            if count_in.advance(ticks) {
                clicks.push(elapsed);
            }
            elapsed += ticks;
        }
        clicks
    }

    #[test]
    fn a_count_in_clicks_every_beat_of_the_measure() {
        let clicks = count_in_clicks(4 * 960, 960, 240.0);
        assert_eq!(clicks, [0.0, 960.0, 1920.0, 2880.0]);
    }

    #[test]
    fn a_count_in_with_coarse_steps_clicks_on_the_first_step_past_each_beat() {
        // steps longer than a beat cannot click more than once each
        let clicks = count_in_clicks(3 * 960, 960, 1000.0);
        assert_eq!(clicks, [0.0, 1000.0, 2000.0]);
    }

    #[test]
    fn no_count_in_until_one_is_requested() {
        assert!(!CountIn::default().is_active());
    }

    #[test]
    fn write_frames_stereo_applies_volume() {
        let left = [1.0, 0.5];
        let right = [-1.0, 0.25];
        let mut output = [9.0_f32; 4];
        write_frames(&mut output, &left, &right, 2, 0.5);
        assert_eq!(output, [0.5, -0.5, 0.25, 0.125]);
    }

    #[test]
    fn write_frames_mono_downmixes() {
        let left = [1.0, 0.5];
        let right = [0.5, 0.25];
        let mut output = [9.0_f32; 2];
        write_frames(&mut output, &left, &right, 1, 1.0);
        assert_eq!(output, [0.75, 0.375]);
    }

    #[test]
    fn write_frames_zeroes_extra_channels() {
        let left = [1.0];
        let right = [0.5];
        // 4-channel device: one frame, extra channels silenced
        let mut output = [9.0_f32; 4];
        write_frames(&mut output, &left, &right, 4, 1.0);
        assert_eq!(output, [1.0, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn write_frames_silences_unrendered_tail() {
        let left = [1.0];
        let right = [0.5];
        // device asks for 3 frames but only 1 was rendered:
        // the stale tail must be silenced, not replayed
        let mut output = [9.0_f32; 6];
        write_frames(&mut output, &left, &right, 2, 1.0);
        assert_eq!(output, [1.0, 0.5, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn write_frames_zeroes_partial_frame_remainder() {
        let left = [1.0];
        let right = [0.5];
        // 5 samples on a stereo device: the dangling half-frame is silenced
        let mut output = [9.0_f32; 5];
        write_frames(&mut output, &left, &right, 2, 1.0);
        assert_eq!(output, [1.0, 0.5, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn metronome_click_produces_sound() {
        fn click_energy(sound_font: &Arc<SoundFont>, with_channel_setup: bool) -> f32 {
            let settings = Arc::new(SynthesizerSettings::new(44100));
            let mut synth = Synthesizer::new(sound_font, &settings).unwrap();
            if with_channel_setup {
                synth.process_midi_message(9, 0xB0, 0x00, 0); // bank select (drum bank 128)
                synth.process_midi_message(9, 0xC0, 0, 0); // program 0
            }
            for key in METRONOME_KEYS {
                synth.note_on(9, key, i32::from(METRONOME_VELOCITY));
            }
            let mut l = vec![0f32; 4410];
            let mut r = vec![0f32; 4410];
            synth.render(&mut l, &mut r);
            l.iter().map(|s| s.abs()).sum()
        }

        let mut sf2: &[u8] = TIMIDITY_SOUND_FONT;
        let sound_font = Arc::new(SoundFont::new(&mut sf2).unwrap());

        // fresh channel 9 is percussion by default
        assert!(
            click_energy(&sound_font, false) > 0.01,
            "no sound on fresh percussion channel"
        );
        // ... and stays audible after the app's channel 9 setup
        // (bank select 0 + program 0, like a drum track)
        assert!(
            click_energy(&sound_font, true) > 0.01,
            "no sound after channel 9 setup"
        );
    }

    #[test]
    fn metronome_events_flow_through_the_sequencer() {
        use crate::audio::midi_builder::MidiBuilder;
        use crate::audio::midi_sequencer::MidiSequencer;
        use crate::parser::song_parser_tests::parse_gp_file;
        use std::rc::Rc;

        let song = parse_gp_file("test-files/Demo v5.gp5").unwrap();
        let tempo = song.tempo.value;
        let song = Rc::new(song);
        let events = MidiBuilder::new().build_for_song(&song);
        let params = MidiPlayerParams::new(tempo, 100);
        params.set_metronome(true);

        let mut sequencer = MidiSequencer::new(events);
        sequencer.advance(TickRate::new(tempo, DEFAULT_SAMPLE_RATE), 0); // init
        let mut clicks = 0;
        for _ in 0..2000 {
            sequencer.advance(
                TickRate::new(params.adjusted_tempo(), DEFAULT_SAMPLE_RATE),
                (DEFAULT_SAMPLE_RATE / 10) as usize,
            );
            let Some(events) = sequencer.get_next_events() else {
                break;
            };
            for midi_event in events {
                // same gating as the audio callback
                let audible = if midi_event.track == Some(METRONOME_TRACK) {
                    params.metronome_enabled()
                } else {
                    true
                };
                if audible && matches!(midi_event.event, MidiEventType::NoteOn(9, 75, _)) {
                    clicks += 1;
                }
            }
        }
        assert!(clicks > 50, "expected metronome clicks, got {clicks}");
    }

    fn test_sound_font() -> Arc<SoundFont> {
        let mut sf2 = TIMIDITY_SOUND_FONT;
        Arc::new(SoundFont::new(&mut sf2).unwrap())
    }

    fn test_renderer(
        font: &Arc<SoundFont>,
        sample_rate: u32,
        events: Vec<MidiEvent>,
    ) -> StreamRenderer {
        let mut sequence = MidiSequencer::new(events);
        sequence.set_tick(1000);
        StreamRenderer::new(
            Arc::new(Mutex::new(sequence)),
            Arc::new(MidiPlayerParams::new(120, 100)),
            Arc::new(Mutex::new(
                AudioPlayer::make_synthesizer(font.clone(), sample_rate).unwrap(),
            )),
            Arc::new(AtomicU32::new(0)),
            Arc::new(Notify::new()),
            sample_rate,
            2,
        )
    }

    fn timestamp(callback_ms: u64, playback_ms: u64) -> cpal::OutputStreamTimestamp {
        cpal::OutputStreamTimestamp {
            callback: cpal::StreamInstant::from_millis(callback_ms),
            playback: cpal::StreamInstant::from_millis(playback_ms),
        }
    }

    #[test]
    fn finish_waits_for_device_playback_and_a_seek_cancels_the_old_deadline() {
        let font = test_sound_font();
        let mut renderer = test_renderer(
            &font,
            DEFAULT_SAMPLE_RATE,
            vec![
                MidiEvent::new_note_on(1000, 0, 60, 100, 0),
                MidiEvent::new_note_off(1001, 0, 60, 0),
            ],
        );
        // 200 ms spans two internal chunks. The device starts playing it at 40 ms.
        let mut output = vec![0.0; 2 * DEFAULT_SAMPLE_RATE as usize / 5];
        renderer.render(&mut output, timestamp(0, 40));
        assert!(!renderer.player_params.is_finished());
        let mut small = [0.0; 64];
        renderer.render(&mut small, timestamp(239, 240));
        assert!(!renderer.player_params.is_finished());

        renderer.sequencer.lock().unwrap().set_tick(1000);
        renderer.render(&mut output, timestamp(240, 250));
        assert!(!renderer.player_params.is_finished());
        renderer.render(&mut small, timestamp(449, 450));
        assert!(!renderer.player_params.is_finished());
        renderer.render(&mut small, timestamp(450, 451));
        assert!(renderer.player_params.is_finished());
    }

    #[test]
    fn tempo_changes_render_the_same_audio_across_callback_sizes_and_sample_rates() {
        let font = test_sound_font();
        for sample_rate in [DEFAULT_SAMPLE_RATE, 48_000] {
            // 64 ticks at 120 BPM and 32 ticks at 60 BPM each last 1/30 second.
            let first = sample_rate as usize / 30;
            let second = first * 2;
            let release = second + first / 2;
            let frames = sample_rate as usize / 10;
            let mut left = vec![0.0; frames];
            let mut right = vec![0.0; frames];
            let mut reference = AudioPlayer::make_synthesizer(font.clone(), sample_rate).unwrap();
            reference.render(&mut left[..first], &mut right[..first]);
            reference.note_on(0, 60, 100);
            reference.render(&mut left[first..second], &mut right[first..second]);
            reference.note_off(0, 60);
            reference.note_on(0, 67, 100);
            reference.render(&mut left[second..release], &mut right[second..release]);
            reference.note_off(0, 67);
            reference.render(&mut left[release..], &mut right[release..]);

            for callback_frames in [frames, 127] {
                let mut renderer = test_renderer(
                    &font,
                    sample_rate,
                    vec![
                        MidiEvent::new_note_on(1064, 0, 60, 100, 0),
                        MidiEvent::new_tempo_change(1064, 60),
                        MidiEvent::new_note_off(1096, 0, 60, 0),
                        MidiEvent::new_note_on(1096, 0, 67, 100, 0),
                        MidiEvent::new_note_off(1112, 0, 67, 0),
                    ],
                );
                let mut output = vec![0.0; frames * 2];
                for buffer in output.chunks_mut(callback_frames * 2) {
                    renderer.render(buffer, timestamp(0, 0));
                }
                for (i, &frame) in output.as_chunks::<2>().0.iter().enumerate() {
                    assert_eq!(
                        frame,
                        [left[i], right[i]],
                        "frame {i}, buffer {callback_frames}, rate {sample_rate}"
                    );
                }
            }
        }
    }

    #[test]
    fn audible_note_onsets_are_within_seven_frames_of_the_scheduled_frame() {
        let font = test_sound_font();
        for sample_rate in [DEFAULT_SAMPLE_RATE, 48_000] {
            let onset = |ticks| {
                let mut renderer = test_renderer(
                    &font,
                    sample_rate,
                    vec![MidiEvent::new_note_on(1000 + ticks, 0, 60, 100, 0)],
                );
                let mut output = [0.0; 512];
                for chunk in output.chunks_mut(22) {
                    renderer.render(chunk, timestamp(0, 0));
                }
                output
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .position(|stereo| stereo.iter().any(|&s| s != 0.0))
                    .unwrap()
            };
            let attack = onset(0);
            for ticks in 1..=8 {
                // At 120 BPM the score advances by 1920 ticks per second.
                let frame = (ticks * sample_rate).div_ceil(1920) as usize;
                let delay = onset(ticks) - attack;
                assert!(
                    (frame..=frame + 7).contains(&delay),
                    "tick {ticks} at {sample_rate} Hz: expected frame {frame}, heard {delay}"
                );
            }
        }
    }

    #[test]
    fn seeking_updates_the_cursor_tick() {
        use crate::audio::playback_order::compute_playback_order;
        use crate::parser::song_parser_tests::parse_gp_file;
        use std::rc::Rc;
        use tokio::sync::Notify;

        let song = parse_gp_file("test-files/Demo v5.gp5").unwrap();
        let tempo = song.tempo.value;
        let song = Rc::new(song);
        let order = compute_playback_order(&song.measure_headers);
        let current_tick = Arc::new(AtomicU32::new(FIRST_TICK));
        let notify = Arc::new(Notify::new());
        let player =
            AudioPlayer::new(song, tempo, 100, None, current_tick.clone(), notify, &order).unwrap();

        // the count-in resolves the measure signature through the cursor
        // tick: seeking must move it
        player.focus_measure_at(10, 0);
        assert_eq!(
            current_tick.load(Ordering::Relaxed),
            player.measure_playback_ticks[10]
        );

        // beat-level seek lands inside the same measure
        player.focus_measure_at(10, 480);
        assert_eq!(
            current_tick.load(Ordering::Relaxed),
            player.measure_playback_ticks[10] + 480
        );
    }
}
