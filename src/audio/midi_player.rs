use crate::audio::FIRST_TICK;
use crate::audio::midi_builder::MidiBuilder;
use crate::audio::midi_event::MidiEventType;
use crate::audio::midi_player_params::MidiPlayerParams;
use crate::audio::midi_sequencer::MidiSequencer;
use crate::parser::song_parser::Song;
use cpal::DefaultStreamConfigError;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::fs::File;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

const DEFAULT_SAMPLE_RATE: u32 = 44100; // number of samples per second

/// Default sound font file is embedded in the binary (6MB)
const TIMIDITY_SOUND_FONT: &[u8] = include_bytes!("../../resources/TimGM6mb.sf2");

pub struct AudioPlayer {
    is_playing: bool,
    song: Rc<Song>,                       // Song to play (shared with app)
    stream: Option<Rc<cpal::Stream>>,     // Stream is not Send & Sync
    sequencer: Arc<Mutex<MidiSequencer>>, // Need a handle to reset sequencer
    player_params: Arc<MidiPlayerParams>, // Lock-free playback parameters
    synthesizer: Arc<Mutex<Synthesizer>>, // Synthesizer for audio output
    sound_font: Arc<SoundFont>,           // Sound font for synthesizer
    current_tick: Arc<AtomicU32>,         // Latest tick reached by the audio callback
    beat_notify: Arc<Notify>,             // Wake UI when current_tick changes
    measure_playback_ticks: Vec<u32>,     // first playback tick per measure (for seeking)
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
        // default to no solo track
        let solo_track_id = None;

        // player params
        let player_params = Arc::new(MidiPlayerParams::new(
            song_tempo,
            tempo_percentage,
            solo_track_id,
        ));

        // midi sequencer initialization
        let builder = MidiBuilder::new();
        let events = builder.build_for_song_with_order(&song, playback_order);

        // build first-playback-tick lookup per measure (for seeking)
        let measure_count = song.measure_headers.len();
        let mut measure_playback_ticks = vec![0_u32; measure_count];
        let mut seen = vec![false; measure_count];
        for &(measure_index, tick_offset) in playback_order {
            if !seen[measure_index] {
                seen[measure_index] = true;
                let header = &song.measure_headers[measure_index];
                measure_playback_ticks[measure_index] =
                    (i64::from(header.start) + tick_offset) as u32;
            }
        }

        // sound font setup
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

        // build new default synthesizer for the stream
        let synthesizer = Self::make_synthesizer(sound_font.clone(), DEFAULT_SAMPLE_RATE)?;
        let midi_sequencer = MidiSequencer::new(events);

        let synthesizer = Arc::new(Mutex::new(synthesizer));
        let sequencer = Arc::new(Mutex::new(midi_sequencer));
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
        })
    }

    fn make_synthesizer(
        sound_font: Arc<SoundFont>,
        sample_rate: u32,
    ) -> Result<Synthesizer, AudioPlayerError> {
        let synthesizer_settings = SynthesizerSettings::new(sample_rate as i32);
        let synthesizer_settings = Arc::new(synthesizer_settings);
        debug_assert_eq!(synthesizer_settings.sample_rate, sample_rate as i32);
        Synthesizer::new(&sound_font, &synthesizer_settings)
            .map_err(|e| AudioPlayerError::SynthesizerError(e.to_string()))
    }

    pub const fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn solo_track_id(&self) -> Option<usize> {
        self.player_params.solo_track_id()
    }

    pub fn toggle_solo_mode(&self, new_track_id: usize) {
        if self.player_params.solo_track_id() == Some(new_track_id) {
            log::info!("Disable solo mode on track {new_track_id}");
            self.player_params.set_solo_track_id(None);
        } else {
            log::info!("Enable solo mode on track {new_track_id}");
            self.player_params.set_solo_track_id(Some(new_track_id));
        }
    }

    pub fn set_tempo_percentage(&self, new_tempo_percentage: u32) {
        self.player_params
            .set_tempo_percentage(new_tempo_percentage);
    }

    pub fn master_volume(&self) -> f32 {
        self.player_params.master_volume()
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.player_params.set_master_volume(volume);
    }

    pub fn stop(&mut self) {
        // Pause stream
        if let Some(stream) = &self.stream {
            log::info!("Stopping audio stream");
            stream.pause().unwrap();
        }
        self.is_playing = false;

        // reset ticks
        let mut sequencer_guard = self.sequencer.lock().unwrap();
        sequencer_guard.reset_last_time();
        sequencer_guard.reset_ticks();
        drop(sequencer_guard);

        // stop all sound in synthesizer
        let mut synthesizer_guard = self.synthesizer.lock().unwrap();
        synthesizer_guard.note_off_all(false);
        drop(synthesizer_guard);

        // reset the UI cursor to the first playable tick so the measure lookup resolves cleanly
        self.current_tick.store(FIRST_TICK, Ordering::Relaxed);
        self.beat_notify.notify_one();

        // Drop stream
        self.stream.take();
    }

    /// Toggle play/pause. Returns an error message if playback fails.
    pub fn toggle_play(&mut self) -> Option<String> {
        log::info!("Toggle audio stream");
        if let Some(ref stream) = self.stream {
            if self.is_playing {
                self.is_playing = false;
                if let Err(err) = stream.pause() {
                    return Some(format!("Failed to pause audio stream: {err}"));
                }
            } else {
                self.is_playing = true;
                // reset last time to not advance time too fast on resume
                self.sequencer.lock().unwrap().reset_last_time();
                if let Err(err) = stream.play() {
                    return Some(format!("Failed to resume audio stream: {err}"));
                }
            }
        } else {
            self.is_playing = true;

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

    pub fn focus_measure(&self, measure_id: usize) {
        log::debug!("Focus audio player on measure:{measure_id}");
        let measure = &self.song.measure_headers[measure_id];
        let measure_start_tick = self.measure_playback_ticks[measure_id];
        let tempo = measure.tempo.value;

        // move sequencer to measure start tick
        let mut sequencer_guard = self.sequencer.lock().unwrap();
        sequencer_guard.set_tick(measure_start_tick);
        drop(sequencer_guard);

        // stop current sound
        let mut synthesizer_guard = self.synthesizer.lock().unwrap();
        synthesizer_guard.note_off_all(false);
        drop(synthesizer_guard);

        // set tempo for focuses measure
        self.player_params.set_tempo(tempo);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioPlayerError {
    #[error("audio device not found")]
    CpalDeviceNotFound,
    #[error("no output configuration found: {0}")]
    CpalOutputConfigNotFound(DefaultStreamConfigError),
    #[error("failed to open sound font file: {0}")]
    SoundFontFileError(String),
    #[error("failed to load sound font: {0}")]
    SoundFontLoadError(String),
    #[error("failed to create synthesizer: {0}")]
    SynthesizerError(String),
    #[error("failed to create audio stream: {0}")]
    StreamError(String),
}

/// Create a new output stream for audio playback.
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

    log::info!("Audio output stream config: {stream_config:?}");

    let mut synthesizer_guard = synthesizer.lock().unwrap();
    if sample_rate != DEFAULT_SAMPLE_RATE {
        // audio output is not using the default sample rate - recreate synthesizer with proper sample rate
        let new_synthesizer = AudioPlayer::make_synthesizer(sound_font, sample_rate)?;
        *synthesizer_guard = new_synthesizer;
    }

    // Apply events at tick=FIRST_TICK to set up synthesizer state
    // otherwise clicking on a measure *before* playing does not produce the correct instrument sound
    sequencer
        .lock()
        .unwrap()
        .events()
        .iter()
        .take_while(|event| event.tick == FIRST_TICK)
        .filter(|event| event.is_midi_message())
        .for_each(|event| {
            if let MidiEventType::MidiMessage(channel, command, data1, data2) = event.event {
                synthesizer_guard.process_midi_message(channel, command, data1, data2);
            }
        });

    drop(synthesizer_guard);

    // Size left and right buffers according to sample rate.
    // The buffer accounts for 0.1 second of audio.
    // e.g. 4410 samples at 44100 Hz is 0.1 second
    let channel_sample_count = sample_rate / 10;

    // reuse buffer for left and right channels across all calls
    let mut left: Vec<f32> = vec![0_f32; channel_sample_count as usize];
    let mut right: Vec<f32> = vec![0_f32; channel_sample_count as usize];

    let err_fn = |err| log::error!("an error occurred on stream: {err}");

    let stream = device.build_output_stream(
        &stream_config,
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut sequencer_guard = sequencer.lock().unwrap();
            sequencer_guard.advance(player_params.adjusted_tempo());
            let mut synthesizer_guard = synthesizer.lock().unwrap();
            // process midi events for current tick
            if let Some(events) = sequencer_guard.get_next_events() {
                let tick = sequencer_guard.get_tick();
                let last_tick = sequencer_guard.get_last_tick();
                if !events.is_empty() {
                    log::debug!(
                        "---> Increase {} ticks [{} -> {}] ({} events)",
                        tick - last_tick,
                        last_tick,
                        tick,
                        events.len()
                    );
                }
                let solo_track_id = player_params.solo_track_id();
                if events
                    .iter()
                    .any(super::midi_event::MidiEvent::is_note_event)
                {
                    current_tick.store(tick, Ordering::Release);
                    beat_notify.notify_one();
                }
                for midi_event in events {
                    match midi_event.event {
                        MidiEventType::NoteOn(channel, key, velocity) => {
                            if let Some(track_id) = solo_track_id {
                                // skip note on events for other tracks in solo mode
                                if midi_event.track != Some(track_id as u8) {
                                    continue;
                                }
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
                            log::info!("Tempo changed to {tempo}");
                            player_params.set_tempo(tempo);
                        }
                        MidiEventType::MidiMessage(channel, command, data1, data2) => {
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
            // Split buffer for this run between left and right
            let mut output_channel_len = output.len() / 2;

            if left.len() < output_channel_len || right.len() < output_channel_len {
                log::info!(
                    "Output buffer larger than expected channel size {} > {}",
                    output_channel_len,
                    left.len()
                );
                output_channel_len = left.len();
            }

            // Render the waveform.
            synthesizer_guard.render(
                &mut left[..output_channel_len],
                &mut right[..output_channel_len],
            );

            let master_volume = player_params.master_volume();

            // Drop locks
            drop(sequencer_guard);
            drop(synthesizer_guard);

            // Interleave the left and right channels into the output buffer.
            for i in 0..output_channel_len {
                output[i * 2] = left[i] * master_volume;
                output[i * 2 + 1] = right[i] * master_volume;
            }
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
