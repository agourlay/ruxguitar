use iced::widget::operation::scroll_to;
use iced::widget::{Id, selector};
use iced::{Point, Size, Task, Theme, theme, window};

use crate::audio::midi_player::AudioPlayer;
use crate::audio::playback_order::compute_playback_order;
use crate::config::{Config, SessionState};
use crate::parser::parse_gp_data;
use crate::ui::picker::{FilePickerError, load_file, open_file_dialog};
use crate::ui::selection_drag::{DragRelease, SelectionDrag};
use crate::ui::selections::{TempoSelection, TrackSelection};
use crate::ui::song_info::SongDisplayInfo;
use crate::ui::tablature::Tablature;
use crate::ui::track_overview::{PLAYING_MARGIN, TrackOverview};
use crate::ui::utils::rux_theme;
use crate::{ApplicationArgs, ThemeChoice};
use iced::widget::scrollable::{AbsoluteOffset, Viewport};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;
use tokio::sync::Notify;

const ICONS_FONT: &[u8] = include_bytes!("../../../resources/icons.ttf");
const WINDOW_SIZE: Size = Size::new(1150.0, 768.0);
/// How long a notice stays over the fullscreen tablature.
const NOTICE_DURATION: Duration = Duration::from_millis(1500);

pub struct RuxApplication {
    /// The open song, if any.
    pub(super) song: Option<LoadedSong>,
    pub(super) tablature_id: Id,
    /// Absent while no song is open, or when the audio failed to start.
    pub(super) audio_player: Option<AudioPlayer>,
    pub(super) prefs: PlaybackPrefs,
    pub(super) tab_file_is_loading: bool,
    sound_font_file: Option<PathBuf>,
    /// Latest tick published by the audio callback, with its wake-up signal.
    pub(super) current_tick: Arc<AtomicU32>,
    pub(super) beat_notify: Arc<Notify>,
    config: Config,
    pub(super) error_message: Option<String>,
    pub(super) is_fullscreen: bool,
    /// What a key just changed, shown for a moment over the fullscreen
    /// tablature, which hides the overview.
    pub(super) notice: Option<String>,
    /// Counts the notices, so an expiry only clears its own.
    notice_count: u64,
    /// The theme asked for on the command line, over the desktop's tone.
    theme_choice: Option<ThemeChoice>,
    /// Built once per tone: iced asks for it on every refresh.
    theme: Theme,
    window_height: f32,
    pub(super) selection_drag: SelectionDrag,
    current_tab_path: Option<PathBuf>,
    session_restore: Option<SessionRestore>,
    /// The track asked for on the command line (numbered from 1), for the
    /// tab file opened at startup only.
    startup_track: Option<usize>,
}

/// A parsed song, with its tablature and the track it shows.
pub(super) struct LoadedSong {
    pub(super) info: SongDisplayInfo,
    pub(super) tracks: Vec<TrackSelection>,
    pub(super) track: TrackSelection,
    pub(super) tablature: Tablature,
    /// Every track's measures at a glance, under the tablature.
    pub(super) overview: TrackOverview,
}

/// Playback preferences, kept at the app level so they survive reloading a file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PlaybackPrefs {
    pub(super) tempo: TempoSelection,
    pub(super) metronome: bool,
    pub(super) count_in: bool,
    /// The volume to give a new player; the live one is the player's own.
    pub(super) master_volume: f32,
}

impl PlaybackPrefs {
    fn apply_to(&self, audio_player: &AudioPlayer) {
        audio_player.set_metronome(self.metronome);
        audio_player.set_count_in(self.count_in);
        audio_player.set_master_volume(self.master_volume);
    }
}

/// The track a song opens on, as an index: the one asked for on the command
/// line (numbered from 1) when the song has it, else the first, with the
/// error to show.
fn opening_track(requested: Option<usize>, track_count: usize) -> (usize, Option<String>) {
    match requested {
        Some(number) if (1..=track_count).contains(&number) => (number - 1, None),
        Some(number) => (
            0,
            Some(format!(
                "Track {number} not found: the song has {track_count} track{}",
                if track_count == 1 { "" } else { "s" }
            )),
        ),
        None => (0, None),
    }
}

/// A saved session waiting to be applied to the file reopened at startup.
#[derive(Debug)]
enum SessionRestore {
    /// The file is loading; the session only applies if it opens.
    OnLoad(SessionState),
    /// The file is open; the session waits for the tablature to be laid out.
    OnLayout(SessionState),
}

impl SessionRestore {
    /// Take the session waiting for its file to load, leaving any other.
    fn take_on_load(slot: &mut Option<Self>) -> Option<SessionState> {
        match slot.take_if(|restore| matches!(restore, Self::OnLoad(_))) {
            Some(Self::OnLoad(session)) => Some(session),
            _ => None,
        }
    }

    /// Take the session waiting for the layout, leaving any other: a
    /// layout pass that runs while the file still loads must not drop it.
    fn take_on_layout(slot: &mut Option<Self>) -> Option<SessionState> {
        match slot.take_if(|restore| matches!(restore, Self::OnLayout(_))) {
            Some(Self::OnLayout(session)) => Some(session),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenFileDialog,
    OpenFile(PathBuf),
    /// File content, parent folder and file name.
    FileOpened(Result<(Vec<u8>, Option<PathBuf>, String, PathBuf), FilePickerError>),
    /// A track was picked, by its label in the overview or its number key.
    TrackOverviewTrack(usize),
    /// A square was pressed in the overview: its track and measure.
    TrackOverviewMeasure(usize, usize),
    TrackOverviewScrolled(Viewport),
    TrackOverviewLabelsScrolled(Viewport),
    SelectionDragStart(usize, usize, Point),
    SelectionDragTo(usize, Point),
    SelectionDragEnd,
    FocusTick(u32),
    NextMeasure,
    PreviousMeasure,
    NextTrack,
    PreviousTrack,
    PlayPause,
    StopPlayer,
    /// Toggle solo of the currently selected track.
    ToggleSolo,
    /// Toggle mute of the currently selected track.
    ToggleMute,
    /// Toggle solo for a specific track (from the overview).
    ToggleTrackSolo(usize),
    /// Toggle mute for a specific track (from the overview).
    ToggleTrackMute(usize),
    ToggleMetronome,
    ToggleCountIn,
    /// The window took a new size.
    WindowSized(Size),
    /// The layout changed: measure the tablature again.
    WindowResized,
    TablatureResized(Size),
    /// The desktop switched between its light and dark tones.
    DesktopToneChanged(theme::Mode),
    /// The notice with this count has been shown long enough.
    NoticeExpired(u64),
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
            song: None,
            tablature_id: Id::new("tablature-outer-container"),
            audio_player: None,
            prefs: PlaybackPrefs {
                tempo: TempoSelection::new(session.tempo_percentage),
                metronome: false,
                count_in: false,
                master_volume: session.master_volume.clamp(0.0, 1.0),
            },
            tab_file_is_loading: false,
            sound_font_file,
            current_tick: Arc::new(AtomicU32::new(0)),
            beat_notify: Arc::new(Notify::new()),
            config,
            error_message: None,
            is_fullscreen: false,
            notice: None,
            notice_count: 0,
            theme_choice: None,
            theme: rux_theme(is_dark(None, theme::Mode::None)),
            window_height: WINDOW_SIZE.height,
            selection_drag: SelectionDrag::Idle,
            current_tab_path: None,
            session_restore: None,
            startup_track: None,
        }
    }

    fn boot(args: &ApplicationArgs) -> (Self, Task<Message>) {
        let mut app = Self::new(args.sound_font_bank.clone(), args.local_config.clone());
        app.theme_choice = args.theme;
        app.theme = rux_theme(is_dark(app.theme_choice, theme::Mode::None));

        let init_task = if let Some(path) = &args.tab_file_path {
            app.startup_track = args.track;
            Task::done(Message::OpenFile(path.clone()))
        } else if let Some(path) = app.config.session().last_tab_file.clone() {
            if path.is_file() {
                app.session_restore = Some(SessionRestore::OnLoad(app.config.session().clone()));
                Task::done(Message::OpenFile(path))
            } else {
                Task::none()
            }
        } else {
            Task::none()
        };
        let desktop_tone = iced::system::theme().map(Message::DesktopToneChanged);
        (app, Task::batch([init_task, desktop_tone]))
    }

    /// The command line's theme, or the desktop's tone.
    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    /// Show `text` for a moment over the fullscreen tablature.
    fn notify(&mut self, text: String) -> Task<Message> {
        if !self.is_fullscreen {
            return Task::none();
        }
        self.notice = Some(text);
        self.notice_count += 1;
        let count = self.notice_count;
        // built when polled: the timer needs the runtime
        Task::perform(
            async { tokio::time::sleep(NOTICE_DURATION).await },
            move |()| Message::NoticeExpired(count),
        )
    }

    /// Tell which track shows and how it sounds, when the overview that
    /// shows it is hidden.
    fn notify_track(&mut self) -> Task<Message> {
        let Some(song) = &self.song else {
            return Task::none();
        };
        let mut text = song.track.to_string();
        if let Some(player) = &self.audio_player {
            if player.is_track_solo(song.track.index) {
                text.push_str(" \u{2022} solo");
            }
            if player.is_track_muted(song.track.index) {
                text.push_str(" \u{2022} muted");
            }
        }
        self.notify(text)
    }

    /// Loop the selected measures, or play through when nothing is selected.
    /// Returns the measure each pass of the loop starts on.
    fn sync_measure_loop(&self) -> Option<usize> {
        let (Some(audio_player), Some(song)) = (&self.audio_player, &self.song) else {
            return None;
        };
        if let Some((start, end)) = song.tablature.selection_range() {
            audio_player.set_measure_loop(start, end)
        } else {
            audio_player.clear_measure_loop();
            None
        }
    }

    fn persist_session(&mut self) {
        if self.session_restore.is_some() {
            // The tablature does not yet reflect the saved cursor and selection.
            return;
        }
        let (measure_index, beat_index, selection_start, selection_end) =
            if let Some(LoadedSong { tablature, .. }) = &self.song {
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
            .unwrap_or(self.prefs.master_volume);

        let session = SessionState {
            last_tab_file: self.current_tab_path.clone(),
            track_index: self.song.as_ref().map_or(0, |song| song.track.index),
            tempo_percentage: self.prefs.tempo.percentage,
            master_volume,
            measure_index,
            beat_index,
            selection_start,
            selection_end,
        };
        self.prefs.master_volume = master_volume;
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
        let Some(song) = &mut self.song else {
            return Task::none();
        };

        let track_count = song.tracks.len();
        if track_count == 0 {
            return Task::none();
        }
        let track = session.track_index.min(track_count - 1);
        if track != song.track.index {
            song.track = song.tracks[track].clone();
            song.tablature.update_track(track);
        }
        let tablature = &mut song.tablature;

        let tempo = session.tempo_percentage.clamp(25, 200);
        self.prefs.tempo = TempoSelection::new(tempo);

        let volume = session.master_volume.clamp(0.0, 1.0);
        self.prefs.master_volume = volume;

        let measure_count = tablature.measure_count();
        if measure_count == 0 {
            return Task::none();
        }
        let measure = session.measure_index.min(measure_count - 1);
        let beat = session.beat_index;
        tablature.focus_on_measure_beat(measure, beat);
        if let Some(player) = &self.audio_player {
            player.set_tempo_percentage(tempo);
            player.set_master_volume(volume);
            let beat_offset = tablature.beat_tick_offset(measure, tablature.focused_beat());
            player.focus_measure_at(measure, beat_offset);
        }

        match (session.selection_start, session.selection_end) {
            (Some(start), Some(end)) => {
                let lo = start.min(end).min(measure_count - 1);
                let hi = start.max(end).min(measure_count - 1);
                tablature.restore_selection(lo, hi);
            }
            _ => tablature.clear_selection(),
        }

        let follow_track = song
            .overview
            .follow_track(track)
            .map_or_else(Task::none, |(id, offset)| scroll_to(id, offset));
        let scroll_id = tablature.scroll_id.clone();
        let scroll_task = tablature
            .page_scroll_offset(measure)
            .map_or_else(Task::none, |y| {
                scroll_to(scroll_id, AbsoluteOffset { x: 0.0, y })
            });
        self.persist_session();
        Task::batch([scroll_task, follow_track])
    }

    pub fn start(args: ApplicationArgs) -> iced::Result {
        let antialiasing = !args.no_antialiasing;
        iced::application(move || Self::boot(&args), Self::update, Self::view)
            .title(Self::title)
            .subscription(Self::subscription)
            .default_font(iced::Font::MONOSPACE)
            .theme(Self::theme)
            .font(ICONS_FONT)
            .window_size(WINDOW_SIZE)
            .centered()
            .exit_on_close_request(false)
            .antialiasing(antialiasing)
            .run()
    }

    fn title(&self) -> String {
        match &self.song {
            Some(song) => format!("Ruxguitar - {}", song.info.file_name),
            None => String::from("Ruxguitar - untitled"),
        }
    }

    fn focus_measure_with_scroll(&mut self, measure_id: usize) -> Task<Message> {
        let Some(LoadedSong { tablature, .. }) = &mut self.song else {
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

    /// A file was read: parse it, show it and get it ready to play.
    fn on_file_opened(
        &mut self,
        result: Result<(Vec<u8>, Option<PathBuf>, String, PathBuf), FilePickerError>,
    ) -> Task<Message> {
        self.tab_file_is_loading = false;
        // a startup restore, like a track asked for on the command line,
        // only applies to the file opened at startup, whether or not that
        // load succeeds
        let restore_session = SessionRestore::take_on_load(&mut self.session_restore);
        let startup_track = self.startup_track.take();
        let (contents, parent_folder, file_name, tab_path) = match result {
            Ok(file) => file,
            Err(FilePickerError::DialogClosed) => return Task::none(),
            Err(err) => {
                return Task::done(Message::ReportError(format!("Failed to open file: {err}")));
            }
        };
        let Ok(song) = parse_gp_data(&contents) else {
            return Task::done(Message::ReportError("Failed to parse GP file".to_string()));
        };
        let track_selections = TrackSelection::all_of(&song);
        if track_selections.is_empty() {
            return Task::done(Message::ReportError(
                "No tracks found in GP file".to_string(),
            ));
        }
        let info = SongDisplayInfo::new(&song, file_name);
        // the first track, unless the command line asked for another
        let (default_track, track_error) = opening_track(startup_track, track_selections.len());
        let track = track_selections[default_track].clone();
        // share song ownership with tablature and player
        let song_rc = Rc::new(song);
        let playback_order = compute_playback_order(&song_rc.measure_headers);
        let mut prefs = self.prefs;
        if restore_session.is_none() {
            prefs.tempo = TempoSelection::default();
            prefs.master_volume = 1.0;
        }
        let (audio_player, audio_error) = match AudioPlayer::new(
            song_rc.clone(),
            song_rc.tempo.value,
            prefs.tempo.percentage,
            self.sound_font_file.clone(),
            self.current_tick.clone(),
            self.beat_notify.clone(),
            &playback_order,
        ) {
            Ok(player) => {
                prefs.apply_to(&player);
                (Some(player), Task::none())
            }
            Err(err) => {
                let error = Task::done(Message::ReportError(format!(
                    "Failed to initialize audio: {err}"
                )));
                if self.audio_player.is_some() {
                    return error;
                }
                // Without a working player, the new tablature can still be read.
                (None, error)
            }
        };
        if let Err(err) = self.config.set_tabs_folder(parent_folder) {
            return Task::done(Message::ReportError(format!(
                "Failed to set tabs folder: {err}"
            )));
        }
        if let Some(player) = &mut self.audio_player {
            player.stop();
        }
        self.audio_player = audio_player;
        self.prefs = prefs;
        self.session_restore = restore_session.map(SessionRestore::OnLayout);
        let tablature_scroll_id = Id::new("tablature-scroll-elements");
        let tablature = Tablature::new(
            song_rc.clone(),
            default_track,
            tablature_scroll_id.clone(),
            &playback_order,
        );
        let mut overview = TrackOverview::new(&song_rc, self.window_height);
        // its view is not known yet: the row shows once it is
        let _ = overview.follow_track(default_track);
        self.song = Some(LoadedSong {
            info,
            tracks: track_selections,
            track,
            tablature,
            overview,
        });
        self.current_tab_path = Some(tab_path);
        self.persist_session();
        Task::batch([
            audio_error,
            scroll_to(tablature_scroll_id, AbsoluteOffset::<f32>::default()),
            Task::done(Message::WindowResized),
            track_error.map_or_else(Task::none, |error| Task::done(Message::ReportError(error))),
        ])
    }

    /// The press on the tablature was released: settle the selection, the
    /// loop and the focus.
    fn on_selection_drag_end(&mut self) -> Task<Message> {
        let Some(DragRelease { moved, focus_beat }) = self.selection_drag.release() else {
            return Task::none();
        };
        if let Some(LoadedSong { tablature, .. }) = &mut self.song {
            tablature.set_pointer_drag_active(false);
            if !moved {
                tablature.clear_selection();
            }
        }
        // keep a running loop in step with the selection
        let loop_start = self.sync_measure_loop();
        let focus_task = if let Some(LoadedSong { tablature, .. }) = &mut self.song {
            // a new loop starts playing from its first measure
            let (measure_id, beat_id) = match (loop_start, tablature.selection_range()) {
                (Some(first_measure), _) => (first_measure, 0),
                (None, Some((start, _))) => (start, 0),
                (None, None) => (tablature.selection_anchor(), focus_beat),
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

    fn on_play_pause(&mut self) -> Task<Message> {
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
            let outside_loop = self.song.as_ref().and_then(|song| {
                let tablature = &song.tablature;
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

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        Task::batch([task, self.follow_in_overview()])
    }

    /// Scroll the overview to the focused measure when it moved out of view.
    fn follow_in_overview(&mut self) -> Task<Message> {
        let Some(song) = &mut self.song else {
            return Task::none();
        };
        let is_playing = self
            .audio_player
            .as_ref()
            .is_some_and(AudioPlayer::is_playing);
        let margin = if is_playing { PLAYING_MARGIN } else { 0 };
        song.overview
            .follow(song.tablature.focused_measure(), margin)
            .map_or_else(Task::none, |(id, offset)| scroll_to(id, offset))
    }

    /// Show another track, keeping the focused measure, and bring its row
    /// of the overview into view.
    fn select_track(&mut self, index: usize) -> Task<Message> {
        let Some(song) = &mut self.song else {
            return Task::none();
        };
        let Some(selection) = song.tracks.get(index).cloned() else {
            return Task::none();
        };
        song.tablature.update_track(index);
        song.track = selection;
        let follow = song.overview.follow_track(index);
        self.persist_session();
        Task::batch([
            follow.map_or_else(Task::none, |(id, offset)| scroll_to(id, offset)),
            self.notify_track(),
        ])
    }

    /// The track `step` away from the current one, if any.
    fn track_step(&self, step: isize) -> Option<usize> {
        let song = self.song.as_ref()?;
        song.track
            .index
            .checked_add_signed(step)
            .filter(|&index| index < song.tracks.len())
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TrackOverviewTrack(index) => self.select_track(index),
            Message::NextTrack => self
                .track_step(1)
                .map_or_else(Task::none, |index| self.select_track(index)),
            Message::PreviousTrack => self
                .track_step(-1)
                .map_or_else(Task::none, |index| self.select_track(index)),
            Message::TrackOverviewMeasure(track, measure) => {
                let select = if self.song.as_ref().is_some_and(|s| s.track.index != track) {
                    self.select_track(track)
                } else {
                    Task::none()
                };
                Task::batch([select, self.focus_measure_with_scroll(measure)])
            }
            Message::TrackOverviewScrolled(viewport) => self
                .song
                .as_mut()
                .and_then(|song| song.overview.squares_scrolled(viewport))
                .map_or_else(Task::none, |(id, offset)| scroll_to(id, offset)),
            Message::TrackOverviewLabelsScrolled(viewport) => self
                .song
                .as_mut()
                .and_then(|song| song.overview.labels_scrolled(viewport))
                .map_or_else(Task::none, |(id, offset)| scroll_to(id, offset)),
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
            Message::FileOpened(result) => self.on_file_opened(result),
            Message::SelectionDragStart(measure_id, beat_id, press_position) => {
                self.selection_drag = SelectionDrag::press(press_position, beat_id);
                if let Some(LoadedSong { tablature, .. }) = &mut self.song {
                    tablature.set_pointer_drag_active(true);
                    tablature.begin_selection_drag(measure_id);
                }
                Task::none()
            }
            Message::SelectionDragTo(measure_id, cursor_position) => {
                if self.selection_drag.move_to(cursor_position)
                    && let Some(LoadedSong { tablature, .. }) = &mut self.song
                {
                    tablature.extend_selection_drag(measure_id);
                }
                Task::none()
            }
            Message::SelectionDragEnd => self.on_selection_drag_end(),
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
                let Some(LoadedSong { tablature, .. }) = &mut self.song else {
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
                let target = self
                    .song
                    .as_ref()
                    .map(|song| &song.tablature)
                    .and_then(|t| {
                        let next = t.focused_measure() + 1;
                        (next < t.measure_count()).then_some(next)
                    });
                target.map_or_else(Task::none, |m| self.focus_measure_with_scroll(m))
            }
            Message::PreviousMeasure => {
                let target = self
                    .song
                    .as_ref()
                    .and_then(|song| song.tablature.focused_measure().checked_sub(1));
                target.map_or_else(Task::none, |m| self.focus_measure_with_scroll(m))
            }
            Message::PlayPause => self.on_play_pause(),
            Message::StopPlayer => {
                if self.audio_player.is_some() && self.song.is_some() {
                    self.persist_session();
                }
                if let (Some(audio_player), Some(LoadedSong { tablature, .. })) =
                    (&mut self.audio_player, &mut self.song)
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
                if let (Some(audio_player), Some(song)) = (&self.audio_player, &self.song) {
                    audio_player.toggle_track_solo(song.track.index);
                }
                self.notify_track()
            }
            Message::ToggleMute => {
                if let (Some(audio_player), Some(song)) = (&self.audio_player, &self.song) {
                    audio_player.toggle_track_mute(song.track.index);
                }
                self.notify_track()
            }
            Message::ToggleTrackSolo(track_index) => {
                if let Some(audio_player) = &self.audio_player {
                    audio_player.toggle_track_solo(track_index);
                }
                Task::none()
            }
            Message::ToggleTrackMute(track_index) => {
                if let Some(audio_player) = &self.audio_player {
                    audio_player.toggle_track_mute(track_index);
                }
                Task::none()
            }
            Message::ToggleMetronome => {
                self.prefs.metronome = !self.prefs.metronome;
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_metronome(self.prefs.metronome);
                }
                Task::none()
            }
            Message::ToggleCountIn => {
                self.prefs.count_in = !self.prefs.count_in;
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_count_in(self.prefs.count_in);
                }
                Task::none()
            }
            Message::WindowSized(size) => {
                self.window_height = size.height;
                if let Some(song) = &mut self.song {
                    song.overview.window_resized(size.height);
                }
                Task::done(Message::WindowResized)
            }
            Message::DesktopToneChanged(tone) => {
                self.theme = rux_theme(is_dark(self.theme_choice, tone));
                Task::none()
            }
            Message::NoticeExpired(count) => {
                if count == self.notice_count {
                    self.notice = None;
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
                if let Some(LoadedSong { tablature, .. }) = &mut self.song {
                    tablature.update_container_size(
                        tablature_container_size.width,
                        tablature_container_size.height,
                    );
                }
                if tablature_container_size.width > 0.0
                    && let Some(session) = SessionRestore::take_on_layout(&mut self.session_restore)
                {
                    return self.apply_session_restore(&session);
                }
                Task::none()
            }
            Message::TempoSelected(tempos_selection) => {
                if let Some(audio_player) = &self.audio_player {
                    audio_player.set_tempo_percentage(tempos_selection.percentage);
                }
                self.prefs.tempo = tempos_selection;
                self.persist_session();
                Task::none()
            }
            Message::IncreaseTempo => {
                if self.tab_file_is_loading {
                    return Task::none();
                }
                self.prefs.tempo.faster().map_or_else(Task::none, |tempo| {
                    Task::done(Message::TempoSelected(tempo))
                })
            }
            Message::DecreaseTempo => {
                if self.tab_file_is_loading {
                    return Task::none();
                }
                self.prefs.tempo.slower().map_or_else(Task::none, |tempo| {
                    Task::done(Message::TempoSelected(tempo))
                })
            }
            Message::ToggleFullscreen => {
                self.is_fullscreen = !self.is_fullscreen;
                self.notice = None;
                if !self.is_fullscreen
                    && let Some(song) = &mut self.song
                {
                    // shown again, the overview starts scrolled back: follow
                    // the focused measure once it reports its view
                    song.overview.reset_view(song.track.index);
                }
                let mode = if self.is_fullscreen {
                    window::Mode::Fullscreen
                } else {
                    window::Mode::Windowed
                };
                window::latest().and_then(move |id| window::set_mode(id, mode))
            }
            Message::MasterVolumeChanged(volume) => {
                self.prefs.master_volume = volume;
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
}

/// Whether to run on the dark tone.
///
/// An explicit choice always wins, then the desktop's tone - except on
/// Linux, where winit often reports none: the dark tab stays the default
/// there, and the light one elsewhere, as iced would pick.
fn is_dark(choice: Option<ThemeChoice>, desktop_tone: theme::Mode) -> bool {
    match (choice, desktop_tone) {
        (Some(ThemeChoice::Dark), _) | (None, theme::Mode::Dark) => true,
        (Some(ThemeChoice::Light), _) | (None, theme::Mode::Light) => false,
        (None, theme::Mode::None) => cfg!(target_os = "linux"),
    }
}

#[cfg(test)]
mod demo;

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_demo_song() -> RuxApplication {
        let mut app = RuxApplication::new(None, Config::in_memory(SessionState::default()));
        let path = PathBuf::from("test-files/Demo v5.gp5");
        let _ = app.update(Message::FileOpened(Ok((
            std::fs::read(&path).unwrap(),
            None,
            "Demo v5.gp5".into(),
            path,
        ))));
        assert!(app.audio_player.is_some());
        app
    }

    #[test]
    fn dragging_backwards_selects_a_range_and_clicking_seeks_to_a_beat() {
        let mut app = app_with_demo_song();
        let _ = app.update(Message::TrackOverviewTrack(2));
        let press = Point::new(100.0, 100.0);
        let _ = app.update(Message::SelectionDragStart(9, 0, press));
        let _ = app.update(Message::SelectionDragTo(5, Point::new(40.0, 100.0)));
        let _ = app.update(Message::SelectionDragEnd);
        let tab = &app.song.as_ref().unwrap().tablature;
        assert_eq!(tab.selection_range(), Some((5, 9)));
        assert_eq!((tab.focused_measure(), tab.focused_beat()), (5, 0));
        let saved = app.config.session();
        assert_eq!(
            (saved.selection_start, saved.selection_end),
            (Some(5), Some(9))
        );

        let measure = (0..tab.measure_count())
            .find(|&m| tab.beat_tick_offset(m, 1) > 0)
            .expect("demo track has a measure with a second beat");
        let _ = app.update(Message::SelectionDragStart(measure, 1, press));
        let _ = app.update(Message::SelectionDragTo(measure, Point::new(102.0, 100.0)));
        let _ = app.update(Message::SelectionDragEnd);
        let tab = &app.song.as_ref().unwrap().tablature;
        assert_eq!(tab.selection_range(), None);
        assert_eq!((tab.focused_measure(), tab.focused_beat()), (measure, 1));
        let saved = app.config.session();
        assert_eq!((saved.selection_start, saved.selection_end), (None, None));
        assert_eq!((saved.measure_index, saved.beat_index), (measure, 1));
        let tick = app.current_tick.load(std::sync::atomic::Ordering::Relaxed);
        let _ = app.update(Message::FocusTick(tick));
        let tab = &app.song.as_ref().unwrap().tablature;
        assert_eq!((tab.focused_measure(), tab.focused_beat()), (measure, 1));
    }

    #[test]
    fn measure_navigation_stops_at_the_ends_and_stop_saves_the_resume_position() {
        let mut app = app_with_demo_song();
        let _ = app.update(Message::PreviousMeasure);
        assert_eq!(app.song.as_ref().unwrap().tablature.focused_measure(), 0);
        let last = app.song.as_ref().unwrap().tablature.measure_count() - 1;
        let _ = app.update(Message::TrackOverviewMeasure(1, last - 1));
        assert_eq!(app.config.session().track_index, 1);
        assert_eq!(app.config.session().measure_index, last - 1);
        let _ = app.update(Message::NextMeasure);
        let _ = app.update(Message::NextMeasure);
        assert_eq!(app.song.as_ref().unwrap().tablature.focused_measure(), last);
        let _ = app.update(Message::PreviousMeasure);
        assert_eq!(
            app.song.as_ref().unwrap().tablature.focused_measure(),
            last - 1
        );
        // Playback moves the cursor without saving on every tick.
        let second_measure_tick =
            app.song.as_ref().unwrap().tablature.song.measure_headers[1].start;
        let _ = app.update(Message::FocusTick(second_measure_tick));
        assert_eq!(app.song.as_ref().unwrap().tablature.focused_measure(), 1);
        let _ = app.update(Message::StopPlayer);
        assert_eq!(app.song.as_ref().unwrap().tablature.focused_measure(), 0);
        assert_eq!(app.config.session().measure_index, 1);
    }

    #[test]
    fn track_controls_address_the_selected_track_or_the_named_overview_track() {
        let mut app = app_with_demo_song();
        let _ = app.update(Message::TrackOverviewTrack(1));
        let _ = app.update(Message::ToggleSolo);
        let _ = app.update(Message::ToggleTrackMute(0));
        let player = app.audio_player.as_ref().unwrap();
        assert!(player.is_track_solo(1));
        assert!(!player.is_track_solo(0));
        assert!(player.is_track_muted(0));
        assert!(!player.is_track_muted(1));

        let _ = app.update(Message::TrackOverviewTrack(0));
        let _ = app.update(Message::ToggleTrackSolo(1));
        let _ = app.update(Message::ToggleMute);
        let player = app.audio_player.as_ref().unwrap();
        assert!(!player.is_track_solo(1));
        assert!(!player.is_track_muted(0));
    }

    #[test]
    fn up_and_down_step_through_the_tracks_and_stop_at_the_ends() {
        let mut app = app_with_demo_song();
        let last = app.song.as_ref().unwrap().tracks.len() - 1;
        assert!(last > 0);
        let current = |app: &RuxApplication| app.song.as_ref().unwrap().track.index;
        let _ = app.update(Message::PreviousTrack);
        assert_eq!(current(&app), 0);
        let _ = app.update(Message::NextTrack);
        assert_eq!(current(&app), 1);
        assert_eq!(app.config.session().track_index, 1);
        let _ = app.update(Message::TrackOverviewTrack(last));
        let _ = app.update(Message::NextTrack);
        assert_eq!(current(&app), last);
    }

    #[test]
    fn a_track_change_is_told_in_fullscreen_only_and_cleared_by_its_own_expiry() {
        let mut app = app_with_demo_song();
        let _ = app.update(Message::NextTrack);
        assert_eq!(app.notice, None);
        let _ = app.update(Message::ToggleFullscreen);
        let _ = app.update(Message::NextTrack);
        let first = app.notice.clone().expect("a notice in fullscreen");
        assert!(first.starts_with("3 - "));
        let _ = app.update(Message::ToggleSolo);
        let second = app.notice.clone().unwrap();
        assert!(second.ends_with("solo"));
        // the first notice's expiry leaves the second one up
        let _ = app.update(Message::NoticeExpired(app.notice_count - 1));
        assert_eq!(app.notice, Some(second));
        let _ = app.update(Message::NoticeExpired(app.notice_count));
        assert_eq!(app.notice, None);
        // leaving fullscreen takes the notice with it
        let _ = app.update(Message::ToggleMute);
        assert!(app.notice.is_some());
        let _ = app.update(Message::ToggleFullscreen);
        assert_eq!(app.notice, None);
    }

    #[test]
    fn the_theme_follows_the_choice_then_the_desktop() {
        use theme::Mode;
        let mut app = RuxApplication::new(None, Config::in_memory(SessionState::default()));
        let _ = app.update(Message::DesktopToneChanged(Mode::Light));
        assert_eq!(app.theme(), rux_theme(false));
        let _ = app.update(Message::DesktopToneChanged(Mode::Dark));
        assert_eq!(app.theme(), rux_theme(true));
        app.theme_choice = Some(ThemeChoice::Light);
        let _ = app.update(Message::DesktopToneChanged(Mode::Dark));
        assert_eq!(app.theme(), rux_theme(false));

        assert!(is_dark(Some(ThemeChoice::Dark), Mode::Light));
        assert!(!is_dark(Some(ThemeChoice::Light), Mode::Dark));
        assert!(is_dark(None, Mode::Dark));
        assert!(!is_dark(None, Mode::Light));
        assert_eq!(is_dark(None, Mode::None), cfg!(target_os = "linux"));
    }

    #[test]
    fn the_track_asked_for_opens_numbered_from_one() {
        assert_eq!(opening_track(None, 8), (0, None));
        assert_eq!(opening_track(Some(1), 8), (0, None));
        assert_eq!(opening_track(Some(8), 8), (7, None));
    }

    #[test]
    fn canceling_or_failing_to_replace_a_file_keeps_its_song_and_player() {
        let mut app = app_with_demo_song();
        let path = app.current_tab_path.clone().unwrap();
        let contents = std::fs::read(&path).unwrap();
        let song = app.song.as_ref().unwrap().tablature.song.clone();
        let player = app.audio_player.as_ref().unwrap();
        player.focus_measure_at(2, 0);
        player.toggle_track_solo(0);
        // A valid replacement also has to initialize its own sound font.
        app.sound_font_file = Some(path.with_extension("missing-soundfont"));
        let replacement = |contents| Ok((contents, None, "new.gp5".into(), "new.gp5".into()));
        let tick = app.current_tick.load(std::sync::atomic::Ordering::Relaxed);
        for (result, reports_error) in [
            (Err(FilePickerError::DialogClosed), false),
            (Err(FilePickerError::IoError("unreadable".into())), true),
            (replacement(Vec::new()), true),
            (replacement(contents), true),
        ] {
            app.tab_file_is_loading = true;
            let task = app.on_file_opened(result);
            assert_eq!(task.units() > 0, reports_error);
            assert!(!app.tab_file_is_loading);
            assert!(app.audio_player.as_ref().unwrap().is_track_solo(0));
            assert_eq!(
                app.current_tick.load(std::sync::atomic::Ordering::Relaxed),
                tick
            );
            assert!(Rc::ptr_eq(
                &app.song.as_ref().unwrap().tablature.song,
                &song
            ));
            assert_eq!(app.current_tab_path.as_ref(), Some(&path));
        }
    }

    #[test]
    fn a_failed_soundfont_still_restores_the_saved_tablature() {
        let path = PathBuf::from("test-files/Demo v5.gp5");
        let session = SessionState {
            last_tab_file: Some(path.clone()),
            track_index: 2,
            measure_index: 8,
            beat_index: 1,
            selection_start: Some(5),
            selection_end: Some(9),
            tempo_percentage: 75,
            master_volume: 0.5,
        };
        let config = Config::in_memory(session.clone());
        let mut app = RuxApplication::new(Some(path.with_extension("missing-soundfont")), config);
        app.session_restore = Some(SessionRestore::OnLoad(session));
        let _ = app.on_file_opened(Ok((
            std::fs::read(&path).unwrap(),
            None,
            "Demo v5.gp5".into(),
            path,
        )));
        assert!(app.audio_player.is_none());
        assert_eq!(app.config.session().track_index, 2);
        assert_eq!(app.config.session().measure_index, 8);

        let _ = app.handle(Message::TablatureResized(Size::new(1024.0, 768.0)));
        let song = app.song.as_ref().unwrap();
        assert_eq!(song.track.index, 2);
        assert_eq!(song.tablature.focused_measure(), 8);
        assert_eq!(song.tablature.focused_beat(), 1);
        assert_eq!(song.tablature.selection_range(), Some((5, 9)));
        assert_eq!(app.config.session().tempo_percentage, 75);
        assert_eq!(app.config.session().master_volume, 0.5);
    }

    #[test]
    fn a_startup_track_does_not_outlive_a_failed_startup_load() {
        let mut app = RuxApplication::new(None, Config::default());
        app.startup_track = Some(3);
        // the file given on the command line cannot be read
        let _ = app.on_file_opened(Err(FilePickerError::IoError("unreadable".to_string())));
        // the next file opened starts on its first track again
        assert_eq!(app.startup_track, None);
    }

    #[test]
    fn a_track_the_song_lacks_opens_the_first_with_an_error() {
        let (track, error) = opening_track(Some(9), 8);
        assert_eq!(track, 0);
        assert_eq!(
            error.as_deref(),
            Some("Track 9 not found: the song has 8 tracks")
        );
        let (_, error) = opening_track(Some(2), 1);
        assert_eq!(
            error.as_deref(),
            Some("Track 2 not found: the song has 1 track")
        );
    }

    #[test]
    fn a_layout_pass_while_loading_keeps_the_startup_restore() {
        let mut slot = Some(SessionRestore::OnLoad(SessionState::default()));
        assert!(SessionRestore::take_on_layout(&mut slot).is_none());
        assert!(matches!(slot, Some(SessionRestore::OnLoad(_))));
        assert!(SessionRestore::take_on_load(&mut slot).is_some());
        assert!(slot.is_none());
    }

    #[test]
    fn a_load_keeps_a_restore_waiting_for_the_layout() {
        let mut slot = Some(SessionRestore::OnLayout(SessionState::default()));
        assert!(SessionRestore::take_on_load(&mut slot).is_none());
        assert!(SessionRestore::take_on_layout(&mut slot).is_some());
        assert!(slot.is_none());
    }
}
