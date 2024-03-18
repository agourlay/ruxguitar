use crate::audio::midi_builder::MidiBuilder;
use crate::audio::midi_event::MidiEventType;
use crate::audio::midi_player_params::MidiPlayerParams;
use crate::audio::midi_sequencer::MidiSequencer;
use crate::audio::FIRST_TICK;
use crate::parser::song_parser::Song;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::BufferSize;
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::fs::File;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tokio::sync::watch::Sender;

const SAMPLE_RATE: u32 = 44100; // number of samples per second

/// Default sound font file is embedded in the binary (6MB)
const TIMIDITY_SOUND_FONT: &[u8] = include_bytes!("../../resources/TimGM6mb.sf2");

pub struct AudioPlayer {
    is_playing: bool,
    song: Rc<Song>,                              // Song to play (shared with app)
    stream: Option<Rc<cpal::Stream>>,            // Stream is not Send & Sync
    sequencer: Arc<Mutex<MidiSequencer>>,        // Need a handle to reset sequencer
    player_params: Arc<Mutex<MidiPlayerParams>>, // Use to communicate play changes to sequencer
    synthesizer: Arc<Mutex<Synthesizer>>,        // Synthesizer for audio output
    beat_sender: Arc<Sender<usize>>,             // Notify beat changes
}

impl AudioPlayer {
    pub fn new(
        song: Rc<Song>,
        song_tempo: i32,
        sound_font_file: Option<PathBuf>,
        beat_sender: Arc<Sender<usize>>,
    ) -> Self {
        // default to no solo track
        let solo_track_id = None;

        // player params
        let player_params = Arc::new(Mutex::new(MidiPlayerParams::new(song_tempo, solo_track_id)));

        // midi sequencer initialization
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);

        // sound font setup
        let sound_font = match sound_font_file {
            Some(sound_font_file) => {
                let mut sf2 = File::open(sound_font_file).unwrap();
                SoundFont::new(&mut sf2).unwrap()
            }
            None => {
                let mut sf2 = TIMIDITY_SOUND_FONT;
                SoundFont::new(&mut sf2).unwrap()
            }
        };
        let sound_font = Arc::new(sound_font);

        let synthesizer_settings = SynthesizerSettings::new(SAMPLE_RATE as i32);
        let synthesizer_settings = Arc::new(synthesizer_settings);
        assert_eq!(synthesizer_settings.sample_rate, SAMPLE_RATE as i32);

        // build new synthesizer for the stream
        let mut synthesizer = Synthesizer::new(&sound_font, &synthesizer_settings).unwrap();

        // apply events at tick=FIRST_TICK to set up synthesizer state
        // otherwise a picking a measure *before* playing does produce the correct sound
        events
            .iter()
            .take_while(|event| event.tick == FIRST_TICK)
            .filter(|event| event.is_midi_message())
            .for_each(|event| {
                if let MidiEventType::MidiMessage(channel, command, data1, data2) = event.event {
                    synthesizer.process_midi_message(channel, command, data1, data2);
                }
            });
        let midi_sequencer = MidiSequencer::new(events);

        let synthesizer = Arc::new(Mutex::new(synthesizer));
        let sequencer = Arc::new(Mutex::new(midi_sequencer));
        Self {
            is_playing: false,
            song,
            stream: None,
            sequencer,
            player_params,
            synthesizer,
            beat_sender,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn solo_track_id(&self) -> Option<usize> {
        self.player_params.lock().unwrap().solo_track_id()
    }

    pub fn toggle_solo_mode(&mut self, new_track_id: usize) {
        let mut params_guard = self.player_params.lock().unwrap();
        if params_guard.solo_track_id() == Some(new_track_id) {
            log::info!("Disable solo mode on track {}", new_track_id);
            params_guard.set_solo_track_id(None);
        } else {
            log::info!("Enable solo mode on track {}", new_track_id);
            params_guard.set_solo_track_id(Some(new_track_id));
        }
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

        // Drop stream
        self.stream.take();
    }

    pub fn toggle_play(&mut self) {
        log::info!("Toggle audio stream");
        if let Some(ref stream) = self.stream {
            if self.is_playing {
                self.is_playing = false;
                stream.pause().unwrap();
            } else {
                self.is_playing = true;
                // reset last time to not advance time too fast on resume
                self.sequencer.lock().unwrap().reset_last_time();
                stream.play().unwrap();
            }
        } else {
            self.is_playing = true;
            let stream = new_output_stream(
                self.sequencer.clone(),
                self.player_params.clone(),
                self.synthesizer.clone(),
                self.beat_sender.clone(),
            );
            self.stream = Some(Rc::new(stream));
        }
    }

    pub fn focus_measure(&mut self, measure_id: usize) {
        log::debug!("Focus audio player on measure:{}", measure_id);
        let measure = &self.song.measure_headers[measure_id];
        let measure_start_tick = measure.start;
        let tempo = measure.tempo.value;

        // move sequencer to measure start tick
        let mut sequencer_guard = self.sequencer.lock().unwrap();
        sequencer_guard.set_tick(measure_start_tick as usize);
        drop(sequencer_guard);

        // stop current sound
        let mut synthesizer_guard = self.synthesizer.lock().unwrap();
        synthesizer_guard.note_off_all(false);
        drop(synthesizer_guard);

        // set tempo for focuses measure
        let mut player_params_guard = self.player_params.lock().unwrap();
        player_params_guard.set_tempo(tempo);
    }
}

/// Create a new output stream for audio playback.
fn new_output_stream(
    sequencer: Arc<Mutex<MidiSequencer>>,
    player_params: Arc<Mutex<MidiPlayerParams>>,
    synthesizer: Arc<Mutex<Synthesizer>>,
    beat_notifier: Arc<Sender<usize>>,
) -> cpal::Stream {
    // Initialize audio output
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();

    let config = device.default_output_config().unwrap();
    assert!(
        config.sample_format().is_float(),
        "{}",
        format!("Unsupported sample format {}", config.sample_format())
    );
    let stream_config: cpal::StreamConfig = config.into();

    let channels_count = stream_config.channels as usize;
    assert_eq!(channels_count, 2);
    assert_eq!(stream_config.sample_rate.0, SAMPLE_RATE);
    assert_eq!(stream_config.buffer_size, BufferSize::Default);

    // TODO Size initial buffer properly?
    // 4410 samples at 44100 Hz is 0.1 second
    let mono_sample_count = 4410;

    // reuse buffer for left and right channels across all calls
    let mut left: Vec<f32> = vec![0_f32; mono_sample_count];
    let mut right: Vec<f32> = vec![0_f32; mono_sample_count];

    let err_fn = |err| log::error!("an error occurred on stream: {}", err);

    let stream = device.build_output_stream(
        &stream_config,
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut player_params_guard = player_params.lock().unwrap();
            let mut sequencer_guard = sequencer.lock().unwrap();
            sequencer_guard.advance(player_params_guard.tempo());
            let mut synthesizer_guard = synthesizer.lock().unwrap();
            // process midi events for current tick
            if let Some(events) = sequencer_guard.get_next_events() {
                let tick = sequencer_guard.get_tick();
                let last_tick = sequencer_guard.get_last_tick();
                if !events.is_empty() {
                    log::debug!(
                        "Increase {} ticks [{} -> {}] ({} events)",
                        tick - last_tick,
                        last_tick,
                        tick,
                        events.len()
                    );
                }
                let solo_track_id = player_params_guard.solo_track_id();
                if events.iter().any(|event| event.is_note_event()) {
                    beat_notifier
                        .send(tick)
                        .expect("Failed to send beat notification");
                }
                for midi_event in events {
                    match midi_event.event {
                        MidiEventType::NoteOn(channel, key, velocity) => {
                            if let Some(track_id) = solo_track_id {
                                // skip note on events for other tracks in solo mode
                                if midi_event.track != Some(track_id) {
                                    continue;
                                }
                            }
                            log::debug!(
                                "Note on: channel={}, key={}, velocity={}",
                                channel,
                                key,
                                velocity
                            );
                            synthesizer_guard.note_on(channel, key, velocity as i32);
                        }
                        MidiEventType::NoteOff(channel, key) => {
                            log::debug!("Note off: channel={}, key={}", channel, key);
                            synthesizer_guard.note_off(channel, key);
                        }
                        MidiEventType::TempoChange(tempo) => {
                            log::info!("Tempo changed to {}", tempo);
                            player_params_guard.set_tempo(tempo);
                        }
                        MidiEventType::MidiMessage(channel, command, data1, data2) => {
                            log::debug!(
                                "Midi message: channel={}, command={}, data1={}, data2={}",
                                channel,
                                command,
                                data1,
                                data2
                            );
                            synthesizer_guard.process_midi_message(channel, command, data1, data2)
                        }
                    }
                }
            }
            // Split buffer in two channels (left and right)
            let channel_len = output.len() / 2;

            if left.len() < channel_len || right.len() < channel_len {
                log::warn!("Buffer too small, skipping audio rendering");
                return;
            }

            // Render the waveform.
            synthesizer_guard.render(&mut left[..channel_len], &mut right[..channel_len]);

            // Drop locks
            drop(sequencer_guard);
            drop(synthesizer_guard);
            drop(player_params_guard);

            // Interleave the left and right channels into the output buffer.
            for (i, (l, r)) in left.iter().zip(right.iter()).take(channel_len).enumerate() {
                output[i * 2] = *l;
                output[i * 2 + 1] = *r;
            }
        },
        err_fn,
        None, // blocking stream
    );
    let stream = stream.unwrap();
    stream.play().unwrap();
    stream
}
