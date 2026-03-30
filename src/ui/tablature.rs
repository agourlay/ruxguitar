use crate::parser::song_parser::Song;
use crate::ui::application::Message;
use crate::ui::canvas_measure::CanvasMeasure;
use iced::widget::{Id, Row, column, scrollable};
use iced::{Element, Length};
use std::collections::BTreeMap;
use std::rc::Rc;

const INNER_PADDING: f32 = 10.0;
const SCROLLBAR_WIDTH: f32 = 10.0; // iced default scrollbar width (iced_widget/src/scrollable.rs)

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
    pub fn new(
        song: Rc<Song>,
        track_id: usize,
        scroll_id: Id,
        playback_order: &[(usize, i64)],
    ) -> Self {
        let measure_count = song.measure_headers.len();
        // build tick-to-measure map including expanded repeat ticks
        let mut measure_per_tick = BTreeMap::new();
        for (measure_index, tick_offset) in playback_order {
            let header = &song.measure_headers[*measure_index];
            let playback_tick = (i64::from(header.start) + tick_offset) as u32;
            measure_per_tick.insert(playback_tick, *measure_index as u32);
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
        self.update_first_on_line();
    }

    pub fn update_container_width(&mut self, width: f32) {
        // recompute line tracker on width change
        self.line_tracker = LineTracker::make(
            &self.canvas_measures,
            width - (INNER_PADDING * 2.0) - SCROLLBAR_WIDTH, // remove padding and scrollbar
        );
        // mark which measures start a new line and clear caches
        self.update_first_on_line();
    }

    /// Update the `is_first_on_line` flag on each measure based on the line tracker
    /// and clear caches for measures that changed line assignment.
    fn update_first_on_line(&mut self) {
        let mut prev_line = 0_u32;
        for cm in &mut self.canvas_measures {
            let line = self.line_tracker.get_line(cm.measure_id);
            let is_first = line != prev_line;
            if cm.is_first_on_line != is_first {
                cm.set_first_on_line(is_first);
                cm.clear_canvas_cache();
            }
            prev_line = line;
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
        // range scan on `measure_per_tick` to find measure index and playback start tick
        let (playback_start, measure_index) = self
            .measure_per_tick
            .range(0..=tick)
            .next_back()
            .map(|(&event_tick, &m_id)| (event_tick, m_id as usize))
            .unwrap_or_else(|| {
                log::warn!("No measure index found for tick:{tick}");
                (0, 0)
            });

        // compute tick offset between playback position and original measure position
        let original_start = self.song.measure_headers[measure_index].start;
        let tick_offset = i64::from(playback_start) - i64::from(original_start);

        // get beat index within the measure containing the tick
        // adjust tick by removing the offset to compare with original beat.start values
        let original_tick = (i64::from(tick) - tick_offset) as u32;
        let voice = &self.song.tracks[track_id].measures[measure_index].voices[0];
        let mut beat_index = 0;
        for (j, beat) in voice.beats.iter().enumerate() {
            if beat.start > original_tick {
                break;
            }
            beat_index = j;
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
                // scroll so the previous line is still visible above
                let scroll_line = focus_line.saturating_sub(2);
                let estimated_y = INNER_PADDING + scroll_line as f32 * self.canvas_measure_height;
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
        let has_layout = self.line_tracker.tablature_container_width > 0.0;

        let content: Element<Message> = if has_layout {
            // Build explicit rows using LineTracker line assignments.
            // Each measure uses FillPortion to stretch and fill the row width.
            let row_width = self.line_tracker.tablature_container_width;
            let mut rows: Vec<Element<Message>> = Vec::new();
            let mut current_row: Vec<Element<Message>> = Vec::new();
            let mut current_line = 0_u32;

            for cm in &self.canvas_measures {
                let line = self.line_tracker.get_line(cm.measure_id);
                if line != current_line && !current_row.is_empty() {
                    rows.push(
                        Row::with_children(std::mem::take(&mut current_row))
                            .width(row_width)
                            .into(),
                    );
                }
                current_line = line;
                current_row.push(cm.view_fill());
            }
            if !current_row.is_empty() {
                rows.push(Row::with_children(current_row).width(row_width).into());
            }

            column(rows).padding(INNER_PADDING).into()
        } else {
            // Before container size is known, use wrapping layout with natural widths
            let measure_elements = self
                .canvas_measures
                .iter()
                .map(|m| m.view())
                .collect::<Vec<Element<Message>>>();

            column![Row::with_children(measure_elements).wrap()]
                .padding(INNER_PADDING)
                .into()
        };

        scrollable(content)
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
        let widths: Vec<f32> = measures.iter().map(|m| m.total_measure_len).collect();
        Self::make_from_widths(&widths, tablature_container_width)
    }

    fn make_from_widths(widths: &[f32], tablature_container_width: f32) -> Self {
        let mut line_tracker = Self {
            measure_to_line: vec![0; widths.len()],
            tablature_container_width,
        };
        let mut current_line = 1;
        let mut horizontal_cursor = 0.0;
        for (i, &width) in widths.iter().enumerate() {
            horizontal_cursor += width;
            if horizontal_cursor > tablature_container_width {
                current_line += 1;
                horizontal_cursor = width;
            }
            line_tracker.measure_to_line[i] = current_line;
        }
        line_tracker
    }

    pub fn get_line(&self, measure_id: usize) -> u32 {
        self.measure_to_line[measure_id]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_tracker_single_line() {
        let widths = vec![100.0, 100.0, 100.0];
        let tracker = LineTracker::make_from_widths(&widths, 500.0);
        assert_eq!(tracker.get_line(0), 1);
        assert_eq!(tracker.get_line(1), 1);
        assert_eq!(tracker.get_line(2), 1);
    }

    #[test]
    fn line_tracker_wraps_to_multiple_lines() {
        let widths = vec![100.0, 100.0, 100.0, 100.0];
        let tracker = LineTracker::make_from_widths(&widths, 250.0);
        // first two fit (200 < 250), third overflows (300 >= 250)
        assert_eq!(tracker.get_line(0), 1);
        assert_eq!(tracker.get_line(1), 1);
        assert_eq!(tracker.get_line(2), 2);
        assert_eq!(tracker.get_line(3), 2);
    }

    #[test]
    fn line_tracker_exact_fit_stays() {
        // measures that exactly fill the width should stay on the same line
        let widths = vec![100.0, 100.0, 100.0];
        let tracker = LineTracker::make_from_widths(&widths, 200.0);
        assert_eq!(tracker.get_line(0), 1);
        assert_eq!(tracker.get_line(1), 1); // 200 == 200, fits exactly
        assert_eq!(tracker.get_line(2), 2); // 300 > 200, wraps
    }

    #[test]
    fn line_tracker_single_wide_measure() {
        // a measure wider than the container gets its own line
        let widths = vec![50.0, 300.0, 50.0];
        let tracker = LineTracker::make_from_widths(&widths, 200.0);
        assert_eq!(tracker.get_line(0), 1);
        assert_eq!(tracker.get_line(1), 2);
        assert_eq!(tracker.get_line(2), 3);
    }

    #[test]
    fn line_tracker_varying_widths() {
        let widths = vec![80.0, 60.0, 90.0, 70.0, 50.0];
        let tracker = LineTracker::make_from_widths(&widths, 200.0);
        // line 1: 80 + 60 = 140 < 200
        // line 1: 140 + 90 = 230 >= 200 → wrap
        // line 2: 90 + 70 = 160 < 200
        // line 2: 160 + 50 = 210 >= 200 → wrap
        assert_eq!(tracker.get_line(0), 1);
        assert_eq!(tracker.get_line(1), 1);
        assert_eq!(tracker.get_line(2), 2);
        assert_eq!(tracker.get_line(3), 2);
        assert_eq!(tracker.get_line(4), 3);
    }

    #[test]
    fn line_tracker_empty() {
        let widths: Vec<f32> = vec![];
        let tracker = LineTracker::make_from_widths(&widths, 500.0);
        assert_eq!(tracker.measure_to_line.len(), 0);
    }

    #[test]
    fn first_on_line_detection() {
        let widths = vec![100.0, 100.0, 100.0, 100.0];
        let tracker = LineTracker::make_from_widths(&widths, 250.0);
        // lines: [1, 1, 2, 2]
        let mut prev_line = 0_u32;
        let mut first_on_line = Vec::new();
        for i in 0..widths.len() {
            let line = tracker.get_line(i);
            first_on_line.push(line != prev_line);
            prev_line = line;
        }
        assert_eq!(first_on_line, vec![true, false, true, false]);
    }
}
