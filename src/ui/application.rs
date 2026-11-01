use iced::widget::operation::scroll_to;
use iced::widget::{Id, selector};
use iced::{Point, Size, Task, Theme, window};

use crate::audio::midi_player::AudioPlayer;
use crate::audio::playback_order::compute_playback_order;
use crate::config::{Config, SessionState};
use crate::parser::parse_gp_data;
use crate::ui::picker::{FilePickerError, load_file, open_file_dialog};
use crate::ui::selection_drag::{DragRelease, SelectionDrag};
use crate::ui::selections::{TempoSelection, TrackSelection};
use crate::ui::song_info::SongDisplayInfo;
use crate::ui::tablature::Tablature;
use crate::{ApplicationArgs, ThemeChoice};
use iced::widget::scrollable::AbsoluteOffset;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use tokio::sync::Notify;

mod subscription;
mod view;

const ICONS_FONT: &[u8] = include_bytes!("../../resources/icons.ttf");

pub struct RuxApplication {
    /// The open song, if any.
    song: Option<LoadedSong>,
    tablature_id: Id,
    /// Absent while no song is open, or when the audio failed to start.
    audio_player: Option<AudioPlayer>,
    prefs: PlaybackPrefs,
    tab_file_is_loading: bool,
    sound_font_file: Option<PathBuf>,
    /// Latest tick published by the audio callback, with its wake-up signal.
    current_tick: Arc<AtomicU32>,
    beat_notify: Arc<Notify>,
    config: Config,
    error_message: Option<String>,
    is_fullscreen: bool,
    selection_drag: SelectionDrag,
    current_tab_path: Option<PathBuf>,
    session_restore: Option<SessionRestore>,
}

/// A parsed song, with its tablature and the track it shows.
struct LoadedSong {
    info: SongDisplayInfo,
    tracks: Vec<TrackSelection>,
    track: TrackSelection,
    tablature: Tablature,
}

/// Playback preferences, kept at the app level so they survive reloading a file.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PlaybackPrefs {
    tempo: TempoSelection,
    metronome: bool,
    count_in: bool,
    /// The volume to give a new player; the live one is the player's own.
    master_volume: f32,
}

impl PlaybackPrefs {
    fn apply_to(&self, audio_player: &AudioPlayer) {
        audio_player.set_metronome(self.metronome);
        audio_player.set_count_in(self.count_in);
        audio_player.set_master_volume(self.master_volume);
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
            selection_drag: SelectionDrag::Idle,
            current_tab_path: None,
            session_restore: None,
        }
    }

    fn boot(args: &ApplicationArgs) -> (Self, Task<Message>) {
        let mut app = Self::new(args.sound_font_bank.clone(), args.local_config.clone());

        let init_task = if let Some(path) = &args.tab_file_path {
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
        (app, init_task)
    }

    /// Fresh tab defaults when the user opens a GP file (not startup session restore).
    fn reset_tab_defaults(&mut self) {
        self.prefs.tempo = TempoSelection::default();
        self.prefs.master_volume = 1.0;
        self.session_restore = None;
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
        let (Some(song), Some(audio_player)) = (&mut self.song, &self.audio_player) else {
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
        audio_player.set_tempo_percentage(tempo);

        let volume = session.master_volume.clamp(0.0, 1.0);
        self.prefs.master_volume = volume;
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
        // stop and drop the previous audio player if any: if loading
        // fails below, the old song must not keep playing against the
        // new tablature state
        if let Some(audio_player) = &mut self.audio_player {
            audio_player.stop();
        }
        self.audio_player = None;
        // a startup restore only applies to the file it reopened,
        // whether or not that load succeeds
        let restore_session = SessionRestore::take_on_load(&mut self.session_restore);
        let (contents, parent_folder, file_name, tab_path) = match result {
            Ok(file) => file,
            Err(err) => {
                return Task::done(Message::ReportError(format!("Failed to open file: {err}")));
            }
        };
        if let Err(err) = self.config.set_tabs_folder(parent_folder) {
            return Task::done(Message::ReportError(format!(
                "Failed to set tabs folder: {err}"
            )));
        }
        let Ok(song) = parse_gp_data(&contents) else {
            return Task::done(Message::ReportError("Failed to parse GP file".to_string()));
        };
        if let Some(session) = restore_session {
            self.session_restore = Some(SessionRestore::OnLayout(session));
        } else {
            self.reset_tab_defaults();
        }
        let track_selections = TrackSelection::all_of(&song);
        if track_selections.is_empty() {
            return Task::done(Message::ReportError(
                "No tracks found in GP file".to_string(),
            ));
        }
        let info = SongDisplayInfo::new(&song, file_name);
        // select first track by default
        let default_track = 0;
        let track = track_selections[default_track].clone();
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
        self.song = Some(LoadedSong {
            info,
            tracks: track_selections,
            track,
            tablature,
        });
        let audio_player = match AudioPlayer::new(
            song_rc.clone(),
            song_rc.tempo.value,
            self.prefs.tempo.percentage,
            self.sound_font_file.clone(),
            self.current_tick.clone(),
            self.beat_notify.clone(),
            &playback_order,
        ) {
            Ok(audio_player) => audio_player,
            Err(err) => {
                return Task::done(Message::ReportError(format!(
                    "Failed to initialize audio: {err}"
                )));
            }
        };
        // apply persistent playback preferences
        self.prefs.apply_to(&audio_player);
        self.audio_player = Some(audio_player);
        self.current_tab_path = Some(tab_path);
        self.persist_session();
        Task::batch([
            scroll_to(tablature_scroll_id, AbsoluteOffset::<f32>::default()),
            Task::done(Message::WindowResized),
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
        match message {
            Message::TrackSelected(selection) => {
                if let Some(song) = &mut self.song {
                    song.tablature.update_track(selection.index);
                    song.track = selection;
                }
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
                    audio_player.toggle_solo_mode(song.track.index);
                }
                Task::none()
            }
            Message::ToggleMute => {
                if let (Some(audio_player), Some(song)) = (&self.audio_player, &self.song) {
                    audio_player.toggle_track_mute(song.track.index);
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

#[cfg(test)]
mod tests {
    use super::*;

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
