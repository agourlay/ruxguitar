use iced::advanced::text::Shaping::Auto;
use iced::widget::operation::scroll_to;
use iced::widget::space::horizontal;
use iced::widget::{Id, Text, column, container, pick_list, row, rule, selector, text};
use iced::{Alignment, Border, Element, Size, Subscription, Task, Theme, keyboard, stream, window};
use std::fmt::Display;

use crate::ApplicationArgs;
use crate::audio::midi_player::AudioPlayer;
use crate::audio::playback_order::compute_playback_order;
use crate::config::Config;
use crate::parser::song_parser::{GpVersion, MeasureHeader, QUARTER_TIME, Song, parse_gp_data};
use crate::ui::icons::{open_icon, pause_icon, play_icon, solo_icon, stop_icon};
use crate::ui::picker::{FilePickerError, load_file, open_file_dialog};
use crate::ui::tablature::Tablature;
use crate::ui::tuning::tuning_label;
use crate::ui::utils::{action_gated, action_toggle, modal, untitled_text_table_box};
use iced::futures::{SinkExt, Stream};
use iced::keyboard::key::Named::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, F11, Space};
use iced::widget::scrollable::AbsoluteOffset;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::watch::{Receiver, Sender};

const ICONS_FONT: &[u8] = include_bytes!("../../resources/icons.ttf");

pub struct RuxApplication {
    song_info: Option<SongDisplayInfo>,       // parsed song
    track_selection: TrackSelection,          // selected track
    all_tracks: Vec<TrackSelection>,          // all possible tracks
    tablature: Option<Tablature>,             // loaded tablature
    tablature_id: Id,                         // tablature container id
    tempo_selection: TempoSelection,          // tempo percentage for playback
    audio_player: Option<AudioPlayer>,        // audio player
    tab_file_is_loading: bool,                // file loading flag in progress
    sound_font_file: Option<PathBuf>,         // sound font file
    beat_sender: Arc<Sender<u32>>,            // beat notifier
    beat_receiver: Arc<Mutex<Receiver<u32>>>, // beat receiver
    config: Config,                           // local configuration
    error_message: Option<String>,            // error message to display
    is_fullscreen: bool,                      // F11 toggles fullscreen + hides chrome
}

#[derive(Debug)]
struct SongDisplayInfo {
    name: String,
    artist: String,
    subtitle: String,
    album: String,
    author: String,
    writer: String,
    copyright: String,
    gp_version: GpVersion,
    file_name: String,
}

impl SongDisplayInfo {
    fn new(song: &Song, file_name: String) -> Self {
        Self {
            name: song.song_info.name.clone(),
            artist: song.song_info.artist.clone(),
            subtitle: song.song_info.subtitle.clone(),
            album: song.song_info.album.clone(),
            author: song.song_info.author.clone(),
            writer: song.song_info.writer.clone(),
            copyright: song.song_info.copyright.clone(),
            gp_version: song.version,
            file_name,
        }
    }

    /// Metadata fields joined with " \u{2022} ", skipping empty ones.
    /// Returns `None` if no metadata is available.
    fn metadata_line(&self) -> Option<String> {
        let parts: Vec<&str> = [
            self.subtitle.as_str(),
            self.album.as_str(),
            self.author.as_str(),
            self.writer.as_str(),
            self.copyright.as_str(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" \u{2022} "))
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TempoSelection {
    percentage: u32,
}

impl Default for TempoSelection {
    fn default() -> Self {
        Self::new(100)
    }
}

impl TempoSelection {
    const fn new(percentage: u32) -> Self {
        Self { percentage }
    }

    const PRESET: [Self; 9] = {
        [
            Self::new(25),
            Self::new(50),
            Self::new(60),
            Self::new(70),
            Self::new(80),
            Self::new(90),
            Self::new(100),
            Self::new(150),
            Self::new(200),
        ]
    };
}

impl Display for TempoSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.percentage)
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TrackSelection {
    index: usize,
    name: String,
    tuning: Option<String>,
}

impl TrackSelection {
    const fn new(index: usize, name: String, tuning: Option<String>) -> Self {
        Self {
            index,
            name,
            tuning,
        }
    }
}

impl Display for TrackSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} - {}", self.index + 1, self.name)?;
        if let Some(tuning) = &self.tuning {
            write!(f, " ({tuning})")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenFileDialog,    // open file dialog
    OpenFile(PathBuf), // open file path
    FileOpened(Result<(Vec<u8>, Option<PathBuf>, String), FilePickerError>), // file content, parent folder & file name
    TrackSelected(TrackSelection),                                           // track selection
    FocusMeasure(usize),           // used when clicking on measure in tablature
    FocusTick(u32),                // focus on a specific tick in the tablature
    NextMeasure,                   // focus next measure
    PreviousMeasure,               // focus previous measure
    PlayPause,                     // toggle play/pause
    StopPlayer,                    // stop playback
    ToggleSolo,                    // toggle solo mode
    WindowResized,                 // window resized
    TablatureResized(Size),        // tablature resized
    TempoSelected(TempoSelection), // tempo selected
    IncreaseTempo,                 // increase tempo
    DecreaseTempo,                 // decrease selection
    ClearError,                    // clear error message
    ReportError(String),           // report error message
    ToggleFullscreen,              // toggle fullscreen + hide chrome
}

impl RuxApplication {
    fn new(sound_font_file: Option<PathBuf>, config: Config) -> Self {
        let (beat_sender, beat_receiver) = tokio::sync::watch::channel(0_u32);
        Self {
            song_info: None,
            track_selection: TrackSelection::default(),
            all_tracks: vec![],
            tablature: None,
            tablature_id: Id::new("tablature-outer-container"),
            tempo_selection: TempoSelection::default(),
            audio_player: None,
            tab_file_is_loading: false,
            sound_font_file,
            beat_receiver: Arc::new(Mutex::new(beat_receiver)),
            beat_sender: Arc::new(beat_sender),
            config,
            error_message: None,
            is_fullscreen: false,
        }
    }

    fn boot(args: &ApplicationArgs) -> (Self, Task<Message>) {
        let app = Self::new(args.sound_font_bank.clone(), args.local_config.clone());

        let init_task = args
            .tab_file_path
            .as_ref()
            .map_or_else(Task::none, |f| Task::done(Message::OpenFile(f.clone())));
        (app, init_task)
    }

    pub fn start(args: ApplicationArgs) -> iced::Result {
        let antialiasing = !args.no_antialiasing;
        iced::application(move || Self::boot(&args), Self::update, Self::view)
            .title(Self::title)
            .subscription(Self::subscription)
            .default_font(iced::Font::MONOSPACE)
            .theme(Self::theme)
            .font(ICONS_FONT)
            .window_size((1150.0, 768.0))
            .centered()
            .antialiasing(antialiasing)
            .run()
    }

    fn title(&self) -> String {
        match &self.song_info {
            Some(song_info) => format!("Ruxguitar - {}", song_info.file_name),
            None => String::from("Ruxguitar - untitled"),
        }
    }

    fn focus_measure_with_scroll(&mut self, measure_id: usize) -> Task<Message> {
        let Some(tablature) = &mut self.tablature else {
            return Task::none();
        };
        tablature.focus_on_measure(measure_id);
        let scroll_offset = tablature.scroll_offset_for_measure(measure_id);
        let scroll_id = tablature.scroll_id.clone();
        if let Some(audio_player) = &self.audio_player {
            audio_player.focus_measure(measure_id);
        }
        scroll_offset.map_or_else(Task::none, |y| {
            scroll_to(scroll_id, AbsoluteOffset { x: 0.0, y })
        })
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TrackSelected(selection) => {
                if let Some(tablature) = self.tablature.as_mut() {
                    tablature.update_track(selection.index);
                }
                self.track_selection = selection;
                Task::none()
            }
            Message::OpenFileDialog => {
                if self.tab_file_is_loading {
                    Task::none()
                } else {
                    self.tab_file_is_loading = true;
                    Task::perform(
                        open_file_dialog(self.config.get_tabs_folder()),
                        Message::FileOpened,
                    )
                }
            }
            Message::OpenFile(path) => {
                if self.tab_file_is_loading {
                    Task::none()
                } else {
                    self.tab_file_is_loading = true;
                    Task::perform(load_file(path), Message::FileOpened)
                }
            }
            Message::FileOpened(result) => {
                self.tab_file_is_loading = false;
                // stop previous audio player if any
                if let Some(audio_player) = &mut self.audio_player {
                    audio_player.stop();
                }
                match result {
                    Ok((contents, parent_folder, file_name)) => {
                        if let Err(err) = self.config.set_tabs_folder(parent_folder) {
                            return Task::done(Message::ReportError(format!(
                                "Failed to set tabs folder: {err}"
                            )));
                        }
                        if let Ok(song) = parse_gp_data(&contents) {
                            // build all tracks selection
                            let track_selections: Vec<_> = song
                                .tracks
                                .iter()
                                .enumerate()
                                .map(|(index, track)| {
                                    let tuning = song
                                        .midi_channels
                                        .iter()
                                        .find(|c| c.channel_id == track.channel_id)
                                        .filter(|c| !c.is_percussion())
                                        .and_then(|_| tuning_label(&track.strings));
                                    TrackSelection::new(index, track.name.clone(), tuning)
                                })
                                .collect();
                            if track_selections.is_empty() {
                                return Task::done(Message::ReportError(
                                    "No tracks found in GP file".to_string(),
                                ));
                            }
                            self.all_tracks.clone_from(&track_selections);
                            self.song_info = Some(SongDisplayInfo::new(&song, file_name));
                            // select first track by default
                            let default_track = 0;
                            let default_track_selection = track_selections[default_track].clone();
                            self.track_selection = default_track_selection;
                            // share song ownership with tablature and player
                            let song_rc = Rc::new(song);
                            let playback_order = compute_playback_order(&song_rc.measure_headers);
                            let tablature_scroll_id = Id::new("tablature-scroll-elements");
                            let tablature = Tablature::new(
                                song_rc.clone(),
                                default_track,
                                tablature_scroll_id.clone(),
                                &playback_order,
                            );
                            self.tablature = Some(tablature);
                            // audio player initialization
                            match AudioPlayer::new(
                                song_rc.clone(),
                                song_rc.tempo.value,
                                self.tempo_selection.percentage,
                                self.sound_font_file.clone(),
                                self.beat_sender.clone(),
                                &playback_order,
                            ) {
                                Ok(audio_player) => {
                                    self.audio_player = Some(audio_player);
                                    // reset tablature scroll and trigger layout computation
                                    Task::batch([
                                        scroll_to(
                                            tablature_scroll_id,
                                            AbsoluteOffset::<f32>::default(),
                                        ),
                                        Task::done(Message::WindowResized),
                                    ])
                                }
                                Err(err) => Task::done(Message::ReportError(format!(
                                    "Failed to initialize audio: {err}"
                                ))),
                            }
                        } else {
                            Task::done(Message::ReportError("Failed to parse GP file".to_string()))
                        }
                    }
                    Err(err) => {
                        Task::done(Message::ReportError(format!("Failed to open file: {err}")))
                    }
                }
            }
            Message::FocusMeasure(measure_id) => {
                // focus measure in tablature
                if let Some(tablature) = &mut self.tablature {
                    tablature.focus_on_measure(measure_id);
                }
                // focus measure in player
                if let Some(audio_player) = &self.audio_player {
                    audio_player.focus_measure(measure_id);
                }
                Task::none()
            }
            Message::FocusTick(tick) => {
                if let Some(tablature) = &mut self.tablature
                    && let Some(scroll_offset) = tablature.focus_on_tick(tick)
                {
                    // scroll to the focused measure
                    return scroll_to(
                        tablature.scroll_id.clone(),
                        AbsoluteOffset {
                            x: 0.0,
                            y: scroll_offset,
                        },
                    );
                }
                Task::none()
            }
            Message::NextMeasure => {
                let target = self.tablature.as_ref().and_then(|t| {
                    let next = t.focused_measure() + 1;
                    (next < t.measure_count()).then_some(next)
                });
                target.map_or_else(Task::none, |m| self.focus_measure_with_scroll(m))
            }
            Message::PreviousMeasure => {
                let target = self
                    .tablature
                    .as_ref()
                    .and_then(|t| t.focused_measure().checked_sub(1));
                target.map_or_else(Task::none, |m| self.focus_measure_with_scroll(m))
            }
            Message::PlayPause => {
                if self.tab_file_is_loading {
                    return Task::none();
                }
                if let Some(audio_player) = &mut self.audio_player
                    && let Some(err) = audio_player.toggle_play()
                {
                    return Task::done(Message::ReportError(err));
                }
                // Hack to make sure the tablature is aware of its size
                Task::done(Message::WindowResized)
            }
            Message::StopPlayer => {
                if let (Some(audio_player), Some(tablature)) =
                    (&mut self.audio_player, &mut self.tablature)
                {
                    // stop audio player
                    audio_player.stop();
                    // reset tablature focus
                    tablature.focus_on_measure(0);
                    // reset tablature scroll
                    scroll_to(
                        tablature.scroll_id.clone(),
                        AbsoluteOffset::<f32>::default(),
                    )
                } else {
                    Task::none()
                }
            }
            Message::ToggleSolo => {
                if let Some(audio_player) = &self.audio_player {
                    let track = self.track_selection.index;
                    audio_player.toggle_solo_mode(track);
                }
                Task::none()
            }
            Message::WindowResized => {
                // query tablature container size
                selector::find(self.tablature_id.clone()).map(|rect| {
                    Message::TablatureResized(rect.unwrap().visible_bounds().unwrap().size())
                })
            }
            Message::TablatureResized(tablature_container_size) => {
                if let Some(tablature) = &mut self.tablature {
                    tablature.update_container_width(tablature_container_size.width);
                }
                Task::none()
            }
            Message::TempoSelected(tempos_selection) => {
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_tempo_percentage(tempos_selection.percentage);
                }
                self.tempo_selection = tempos_selection;
                Task::none()
            }
            Message::IncreaseTempo => {
                if self.tab_file_is_loading {
                    return Task::none();
                }
                if let Some(current_index) = TempoSelection::PRESET
                    .iter()
                    .position(|t| t == &self.tempo_selection)
                    && current_index < TempoSelection::PRESET.len() - 1
                {
                    let next_tempo = TempoSelection::PRESET[current_index + 1];
                    return Task::done(Message::TempoSelected(next_tempo));
                }
                Task::none()
            }
            Message::DecreaseTempo => {
                if self.tab_file_is_loading {
                    return Task::none();
                }
                if let Some(current_index) = TempoSelection::PRESET
                    .iter()
                    .position(|t| t == &self.tempo_selection)
                    && current_index > 0
                {
                    let previous_tempo = TempoSelection::PRESET[current_index - 1];
                    return Task::done(Message::TempoSelected(previous_tempo));
                }
                Task::none()
            }
            Message::ToggleFullscreen => {
                self.is_fullscreen = !self.is_fullscreen;
                let mode = if self.is_fullscreen {
                    window::Mode::Fullscreen
                } else {
                    window::Mode::Windowed
                };
                window::latest().and_then(move |id| window::set_mode(id, mode))
            }
            Message::ClearError => {
                self.error_message = None;
                Task::none()
            }
            Message::ReportError(error) => {
                log::warn!("{error}");
                self.error_message = Some(error);
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let open_file = action_gated(
            open_icon(),
            "Open file",
            (!self.tab_file_is_loading).then_some(Message::OpenFileDialog),
        );

        let player_control = if let Some(audio_player) = &self.audio_player {
            let (icon, message) = if audio_player.is_playing() {
                (pause_icon(), "Pause")
            } else {
                (play_icon(), "Play")
            };
            let play_button = action_gated(icon, message, Some(Message::PlayPause));
            let stop_button = action_gated(stop_icon(), "Stop", Some(Message::StopPlayer));
            let counter = self
                .tablature
                .as_ref()
                .map(|tab| {
                    let headers = &tab.song.measure_headers;
                    let focused = tab.focused_measure();
                    let total_measures = tab.measure_count();
                    let current_seconds = song_time_up_to_measure(headers, focused);
                    let total_seconds = song_time_up_to_measure(headers, total_measures);
                    format!(
                        "Measure {}/{} \u{2022} {}/{}",
                        focused + 1,
                        total_measures,
                        format_mmss(current_seconds),
                        format_mmss(total_seconds),
                    )
                })
                .unwrap_or_default();
            row![play_button, stop_button, text(counter).size(14)]
                .spacing(10)
                .align_y(Alignment::Center)
        } else {
            row![horizontal()]
        };

        let track_control = if self.all_tracks.is_empty() {
            row![horizontal()]
        } else {
            let tempo_label = text("Tempo").size(14);
            let tempo_percentage = pick_list(
                TempoSelection::PRESET,
                Some(&self.tempo_selection),
                Message::TempoSelected,
            )
            .text_size(14)
            .padding([5, 10]);

            let solo_mode = action_toggle(
                solo_icon(),
                "Solo",
                Message::ToggleSolo,
                self.audio_player
                    .as_ref()
                    .is_some_and(|p| p.solo_track_id().is_some()),
            );

            let track_pick_list = pick_list(
                self.all_tracks.as_slice(),
                Some(&self.track_selection),
                Message::TrackSelected,
            )
            .text_size(14)
            .padding([5, 10]);

            row![tempo_label, tempo_percentage, solo_mode, track_pick_list,]
                .spacing(10)
                .align_y(Alignment::Center)
        };

        let controls = row![
            open_file,
            horizontal(),
            player_control,
            horizontal(),
            track_control,
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let controls = container(controls)
            .padding(10)
            .style(|_theme| container::Style {
                border: Border::default()
                    .color(crate::ui::utils::COLOR_GRAY)
                    .width(1),
                ..Default::default()
            });

        let song_info = if let Some(song) = &self.song_info {
            if !song.artist.is_empty() {
                format!("{} - {}", song.name, song.artist)
            } else {
                song.name.clone()
            }
        } else {
            String::new()
        };
        let metadata_line = self
            .song_info
            .as_ref()
            .and_then(SongDisplayInfo::metadata_line)
            .unwrap_or_default();

        let status = row![
            Text::new(song_info).shaping(Auto),
            horizontal(),
            Text::new(metadata_line).shaping(Auto),
            horizontal(),
            text(if let Some(song) = &self.song_info {
                format!("{:?}", song.gp_version)
            } else {
                String::new()
            }),
        ]
        .spacing(10);

        let status = container(status).padding(4);

        let tablature_view = self
            .tablature
            .as_ref()
            .map_or_else(|| untitled_text_table_box().into(), |t| t.view());

        let tablature = container(tablature_view).id(self.tablature_id.clone());

        let base: Element<Message> = if self.is_fullscreen {
            column![tablature].spacing(20).padding(10).into()
        } else {
            column![controls, tablature, rule::horizontal(1), status,]
                .spacing(20)
                .padding(10)
                .into()
        };

        // add error modal if any
        if let Some(error_message) = &self.error_message {
            let error_view = text(error_message).size(20);
            modal(base, error_view, Message::ClearError)
        } else {
            base
        }
    }

    #[allow(clippy::unused_self)]
    const fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn audio_player_beat_subscription(
        beat_receiver: Arc<Mutex<Receiver<u32>>>,
    ) -> impl Stream<Item = Message> {
        stream::channel(1, async move |mut output| {
            let mut receiver = beat_receiver.lock().await;
            loop {
                // get tick from audio player
                let tick = *receiver.borrow_and_update();
                // publish to UI
                output
                    .send(Message::FocusTick(tick))
                    .await
                    .expect("send failed");
                // wait for next beat
                receiver.changed().await.expect("receiver failed");
            }
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(2);

        // keyboard event subscription
        let keyboard_subscription = keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed {
                modified_key,
                modifiers,
                ..
            } = event
            else {
                return None;
            };
            match modified_key.as_ref() {
                keyboard::Key::Named(Space) => Some(Message::PlayPause),
                keyboard::Key::Named(ArrowUp) if modifiers.control() => {
                    Some(Message::IncreaseTempo)
                }
                keyboard::Key::Named(ArrowDown) if modifiers.control() => {
                    Some(Message::DecreaseTempo)
                }
                keyboard::Key::Named(ArrowLeft) => Some(Message::PreviousMeasure),
                keyboard::Key::Named(ArrowRight) => Some(Message::NextMeasure),
                keyboard::Key::Character(c) if c.eq_ignore_ascii_case("s") => {
                    Some(Message::ToggleSolo)
                }
                keyboard::Key::Named(F11) => Some(Message::ToggleFullscreen),
                _ => None,
            }
        });
        subscriptions.push(keyboard_subscription);

        // next beat notifier subscription
        let beat_receiver = self.beat_receiver.clone();
        subscriptions.push(Subscription::run_with(
            BeatSubscriptionData(beat_receiver.clone()),
            |data| Self::audio_player_beat_subscription(data.0.clone()),
        ));

        let window_resized = window::resize_events().map(|_| Message::WindowResized);
        subscriptions.push(window_resized);

        let file_dropped = window::events().filter_map(|(_, event)| {
            if let window::Event::FileDropped(path) = event {
                Some(Message::OpenFile(path))
            } else {
                None
            }
        });
        subscriptions.push(file_dropped);

        Subscription::batch(subscriptions)
    }
}

/// Seconds elapsed from the song's start up to (but not including) `measure_idx`.
/// Tempo changes across measures are honored. Repeats are ignored — we compute
/// the song's linear duration, not expanded playback time.
fn song_time_up_to_measure(headers: &[MeasureHeader], measure_idx: usize) -> f32 {
    headers
        .iter()
        .take(measure_idx)
        .enumerate()
        .map(|(i, h)| {
            let next_start = headers
                .get(i + 1)
                .map_or(h.start + h.length(), |next| next.start);
            let duration_ticks = next_start.saturating_sub(h.start) as f32;
            duration_ticks / QUARTER_TIME as f32 * 60.0 / h.tempo.value as f32
        })
        .sum()
}

fn format_mmss(seconds: f32) -> String {
    let total = seconds.max(0.0) as u32;
    format!("{}:{:02}", total / 60, total % 60)
}

struct BeatSubscriptionData(Arc<Mutex<Receiver<u32>>>);

impl std::hash::Hash for BeatSubscriptionData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "beat-subscription".hash(state); // The ID is constant
    }
}

impl PartialEq for BeatSubscriptionData {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for BeatSubscriptionData {}
