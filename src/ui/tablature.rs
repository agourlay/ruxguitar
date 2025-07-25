use crate::parser::song_parser::Song;
use crate::ui::application::Message;
use crate::ui::canvas_measure::CanvasMeasure;
use iced::widget::scrollable;
use iced::widget::scrollable::Id;
use iced::{Element, Length};
use iced_aw::Wrap;
use std::collections::BTreeMap;
use std::rc::Rc;

const INNER_PADDING: f32 = 10.0;

pub struct Tablature {
    pub song: Rc<Song>,
    pub track_id: usize,
    pub canvas_measures: Vec<CanvasMeasure>,
    canvas_measure_height: f32,
    focused_measure: usize,
    line_tracker: LineTracker,
    pub scroll_id: Id,
    measure_per_tick: BTreeMap<u32, u32>, // tick to measure index as u32
}

impl Tablature {
    pub fn new(song: Rc<Song>, track_id: usize, scroll_id: Id) -> Self {
        let measure_count = song.measure_headers.len();
        let mut measure_per_tick = BTreeMap::new();
        for (i, measure) in song.measure_headers.iter().enumerate() {
            measure_per_tick.insert(measure.start, i as u32);
        }
        let mut tab = Self {
            song,
            track_id,
            canvas_measures: Vec::with_capacity(measure_count),
            canvas_measure_height: 0.0,
            focused_measure: 0,
            line_tracker: LineTracker::default(),
            scroll_id,
            measure_per_tick,
        };
        tab.load_measures();
        tab
    }

    pub fn load_measures(&mut self) {
        // clear existing measures
        self.canvas_measures.clear();

        // load new measures
        let track = &self.song.tracks[self.track_id];
        let measures = track.measures.len();
        for i in 0..measures {
            let measure_header = &self.song.measure_headers[i];
            let previous_measure_header = if i > 0 {
                self.song.measure_headers.get(i - 1)
            } else {
                None
            };
            let focused = self.focused_measure == i;
            let has_time_signature = i == 0
                || measure_header.time_signature != previous_measure_header.unwrap().time_signature;
            let measure = CanvasMeasure::new(
                i,
                self.track_id,
                self.song.clone(),
                focused,
                has_time_signature,
            );
            if i == 0 {
                // all measures have the same height - grab first one
                self.canvas_measure_height = measure.vertical_measure_height;
            }
            self.canvas_measures.push(measure);
        }
        // recompute line tracker with existing width
        let existing_width = self.line_tracker.tablature_container_width;
        self.line_tracker = LineTracker::make(&self.canvas_measures, existing_width);
    }

    pub fn update_container_width(&mut self, width: f32) {
        // recompute line tracker on width change
        self.line_tracker = LineTracker::make(
            &self.canvas_measures,
            width - (INNER_PADDING * 2.0), // remove padding
        );
        // clear measure cache to draw properly the starting vertical line of measures
        // it should done only for the measures starting a row, otherwise it is overlapping with
        // the end line of the previous measure
        for cm in &self.canvas_measures {
            cm.clear_canva_cache();
        }
    }

    /// Get the measure and beat indexes for the given tick
    /// The measure index is the first measure containing the tick
    ///
    /// | measure 0 | measure 1 | measure 2 | measure 3 |
    /// |-----------|-----------|-----------|-----------|
    /// | 0         | 100       | 200       | 300       |
    ///
    ///
    /// tick: 50
    /// measure_index: 0
    ///
    /// tick: 100
    /// measure_index: 1
    ///
    /// tick: 150
    /// measure_index: 1
    ///
    /// tick: 250
    /// measure_index: 2
    ///
    /// Returns the measure and beat indexes
    pub fn get_measure_beat_indexes_for_tick(&self, track_id: usize, tick: u32) -> (usize, usize) {
        // range scan on `measure_per_tick` to find measure index for tick
        let measure_index = self
            .measure_per_tick
            .range(0..=tick) // range is growing but the cost is still O(log n) with size of the btreemap
            .next_back()
            .map(|(_event_tick, &m_id)| m_id)
            .unwrap_or_else(|| {
                log::warn!("No measure index found for tick:{tick}");
                0
            });

        // indexed as u32 to save space
        let measure_index = measure_index as usize;

        // get beat index within the measure containing the tick
        // only a few beats per measure, full scan is Ok.
        let voice = &self.song.tracks[track_id].measures[measure_index].voices[0];
        let mut beat_index = 0;
        for (j, beat) in voice.beats.iter().enumerate() {
            if beat.start > tick {
                break;
            } else {
                beat_index = j;
            }
        }
        (measure_index, beat_index)
    }

    /// Focus on the beat at the given tick
    ///
    /// Returns the amount of scroll needed to focus on the beat
    pub fn focus_on_tick(&mut self, tick: u32) -> Option<f32> {
        let (new_measure_id, new_beat_id) = if tick == 1 {
            (0, 0)
        } else {
            self.get_measure_beat_indexes_for_tick(self.track_id, tick)
        };
        let current_focus_id = self.focused_measure;
        let current_canvas = self.canvas_measures.get_mut(current_focus_id).unwrap();
        if current_focus_id == new_measure_id {
            // focus on beat id within the same measure
            current_canvas.focus_beat(new_beat_id);
        } else {
            // move to next measure
            current_canvas.toggle_focused();
            let next_focus_id = new_measure_id;
            if next_focus_id < self.canvas_measures.len() {
                self.focused_measure = next_focus_id;
                let next_canvas = self.canvas_measures.get_mut(next_focus_id).unwrap();
                next_canvas.toggle_focused();

                // compute progress of the measure within the song
                let line_tracker = &self.line_tracker;
                let focus_line = line_tracker.get_line(next_focus_id);
                let estimated_y = (focus_line - 1) as f32 * self.canvas_measure_height;
                if focus_line < 2 {
                    return None;
                }
                log::debug!("scrolling to focus_line {focus_line} estimated_y {estimated_y}");
                return Some(estimated_y);
            }
        }
        None
    }

    pub fn focus_on_measure(&mut self, new_measure_id: usize) {
        let measure_headers = &self.song.measure_headers[new_measure_id];
        let tick = measure_headers.start;
        self.focus_on_tick(tick);
    }

    pub fn view(&self) -> Element<'_, Message> {
        let measure_elements = self
            .canvas_measures
            .iter()
            .map(|m| m.view())
            .collect::<Vec<Element<Message>>>();

        let column = Wrap::with_elements(measure_elements)
            .padding(INNER_PADDING)
            .align_items(iced::Alignment::Center); // TODO does not work??

        scrollable(column)
            .id(self.scroll_id.clone())
            .height(Length::Fill)
            .width(Length::Fill)
            .direction(scrollable::Direction::default())
            .into()
    }

    pub fn update_track(&mut self, track: usize) {
        // No op if track is the same
        if track != self.track_id {
            self.track_id = track;
            self.load_measures();
        }
    }
}

#[derive(Default)]
struct LineTracker {
    measure_to_line: Vec<u32>, // measure id to line number
    tablature_container_width: f32,
}

impl LineTracker {
    pub fn make(measures: &[CanvasMeasure], tablature_container_width: f32) -> Self {
        let mut line_tracker = LineTracker {
            measure_to_line: vec![0; measures.len()],
            tablature_container_width,
        };
        let mut current_line = 1;
        let mut horizontal_cursor = 0.0;
        for measure in measures {
            horizontal_cursor += measure.total_measure_len;
            if horizontal_cursor >= tablature_container_width {
                current_line += 1;
                horizontal_cursor = measure.total_measure_len;
            }
            line_tracker.measure_to_line[measure.measure_id] = current_line;
        }
        line_tracker
    }

    pub fn get_line(&self, measure_id: usize) -> u32 {
        self.measure_to_line[measure_id]
    }
}
