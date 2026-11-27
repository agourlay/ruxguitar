//! Records the demo shown by the README and the landing page: the real
//! interface, rendered frame by frame without a window, driven through a
//! short practice session. Run by `site/demo.sh`.
//!
//! The cursor is painted on the frames, and playback is paced by hand
//! rather than by the audio clock, so a recording never depends on the
//! machine it runs on.

use super::*;
use crate::parser::song_parser::QUARTER_TIME;
use iced::advanced::renderer::{Headless, Style};
use iced::advanced::widget::operation::scrollable::scroll_to;
use iced::advanced::{clipboard, graphics};
use iced::mouse::{self, Cursor};
use iced::{Event, Font, Renderer};
use iced_runtime::user_interface::{Cache, UserInterface};
use std::borrow::Cow;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

const WIDTH: u32 = 1150;
const HEIGHT: u32 = 768;
const FPS: u32 = 15;
/// The tablature's room in a window of that size, above a nine-track overview.
const TABLATURE_SIZE: Size = Size::new(1130.0, 420.0);

const TAB_FILE: &str = "test-files/Dream Theater - Pull Me Under.gp5";
/// The riff the demo loops, on the second guitar: track and measures from 0.
const TRACK: usize = 2;
const LOOP: (usize, usize) = (37, 38);
const TEMPO: u32 = 80;

// Where the cursor goes. These follow the layout: check the recording after
// changing the interface.
const START: Point = Point::new(640.0, 430.0);
/// The riff's first measure on the track's row of the overview.
const OVERVIEW_MEASURE: Point = Point::new(788.0, 580.0);
/// The track's solo toggle.
const SOLO: Point = Point::new(193.0, 580.0);
/// Inside the loop's first and last measures, once the tablature shows them.
const DRAG: (Point, Point) = (Point::new(200.0, 140.0), Point::new(580.0, 146.0));
const TEMPO_PICKER: Point = Point::new(800.0, 36.0);
const PLAY: Point = Point::new(275.0, 36.0);
/// Out of the way while the loop plays.
const REST: Point = Point::new(359.0, 62.0);

/// An arrow with its tip on the first pixel: `X` outline, `.` fill.
const ARROW: [&str; 18] = [
    "X           ",
    "XX          ",
    "X.X         ",
    "X..X        ",
    "X...X       ",
    "X....X      ",
    "X.....X     ",
    "X......X    ",
    "X.......X   ",
    "X........X  ",
    "X.........X ",
    "X......XXXXX",
    "X...X..X    ",
    "X..X X..X   ",
    "X.X  X..X   ",
    "XX    X..X  ",
    "X     X..X  ",
    "       XX   ",
];

/// The frames of the recording, encoded as they are filmed.
struct Film {
    renderer: Renderer,
    /// Widget state kept from a frame to the next, as a window would.
    cache: Cache,
    encoder: Child,
    cursor: Point,
    /// Where the tablature is scrolled.
    tablature_scroll: f32,
}

impl Film {
    fn new(gif: &str) -> Self {
        // the icons of the toolbar
        graphics::text::font_system()
            .write()
            .expect("font system")
            .load_font(Cow::Borrowed(ICONS_FONT));
        let renderer = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime")
            .block_on(Renderer::new(
                Font::MONOSPACE,
                16.0.into(),
                Some("tiny-skia"),
            ))
            .expect("tiny-skia headless renderer");
        // few colors and only what changed in a frame: the picture is flat
        // and mostly still, which keeps the file small
        let filter = "split[a][b];[a]palettegen=max_colors=48:stats_mode=diff[p];\
                      [b][p]paletteuse=dither=none:diff_mode=rectangle";
        let encoder = ffmpeg(FPS)
            .args(["-filter_complex", filter, gif])
            .spawn()
            .expect("ffmpeg on the PATH");
        Self {
            renderer,
            cache: Cache::new(),
            encoder,
            cursor: START,
            tablature_scroll: 0.0,
        }
    }

    /// The interface as it stands, in RGB, without the cursor.
    fn render(&mut self, app: &RuxApplication) -> Vec<u8> {
        let theme = app.theme();
        let cursor = Cursor::Available(self.cursor);
        let size = Size::new(WIDTH as f32, HEIGHT as f32);
        let cache = std::mem::take(&mut self.cache);
        let mut interface = UserInterface::build(app.view(), size, cache, &mut self.renderer);
        if let Some(song) = &app.song {
            let offset = AbsoluteOffset {
                x: None,
                y: Some(self.tablature_scroll),
            };
            let mut scroll = scroll_to::<()>(song.tablature.scroll_id.clone(), offset);
            interface.operate(&self.renderer, &mut scroll);
        }
        // widgets take their looks from the events they saw: without a
        // redraw request, buttons draw as disabled
        let events = [
            Event::Mouse(mouse::Event::CursorMoved {
                position: self.cursor,
            }),
            Event::Window(window::Event::RedrawRequested(Instant::now())),
        ];
        let _ = interface.update(
            &events,
            cursor,
            &mut self.renderer,
            &mut clipboard::Null,
            &mut Vec::new(),
        );
        let style = Style {
            text_color: theme.palette().text,
        };
        interface.draw(&mut self.renderer, &theme, &style, cursor);
        self.cache = interface.into_cache();
        self.renderer
            .screenshot(Size::new(WIDTH, HEIGHT), 1.0, theme.palette().background)
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, _]| [r, g, b])
            .collect()
    }

    fn frame(&mut self, app: &RuxApplication, pressed: bool) {
        let mut frame = self.render(app);
        paint_cursor(&mut frame, self.cursor, pressed);
        self.encoder
            .stdin
            .as_mut()
            .expect("ffmpeg stdin")
            .write_all(&frame)
            .expect("frame sent to ffmpeg");
    }

    fn hold(&mut self, app: &RuxApplication, seconds: f32) {
        for _ in 0..frames_in(seconds) {
            self.frame(app, false);
        }
    }

    /// Move the cursor to a point, easing in and out.
    fn glide(&mut self, app: &RuxApplication, to: Point, seconds: f32) {
        let from = self.cursor;
        let steps = frames_in(seconds);
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            self.cursor = between(from, to, t * t * (3.0 - 2.0 * t));
            self.frame(app, false);
        }
    }

    /// Go to a point and press there, long enough to be seen.
    fn click(&mut self, app: &RuxApplication, at: Point, seconds: f32) {
        self.glide(app, at, seconds);
        self.frame(app, true);
        self.frame(app, true);
    }

    /// Turn the tablature's page as the application does to show a measure:
    /// its scroll tasks have no window to run in here.
    fn turn_page(&mut self, app: &mut RuxApplication, measure: usize) {
        let tablature = &mut app.song.as_mut().expect("a song").tablature;
        if let Some(offset) = tablature.page_scroll_offset(measure) {
            self.tablature_scroll = offset;
        }
    }

    fn finish(mut self) {
        drop(self.encoder.stdin.take());
        assert!(self.encoder.wait().expect("ffmpeg ran").success());
    }
}

/// An `ffmpeg` reading raw frames of the recording's size on its input.
fn ffmpeg(fps: u32) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", &format!("{WIDTH}x{HEIGHT}"), "-r", &fps.to_string()])
        .args(["-i", "-"])
        .stdin(Stdio::piped());
    command
}

fn frames_in(seconds: f32) -> u32 {
    (seconds * FPS as f32) as u32
}

fn between(from: Point, to: Point, t: f32) -> Point {
    Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)
}

/// Paint the arrow on a frame, with a ring around its tip while pressed.
fn paint_cursor(frame: &mut [u8], at: Point, pressed: bool) {
    let mut paint = |x: i32, y: i32, color: [u8; 3]| {
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            let index = 3 * (y as usize * WIDTH as usize + x as usize);
            frame[index..index + 3].copy_from_slice(&color);
        }
    };
    let (tip_x, tip_y) = (at.x as i32, at.y as i32);
    if pressed {
        for dy in -12..=12_i32 {
            for dx in -12..=12_i32 {
                if (100..=144).contains(&(dx * dx + dy * dy)) {
                    paint(tip_x + dx, tip_y + dy, [200, 50, 50]);
                }
            }
        }
    }
    for (dy, row) in ARROW.iter().enumerate() {
        for (dx, pixel) in row.bytes().enumerate() {
            let color = match pixel {
                b'X' => [0, 0, 0],
                b'.' => [255, 255, 255],
                _ => continue,
            };
            paint(tip_x + dx as i32, tip_y + dy as i32, color);
        }
    }
}

/// Save a frame as a picture.
fn save_still(frame: &[u8], path: &str) {
    let mut encoder = ffmpeg(1)
        .args(["-frames:v", "1", path])
        .spawn()
        .expect("ffmpeg on the PATH");
    encoder
        .stdin
        .take()
        .expect("ffmpeg stdin")
        .write_all(frame)
        .expect("frame sent to ffmpeg");
    assert!(encoder.wait().expect("ffmpeg ran").success());
}

#[test]
#[ignore = "records the demo: run by site/demo.sh"]
fn record_the_demo() {
    let gif = std::env::var("DEMO_GIF").expect("DEMO_GIF: where to write the recording");
    let still = std::env::var("DEMO_STILL").expect("DEMO_STILL: where to write its still");
    let mut film = Film::new(&gif);

    let mut app = RuxApplication::new(None, Config::in_memory(SessionState::default()));
    app.theme = rux_theme(true);
    let path = PathBuf::from(TAB_FILE);
    let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
    let content = std::fs::read(&path).expect("the demo's tab file");
    let _ = app.update(Message::FileOpened(Ok((content, None, file_name, path))));
    assert!(app.audio_player.is_some(), "the demo needs a player");
    let _ = app.update(Message::TablatureResized(TABLATURE_SIZE));
    let (first, last) = LOOP;
    film.hold(&app, 0.8);

    // pick the riff on the guitar's row of the overview
    film.click(&app, OVERVIEW_MEASURE, 0.9);
    // the track first: its lines are laid out differently
    let _ = app.update(Message::TrackOverviewTrack(TRACK));
    film.turn_page(&mut app, first);
    let _ = app.update(Message::TrackOverviewMeasure(TRACK, first));
    film.hold(&app, 0.7);

    // solo the guitar: the other tracks fade
    film.click(&app, SOLO, 0.7);
    let _ = app.update(Message::ToggleTrackSolo(TRACK));
    film.hold(&app, 0.7);

    // drag over the two measures to loop them
    let (from, to) = DRAG;
    film.glide(&app, from, 0.8);
    let _ = app.update(Message::SelectionDragStart(first, 0, from));
    film.frame(&app, true);
    for step in 1..=10 {
        film.cursor = between(from, to, step as f32 / 10.0);
        if step == 6 {
            let _ = app.update(Message::SelectionDragTo(last, film.cursor));
        }
        film.frame(&app, true);
    }
    let _ = app.update(Message::SelectionDragEnd);
    film.hold(&app, 0.6);

    // slow it down
    film.click(&app, TEMPO_PICKER, 0.8);
    let _ = app.update(Message::TempoSelected(TempoSelection::new(TEMPO)));
    film.hold(&app, 0.7);

    // play: once through the loop, and a little past its return
    film.click(&app, PLAY, 0.8);
    let _ = app.update(Message::PlayPause);
    let (start, end, bpm) = {
        let headers = &app.song.as_ref().unwrap().tablature.song.measure_headers;
        let end = headers[last].start + headers[last].length();
        (headers[first].start, end, headers[first].tempo.value)
    };
    let ticks_per_second = bpm as f32 / 60.0 * QUARTER_TIME as f32 * TEMPO as f32 / 100.0;
    let pass = (end - start) as f32 / ticks_per_second;
    let frames = frames_in(1.25 * pass);
    for frame in 0..frames {
        let elapsed = frame as f32 / FPS as f32;
        let tick = start + (elapsed * ticks_per_second) as u32 % (end - start);
        let _ = app.update(Message::FocusTick(tick));
        let measure = app.song.as_ref().unwrap().tablature.focused_measure();
        film.turn_page(&mut app, measure);
        film.cursor = between(PLAY, REST, (elapsed / 0.8).min(1.0));
        film.frame(&app, false);
        // the still: well into the loop's second measure
        if frame == frames * 3 / 5 {
            save_still(&film.render(&app), &still);
        }
    }
    if let Some(player) = &mut app.audio_player {
        player.stop();
    }
    film.finish();
}
