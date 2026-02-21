use iced::advanced::text::Shaping::Advanced;
use iced::widget::{Text, column, container, horizontal_space, pick_list, row, text};
use iced::{
    Alignment, Border, Color, Element, Size, Subscription, Task, Theme, keyboard, stream, window,
};
use std::borrow::Cow;
use std::fmt::Display;

use crate::ApplicationArgs;
use crate::audio::midi_player::AudioPlayer;
use crate::config::Config;
use crate::parser::song_parser::{GpVersion, Song, parse_gp_data};
use crate::ui::icons::{open_icon, pause_icon, play_icon, solo_icon, stop_icon};
use crate::ui::picker::{FilePickerError, load_file, open_file_dialog};
use crate::ui::tablature::Tablature;
use crate::ui::utils::{action_gated, action_toggle, modal, untitled_text_table_box};
use iced::futures::{SinkExt, Stream};
use iced::keyboard::key::Named::{ArrowDown, ArrowUp, Space};
use iced::widget::container::visible_bounds;
use iced::widget::scrollable::{AbsoluteOffset, Id, scroll_to};
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
    tablature_id: container::Id,              // tablature container id
    tempo_selection: TempoSelection,          // tempo percentage for playback
    audio_player: Option<AudioPlayer>,        // audio player
    tab_file_is_loading: bool,                // file loading flag in progress
    sound_font_file: Option<PathBuf>,         // sound font file
    beat_sender: Arc<Sender<u32>>,            // beat notifier
    beat_receiver: Arc<Mutex<Receiver<u32>>>, // beat receiver
    config: Config,                           // local configuration
    error_message: Option<String>,            // error message to display
}

#[derive(Debug)]
struct SongDisplayInfo {
    name: String,
    artist: String,
    gp_version: GpVersion,
    file_name: String,
}

impl SongDisplayInfo {
    fn new(song: &Song, file_name: String) -> Self {
        Self {
            name: song.song_info.name.clone(),
            artist: song.song_info.artist.clone(),
            gp_version: song.version,
            file_name,
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
}

impl TrackSelection {
    const fn new(index: usize, name: String) -> Self {
        Self { index, name }
    }
}

impl Display for TrackSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} - {}", self.index + 1, self.name)
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
}

impl RuxApplication {
    fn new(sound_font_file: Option<PathBuf>, config: Config) -> Self {
        let (beat_sender, beat_receiver) = tokio::sync::watch::channel(0_u32);
        Self {
            song_info: None,
            track_selection: TrackSelection::default(),
            all_tracks: vec![],
            tablature: None,
            tablature_id: container::Id::new("tablature-outer-container"),
            tempo_selection: TempoSelection::default(),
            audio_player: None,
            tab_file_is_loading: false,
            sound_font_file,
            beat_receiver: Arc::new(Mutex::new(beat_receiver)),
            beat_sender: Arc::new(beat_sender),
            config,
            error_message: None,
        }
    }

    pub fn start(args: ApplicationArgs) -> iced::Result {
        iced::application(Self::title, Self::update, Self::view)
            .subscription(Self::subscription)
            .default_font(iced::Font::MONOSPACE)
            .theme(Self::theme)
            .font(ICONS_FONT)
            .window_size((1150.0, 768.0))
            .centered()
            .antialiasing(!args.no_antialiasing)
            .run_with(move || {
                (
                    Self::new(args.sound_font_bank.clone(), args.local_config.clone()),
                    args.tab_file_path
                        .map_or_else(Task::none, |f| Task::done(Message::OpenFile(f))),
                )
            })
    }

    fn title(&self) -> String {
        match &self.song_info {
            Some(song_info) => format!("Ruxguitar - {}", song_info.file_name),
            None => String::from("Ruxguitar - untitled"),
        }
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
                self.tab_file_is_loading = true;
                Task::perform(load_file(path), Message::FileOpened)
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
                                    TrackSelection::new(index, track.name.clone())
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
                            let tablature_scroll_id =
                                Id::new(Cow::Borrowed("tablature-scroll-elements"));
                            let tablature = Tablature::new(
                                song_rc.clone(),
                                default_track,
                                tablature_scroll_id.clone(),
                            );
                            self.tablature = Some(tablature);
                            // audio player initialization
                            let audio_player = AudioPlayer::new(
                                song_rc.clone(),
                                song_rc.tempo.value,
                                self.tempo_selection.percentage,
                                self.sound_font_file.clone(),
                                self.beat_sender.clone(),
                            );
                            self.audio_player = Some(audio_player);
                            // reset tablature scroll
                            scroll_to(tablature_scroll_id, AbsoluteOffset::default())
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
            Message::PlayPause => {
                if let Some(audio_player) = &mut self.audio_player {
                    audio_player.toggle_play();
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
                    scroll_to(tablature.scroll_id.clone(), AbsoluteOffset::default())
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
                visible_bounds(self.tablature_id.clone())
                    .map(|rect| Message::TablatureResized(rect.unwrap().size()))
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
            row![play_button, stop_button,]
                .spacing(10)
                .align_y(Alignment::Center)
        } else {
            row![horizontal_space()]
        };

        let track_control = if self.all_tracks.is_empty() {
            row![horizontal_space()]
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
            horizontal_space(),
            player_control,
            horizontal_space(),
            track_control,
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let controls = container(controls)
            .padding(10)
            .style(|_theme| container::Style {
                border: Border::default()
                    .color(Color::from_rgb8(0x40, 0x44, 0x4B)) // gray
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
        let status = row![
            Text::new(song_info).shaping(Advanced),
            horizontal_space(),
            text(if let Some(song) = &self.song_info {
                format!("{:?}", song.gp_version)
            } else {
                String::new()
            }),
        ]
        .spacing(10);

        let tablature_view = self
            .tablature
            .as_ref()
            .map_or_else(|| untitled_text_table_box().into(), |t| t.view());

        let tablature = container(tablature_view).id(self.tablature_id.clone());

        let base = column![controls, tablature, status,]
            .spacing(20)
            .padding(10)
            .into();

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
        stream::channel(1, move |mut output| async move {
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
        let keyboard_subscription = keyboard::on_key_press(|key, modifiers| match key.as_ref() {
            keyboard::Key::Named(Space) => Some(Message::PlayPause),
            keyboard::Key::Named(ArrowUp) if modifiers.control() => Some(Message::IncreaseTempo),
            keyboard::Key::Named(ArrowDown) if modifiers.control() => Some(Message::DecreaseTempo),
            _ => None,
        });
        subscriptions.push(keyboard_subscription);

        // next beat notifier subscription
        let audio_player_beat_subscription =
            Self::audio_player_beat_subscription(self.beat_receiver.clone());
        subscriptions.push(Subscription::run_with_id(
            "audio-player-beat",
            audio_player_beat_subscription,
        ));

        let window_resized = window::resize_events().map(|_| Message::WindowResized);
        subscriptions.push(window_resized);

        Subscription::batch(subscriptions)
    }
}
