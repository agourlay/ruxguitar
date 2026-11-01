use iced::advanced::text::Shaping::Auto;
use iced::widget::operation::scroll_to;
use iced::widget::space::horizontal;
use iced::widget::{Id, Text, column, container, pick_list, row, rule, selector, slider, text};
use iced::{
    Alignment, Border, Element, Length, Point, Size, Subscription, Task, Theme, event, keyboard,
    mouse, stream, window,
};

/// Ignore tiny pointer jitter so a click does not start a measure selection.
const SELECTION_DRAG_THRESHOLD_PX: f32 = 12.0;
use std::fmt::Display;

use crate::audio::midi_player::AudioPlayer;
use crate::audio::playback_order::compute_playback_order;
use crate::config::{Config, SessionState};
use crate::parser::parse_gp_data;
use crate::parser::song_parser::{GpVersion, MeasureHeader, QUARTER_TIME, Song};
use crate::ui::icons::{
    count_in_icon, metronome_icon, mute_icon, open_icon, pause_icon, play_icon, solo_icon,
    stop_icon,
};
use crate::ui::picker::{FilePickerError, load_file, open_file_dialog};
use crate::ui::tablature::Tablature;
use crate::ui::tuning::tuning_label;
use crate::ui::utils::{action_gated, action_toggle, modal, untitled_text_table_box};
use crate::{ApplicationArgs, ThemeChoice};
use iced::futures::{SinkExt, Stream};
use iced::keyboard::key::Named::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, F11, Space};
use iced::widget::scrollable::AbsoluteOffset;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Notify;

const ICONS_FONT: &[u8] = include_bytes!("../../resources/icons.ttf");

pub struct RuxApplication {
    song_info: Option<SongDisplayInfo>,
    track_selection: TrackSelection,
    all_tracks: Vec<TrackSelection>,
    tablature: Option<Tablature>,
    tablature_id: Id,
    tempo_selection: TempoSelection,
    audio_player: Option<AudioPlayer>,
    /// Playback preferences kept at the app level so they survive reloading a file.
    metronome_enabled: bool,
    count_in_enabled: bool,
    tab_file_is_loading: bool,
    sound_font_file: Option<PathBuf>,
    /// Latest tick published by the audio callback, with its wake-up signal.
    current_tick: Arc<AtomicU32>,
    beat_notify: Arc<Notify>,
    config: Config,
    error_message: Option<String>,
    is_fullscreen: bool,
    measure_selection_dragging: bool,
    selection_drag_moved: bool,
    selection_press_position: Option<Point>,
    selection_focus_beat: usize,
    current_tab_path: Option<PathBuf>,
    pending_session_restore: Option<SessionState>,
    deferred_session_restore: Option<SessionState>,
    saved_master_volume: f32,
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

    const PRESET: [Self; 9] = [
        Self::new(25),
        Self::new(50),
        Self::new(60),
        Self::new(70),
        Self::new(80),
        Self::new(90),
        Self::new(100),
        Self::new(150),
        Self::new(200),
    ];
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
    OpenFileDialog,
    OpenFile(PathBuf),
    /// File content, parent folder and file name.
    FileOpened(Result<(Vec<u8>, Option<PathBuf>, String, PathBuf), FilePickerError>),
    TrackSelected(TrackSelection),
    SelectionDragStart(usize, usize, Point),
    SelectionDragTo(usize, Point),
    SelectionDragEnd,
    FocusTick(u32),
    NextMeasure,
    PreviousMeasure,
    PlayPause,
    StopPlayer,
    ToggleSolo,
    /// Toggle mute of the currently selected track.
    ToggleMute,
    ToggleMetronome,
    ToggleCountIn,
    WindowResized,
    TablatureResized(Size),
    TempoSelected(TempoSelection),
    IncreaseTempo,
    DecreaseTempo,
    ClearError,
    ReportError(String),
    /// Toggle fullscreen, which also hides the controls and status bar.
    ToggleFullscreen,
    /// Master volume slider, in `0.0..=1.0`.
    MasterVolumeChanged(f32),
    /// The window is closing: save the session, then exit.
    CloseRequested,
}

impl RuxApplication {
    fn new(sound_font_file: Option<PathBuf>, config: Config) -> Self {
        let session = config.session().clone();
        Self {
            song_info: None,
            track_selection: TrackSelection::default(),
            all_tracks: vec![],
            tablature: None,
            tablature_id: Id::new("tablature-outer-container"),
            tempo_selection: TempoSelection::new(session.tempo_percentage),
            audio_player: None,
            metronome_enabled: false,
            count_in_enabled: false,
            tab_file_is_loading: false,
            sound_font_file,
            current_tick: Arc::new(AtomicU32::new(0)),
            beat_notify: Arc::new(Notify::new()),
            config,
            error_message: None,
            is_fullscreen: false,
            measure_selection_dragging: false,
            selection_drag_moved: false,
            selection_press_position: None,
            selection_focus_beat: 0,
            current_tab_path: None,
            pending_session_restore: None,
            deferred_session_restore: None,
            saved_master_volume: session.master_volume.clamp(0.0, 1.0),
        }
    }

    fn boot(args: &ApplicationArgs) -> (Self, Task<Message>) {
        let mut app = Self::new(args.sound_font_bank.clone(), args.local_config.clone());

        let init_task = if let Some(path) = &args.tab_file_path {
            Task::done(Message::OpenFile(path.clone()))
        } else if let Some(path) = app.config.session().last_tab_file.clone() {
            if path.is_file() {
                app.pending_session_restore = Some(app.config.session().clone());
                Task::done(Message::OpenFile(path))
            } else {
                Task::none()
            }
        } else {
            Task::none()
        };
        (app, init_task)
    }

    /// Fresh tab defaults when the user opens a GP file (not startup session restore).
    fn reset_tab_defaults(&mut self) {
        self.tempo_selection = TempoSelection::default();
        self.saved_master_volume = 1.0;
        self.deferred_session_restore = None;
    }

    /// Loop the selected measures, or play through when nothing is selected.
    /// Returns the measure each pass of the loop starts on.
    fn sync_measure_loop(&self) -> Option<usize> {
        let (Some(audio_player), Some(tablature)) = (&self.audio_player, &self.tablature) else {
            return None;
        };
        if let Some((start, end)) = tablature.selection_range() {
            audio_player.set_measure_loop(start, end)
        } else {
            audio_player.clear_measure_loop();
            None
        }
    }

    fn persist_session(&mut self) {
        let (measure_index, beat_index, selection_start, selection_end) =
            if let Some(tablature) = &self.tablature {
                let selection = tablature.selection_range();
                (
                    tablature.focused_measure(),
                    tablature.focused_beat(),
                    selection.map(|(start, _)| start),
                    selection.map(|(_, end)| end),
                )
            } else {
                (
                    self.config.session().measure_index,
                    self.config.session().beat_index,
                    self.config.session().selection_start,
                    self.config.session().selection_end,
                )
            };

        let master_volume = self
            .audio_player
            .as_ref()
            .map(AudioPlayer::master_volume)
            .unwrap_or(self.saved_master_volume);

        let session = SessionState {
            last_tab_file: self.current_tab_path.clone(),
            track_index: self.track_selection.index,
            tempo_percentage: self.tempo_selection.percentage,
            master_volume,
            measure_index,
            beat_index,
            selection_start,
            selection_end,
        };
        self.saved_master_volume = master_volume;
        log::debug!(
            "Session saved (tab={:?}, track={}, measure={})",
            session.last_tab_file,
            session.track_index,
            session.measure_index
        );
        if let Err(err) = self.config.set_session(session) {
            log::warn!("Failed to save session: {err}");
        }
    }

    fn apply_session_restore(&mut self, session: &SessionState) -> Task<Message> {
        let Some(tablature) = &mut self.tablature else {
            return Task::none();
        };
        let Some(audio_player) = &self.audio_player else {
            return Task::none();
        };

        let track_count = self.all_tracks.len();
        if track_count == 0 {
            return Task::none();
        }
        let track = session.track_index.min(track_count - 1);
        if track != self.track_selection.index {
            self.track_selection = self.all_tracks[track].clone();
            tablature.update_track(track);
        }

        let tempo = session.tempo_percentage.clamp(25, 200);
        self.tempo_selection = TempoSelection::new(tempo);
        audio_player.set_tempo_percentage(tempo);

        let volume = session.master_volume.clamp(0.0, 1.0);
        self.saved_master_volume = volume;
        audio_player.set_master_volume(volume);

        let measure_count = tablature.measure_count();
        if measure_count == 0 {
            return Task::none();
        }
        let measure = session.measure_index.min(measure_count - 1);
        let beat = session.beat_index;
        tablature.focus_on_measure_beat(measure, beat);
        let beat_offset = tablature.beat_tick_offset(measure, tablature.focused_beat());
        audio_player.focus_measure_at(measure, beat_offset);

        match (session.selection_start, session.selection_end) {
            (Some(start), Some(end)) => {
                let lo = start.min(end).min(measure_count - 1);
                let hi = start.max(end).min(measure_count - 1);
                tablature.restore_selection(lo, hi);
            }
            _ => tablature.clear_selection(),
        }

        let scroll_id = tablature.scroll_id.clone();
        let scroll_task = tablature
            .page_scroll_offset(measure)
            .map_or_else(Task::none, |y| {
                scroll_to(scroll_id, AbsoluteOffset { x: 0.0, y })
            });
        self.persist_session();
        scroll_task
    }

    pub fn start(args: ApplicationArgs) -> iced::Result {
        let antialiasing = !args.no_antialiasing;
        let theme = resolve_theme(args.theme);
        iced::application(move || Self::boot(&args), Self::update, Self::view)
            .title(Self::title)
            .subscription(Self::subscription)
            .default_font(iced::Font::MONOSPACE)
            .theme(move |_state: &Self| theme.clone())
            .font(ICONS_FONT)
            .window_size((1150.0, 768.0))
            .centered()
            .exit_on_close_request(false)
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
        let scroll_offset = tablature.page_scroll_offset(measure_id);
        let scroll_id = tablature.scroll_id.clone();
        if let Some(audio_player) = &self.audio_player {
            audio_player.focus_measure_at(measure_id, 0);
        }
        let task = scroll_offset.map_or_else(Task::none, |y| {
            scroll_to(scroll_id, AbsoluteOffset { x: 0.0, y })
        });
        self.persist_session();
        task
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TrackSelected(selection) => {
                if let Some(tablature) = self.tablature.as_mut() {
                    tablature.update_track(selection.index);
                }
                self.track_selection = selection;
                self.persist_session();
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
                // stop and drop the previous audio player if any: if loading
                // fails below, the old song must not keep playing against the
                // new tablature state
                if let Some(audio_player) = &mut self.audio_player {
                    audio_player.stop();
                }
                self.audio_player = None;
                // a startup restore only applies to the file it reopened,
                // whether or not that load succeeds
                let restore_session = self.pending_session_restore.take();
                match result {
                    Ok((contents, parent_folder, file_name, tab_path)) => {
                        if let Err(err) = self.config.set_tabs_folder(parent_folder) {
                            return Task::done(Message::ReportError(format!(
                                "Failed to set tabs folder: {err}"
                            )));
                        }
                        if let Ok(song) = parse_gp_data(&contents) {
                            if restore_session.is_none() {
                                self.reset_tab_defaults();
                            } else {
                                self.deferred_session_restore = restore_session;
                            }
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
                            self.track_selection = track_selections[default_track].clone();
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
                                self.current_tick.clone(),
                                self.beat_notify.clone(),
                                &playback_order,
                            ) {
                                Ok(audio_player) => {
                                    // apply persistent playback preferences
                                    audio_player.set_metronome(self.metronome_enabled);
                                    audio_player.set_count_in(self.count_in_enabled);
                                    audio_player.set_master_volume(self.saved_master_volume);
                                    self.audio_player = Some(audio_player);
                                    self.current_tab_path = Some(tab_path);
                                    self.persist_session();
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
            Message::SelectionDragStart(measure_id, beat_id, press_position) => {
                self.measure_selection_dragging = true;
                self.selection_drag_moved = false;
                self.selection_press_position = Some(press_position);
                self.selection_focus_beat = beat_id;
                if let Some(tablature) = &mut self.tablature {
                    tablature.set_pointer_drag_active(true);
                    tablature.begin_selection_drag(measure_id);
                }
                Task::none()
            }
            Message::SelectionDragTo(measure_id, cursor_position) => {
                if !self.measure_selection_dragging {
                    return Task::none();
                }
                let Some(press_position) = self.selection_press_position else {
                    return Task::none();
                };
                let delta = cursor_position - press_position;
                if delta.x * delta.x + delta.y * delta.y
                    < SELECTION_DRAG_THRESHOLD_PX * SELECTION_DRAG_THRESHOLD_PX
                {
                    return Task::none();
                }
                self.selection_drag_moved = true;
                if let Some(tablature) = &mut self.tablature {
                    tablature.extend_selection_drag(measure_id);
                }
                Task::none()
            }
            Message::SelectionDragEnd => {
                if !self.measure_selection_dragging {
                    return Task::none();
                }
                self.measure_selection_dragging = false;
                let dragged = self.selection_drag_moved;
                self.selection_drag_moved = false;
                self.selection_press_position = None;
                if let Some(tablature) = &mut self.tablature {
                    tablature.set_pointer_drag_active(false);
                    if !dragged {
                        tablature.clear_selection();
                    }
                }
                // keep a running loop in step with the selection
                let loop_start = self.sync_measure_loop();
                let focus_task = if let Some(tablature) = &mut self.tablature {
                    // a new loop starts playing from its first measure
                    let (measure_id, beat_id) = match (loop_start, tablature.selection_range()) {
                        (Some(first_measure), _) => (first_measure, 0),
                        (None, Some((start, _))) => (start, 0),
                        (None, None) => (tablature.selection_anchor(), self.selection_focus_beat),
                    };
                    tablature.focus_on_measure_beat(measure_id, beat_id);
                    if let Some(audio_player) = &self.audio_player {
                        let beat_offset = tablature.beat_tick_offset(measure_id, beat_id);
                        audio_player.focus_measure_at(measure_id, beat_offset);
                    }
                    tablature
                        .page_scroll_offset(measure_id)
                        .map(|offset| {
                            scroll_to(
                                tablature.scroll_id.clone(),
                                AbsoluteOffset { x: 0.0, y: offset },
                            )
                        })
                        .unwrap_or_else(Task::none)
                } else {
                    Task::none()
                };
                self.persist_session();
                focus_task
            }
            Message::FocusTick(tick) => {
                // the song ran out: stop the transport rather than leave it
                // playing silence
                if self
                    .audio_player
                    .as_ref()
                    .is_some_and(AudioPlayer::is_finished)
                {
                    return Task::done(Message::StopPlayer);
                }
                let Some(tablature) = &mut self.tablature else {
                    return Task::none();
                };
                tablature.focus_on_tick(tick);
                // hold the page still while it is being read, turn it when
                // playback moves past it
                let focused = tablature.focused_measure();
                tablature
                    .page_scroll_offset(focused)
                    .map_or_else(Task::none, |offset| {
                        scroll_to(
                            tablature.scroll_id.clone(),
                            AbsoluteOffset { x: 0.0, y: offset },
                        )
                    })
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
                let was_playing = self
                    .audio_player
                    .as_ref()
                    .is_some_and(AudioPlayer::is_playing);
                let mut scroll_task = Task::none();
                if !was_playing {
                    let loop_start = self.sync_measure_loop();
                    // resume in place inside the loop, otherwise jump to its start
                    let outside_loop = self.tablature.as_ref().and_then(|tablature| {
                        let (_, end) = tablature.selection_range()?;
                        let start = loop_start?;
                        let focused = tablature.focused_measure();
                        (focused < start || focused > end).then_some(start)
                    });
                    if let Some(start) = outside_loop {
                        scroll_task = self.focus_measure_with_scroll(start);
                    }
                }
                if let Some(audio_player) = &mut self.audio_player {
                    if let Some(err) = audio_player.toggle_play() {
                        return Task::done(Message::ReportError(err));
                    }
                    if was_playing {
                        self.persist_session();
                    }
                }
                // Hack to make sure the tablature is aware of its size
                Task::batch([scroll_task, Task::done(Message::WindowResized)])
            }
            Message::StopPlayer => {
                if self.audio_player.is_some() && self.tablature.is_some() {
                    self.persist_session();
                }
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
            Message::ToggleMute => {
                if let Some(audio_player) = &self.audio_player {
                    let track = self.track_selection.index;
                    audio_player.toggle_track_mute(track);
                }
                Task::none()
            }
            Message::ToggleMetronome => {
                self.metronome_enabled = !self.metronome_enabled;
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_metronome(self.metronome_enabled);
                }
                Task::none()
            }
            Message::ToggleCountIn => {
                self.count_in_enabled = !self.count_in_enabled;
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_count_in(self.count_in_enabled);
                }
                Task::none()
            }
            Message::WindowResized => {
                // query tablature container size
                selector::find(self.tablature_id.clone()).then(|target| {
                    // the container can be fully clipped out of the viewport
                    // (e.g. window resized very short): skip the layout update
                    target
                        .and_then(|t| t.visible_bounds())
                        .map_or_else(Task::none, |bounds| {
                            Task::done(Message::TablatureResized(bounds.size()))
                        })
                })
            }
            Message::TablatureResized(tablature_container_size) => {
                if let Some(tablature) = &mut self.tablature {
                    tablature.update_container_size(
                        tablature_container_size.width,
                        tablature_container_size.height,
                    );
                }
                if tablature_container_size.width > 0.0
                    && let Some(session) = self.deferred_session_restore.take()
                {
                    return self.apply_session_restore(&session);
                }
                Task::none()
            }
            Message::TempoSelected(tempos_selection) => {
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_tempo_percentage(tempos_selection.percentage);
                }
                self.tempo_selection = tempos_selection;
                self.persist_session();
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
            Message::MasterVolumeChanged(volume) => {
                self.saved_master_volume = volume;
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_master_volume(volume);
                }
                self.persist_session();
                Task::none()
            }
            Message::CloseRequested => {
                self.persist_session();
                iced::exit()
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

            let mute_mode = action_toggle(
                mute_icon(),
                "Mute",
                Message::ToggleMute,
                self.audio_player
                    .as_ref()
                    .is_some_and(|p| p.is_track_muted(self.track_selection.index)),
            );

            let metronome_mode = action_toggle(
                metronome_icon(),
                "Metronome",
                Message::ToggleMetronome,
                self.metronome_enabled,
            );

            let count_in_mode = action_toggle(
                count_in_icon(),
                "Count-in",
                Message::ToggleCountIn,
                self.count_in_enabled,
            );

            let track_pick_list = pick_list(
                self.all_tracks.as_slice(),
                Some(&self.track_selection),
                Message::TrackSelected,
            )
            .text_size(14)
            .padding([5, 10]);

            let volume_label = text("Volume").size(14);
            let current_volume = self
                .audio_player
                .as_ref()
                .map_or(self.saved_master_volume, AudioPlayer::master_volume);
            let volume_slider = slider(0.0..=1.0, current_volume, Message::MasterVolumeChanged)
                .step(0.01_f32)
                .width(100);

            row![
                tempo_label,
                tempo_percentage,
                volume_label,
                volume_slider,
                metronome_mode,
                count_in_mode,
                solo_mode,
                mute_mode,
                track_pick_list,
            ]
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
            .style(|theme: &Theme| container::Style {
                // a muted border that reads on either theme tone
                border: Border::default()
                    .color(crate::ui::utils::TablatureColors::of(theme).string_line)
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

        let gp_version = if let Some(song) = &self.song_info {
            format!("{:?}", song.gp_version)
        } else {
            String::new()
        };
        let status = row![
            container(Text::new(song_info).shaping(Auto))
                .width(Length::FillPortion(1))
                .align_x(Alignment::Start),
            container(Text::new(metadata_line).shaping(Auto))
                .width(Length::FillPortion(1))
                .align_x(Alignment::Center),
            container(text(gp_version))
                .width(Length::FillPortion(1))
                .align_x(Alignment::End),
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
    fn audio_player_beat_subscription(
        current_tick: Arc<AtomicU32>,
        beat_notify: Arc<Notify>,
    ) -> impl Stream<Item = Message> {
        stream::channel(1, async move |mut output| {
            loop {
                beat_notify.notified().await;
                let tick = current_tick.load(Ordering::Acquire);
                output
                    .send(Message::FocusTick(tick))
                    .await
                    .expect("send failed");
            }
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(4);

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
                keyboard::Key::Character(c) if c.eq_ignore_ascii_case("m") => {
                    Some(Message::ToggleMute)
                }
                keyboard::Key::Named(F11) => Some(Message::ToggleFullscreen),
                _ => None,
            }
        });
        subscriptions.push(keyboard_subscription);

        // next beat notifier subscription
        subscriptions.push(Subscription::run_with(
            BeatSubscriptionData(self.current_tick.clone(), self.beat_notify.clone()),
            |data| Self::audio_player_beat_subscription(data.0.clone(), data.1.clone()),
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

        // closing is handled here so the session is saved before exiting
        subscriptions.push(window::close_requests().map(|_| Message::CloseRequested));

        if self.measure_selection_dragging {
            let drag_release = event::listen_with(|event, _status, _id| {
                if matches!(
                    event,
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                ) {
                    Some(Message::SelectionDragEnd)
                } else {
                    None
                }
            });
            subscriptions.push(drag_release);
        }

        Subscription::batch(subscriptions)
    }
}

/// The theme to run with, or `None` to let iced follow the desktop.
///
/// An explicit choice always wins. Otherwise iced resolves the desktop
/// preference - except on Linux, where winit reports none and iced would
/// fall back to light whatever the desktop looks like; the dark tab stays
/// the default there.
fn resolve_theme(choice: Option<ThemeChoice>) -> Option<Theme> {
    choice
        .map(Theme::from)
        .or_else(|| cfg!(target_os = "linux").then_some(Theme::Dark))
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

struct BeatSubscriptionData(Arc<AtomicU32>, Arc<Notify>);

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
