use crate::audio::playback_order::playback_tick;
use crate::parser::song_parser::Song;
use crate::ui::application::Message;
use crate::ui::canvas_measure::{CanvasMeasure, RowSpacing};
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
    line_heights: Vec<f32>, // rendered height of each line, in order
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
            let tick = playback_tick(header.start, *tick_offset);
            measure_per_tick.insert(tick, *measure_index as u32);
        }
        let mut tab = Self {
            song,
            track_id,
            canvas_measures: Vec::with_capacity(measure_count),
            line_heights: Vec::new(),
            focused_measure: 0,
            line_tracker: LineTracker::default(),
            scroll_id,
            measure_per_tick,
        };
        tab.load_measures();
        tab
    }

    pub fn load_measures(&mut self) {
        self.canvas_measures.clear();

        let measures = self.song.tracks[self.track_id].measures.len();
        for i in 0..measures {
            // the first measure always shows its time signature, the others
            // only when it changed
            let has_time_signature = i.checked_sub(1).is_none_or(|previous| {
                self.song.measure_headers[i].time_signature
                    != self.song.measure_headers[previous].time_signature
            });
            let measure = CanvasMeasure::new(
                i,
                self.track_id,
                self.song.clone(),
                self.focused_measure == i,
                has_time_signature,
            );
            self.canvas_measures.push(measure);
        }
        // recompute line tracker with existing width
        let existing_width = self.line_tracker.tablature_container_width;
        self.line_tracker = LineTracker::make(&self.canvas_measures, existing_width);
        self.update_first_on_line();
        self.update_line_spacing();
    }

    pub fn update_container_width(&mut self, width: f32) {
        // recompute line tracker on width change
        self.line_tracker = LineTracker::make(
            &self.canvas_measures,
            width - (INNER_PADDING * 2.0) - SCROLLBAR_WIDTH, // remove padding and scrollbar
        );
        // mark which measures start a new line and clear caches
        self.update_first_on_line();
        self.update_line_spacing();
    }

    /// Give every measure of a line the same annotation rows, sized for the
    /// most demanding measure on it, so their staves stay aligned.
    fn update_line_spacing(&mut self) {
        let Some(line_count) = self.line_tracker.line_count() else {
            self.line_heights.clear();
            return;
        };
        let mut per_line = vec![RowSpacing::default(); line_count];
        for cm in &self.canvas_measures {
            let line = self.line_tracker.get_line(cm.measure_id) as usize - 1;
            per_line[line].merge(cm.row_needs());
        }
        for cm in &mut self.canvas_measures {
            let line = self.line_tracker.get_line(cm.measure_id) as usize - 1;
            cm.set_row_spacing(per_line[line]);
        }
        // line heights drive the playback scroll offset
        self.line_heights = vec![0.0; line_count];
        for cm in &self.canvas_measures {
            let line = self.line_tracker.get_line(cm.measure_id) as usize - 1;
            self.line_heights[line] = self.line_heights[line].max(cm.vertical_measure_height);
        }
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

    /// The measure containing `tick`, with the tick mapped back onto the
    /// measure's own timeline (playback repeats shift it forward).
    fn measure_and_original_tick(&self, tick: u32) -> (usize, u32) {
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
        let original_tick = (i64::from(tick) - tick_offset) as u32;
        (measure_index, original_tick)
    }

    /// The measure and beat containing `tick`, i.e. the last ones starting at
    /// or before it.
    pub fn get_measure_beat_indexes_for_tick(&self, track_id: usize, tick: u32) -> (usize, usize) {
        let (measure_index, original_tick) = self.measure_and_original_tick(tick);

        // get beat index within the measure containing the tick
        // adjust tick by removing the offset to compare with original beat.start values
        let voice = &self.song.tracks[track_id].measures[measure_index].voices[0];
        let beat_index = voice
            .beats
            .partition_point(|beat| beat.start <= original_tick)
            .saturating_sub(1);
        (measure_index, beat_index)
    }

    /// Move the highlight to the beat at the given playback tick. Scrolling
    /// is driven separately by [`Self::playback_scroll_offset`].
    pub fn focus_on_tick(&mut self, tick: u32) {
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
                // beat notifications coalesce, so the first tick in a measure
                // may already be past beat 0
                next_canvas.focus_beat(new_beat_id);
            }
        }
    }

    pub fn focus_on_measure(&mut self, new_measure_id: usize) {
        self.focus_on_measure_beat(new_measure_id, 0);
    }

    pub fn focus_on_measure_beat(&mut self, new_measure_id: usize, beat_id: usize) {
        let current_focus_id = self.focused_measure;
        if current_focus_id != new_measure_id {
            let current_canvas = self.canvas_measures.get_mut(current_focus_id).unwrap();
            current_canvas.toggle_focused();
            self.focused_measure = new_measure_id;
            let next_canvas = self.canvas_measures.get_mut(new_measure_id).unwrap();
            next_canvas.toggle_focused();
        }
        let canvas = self.canvas_measures.get_mut(new_measure_id).unwrap();
        canvas.focus_beat(beat_id);
    }

    /// Tick offset of a beat from the start of its measure.
    pub fn beat_tick_offset(&self, measure_id: usize, beat_id: usize) -> u32 {
        let measure_start = self.song.measure_headers[measure_id].start;
        self.song.tracks[self.track_id].measures[measure_id].voices[0]
            .beats
            .get(beat_id)
            .map_or(0, |beat| beat.start.saturating_sub(measure_start))
    }

    pub const fn focused_measure(&self) -> usize {
        self.focused_measure
    }

    pub const fn measure_count(&self) -> usize {
        self.canvas_measures.len()
    }

    /// Scroll offset following the playback position continuously: the view
    /// glides across a line as the cursor crosses it, instead of jumping
    /// when the line changes.
    pub fn playback_scroll_offset(&self, tick: u32) -> f32 {
        let (measure_id, original_tick) = self.measure_and_original_tick(tick);
        let header = &self.song.measure_headers[measure_id];
        let measure_length = header.length();
        let progress_in_measure = if measure_length == 0 {
            0.0
        } else {
            (original_tick.saturating_sub(header.start) as f32 / measure_length as f32).clamp(0.0, 1.0)
        };

        let line = self.line_tracker.get_line(measure_id);
        let (first_measure, measure_count) = self.line_tracker.line_span(line);
        let progress_in_line = if measure_count == 0 {
            0.0
        } else {
            let index_in_line = measure_id.saturating_sub(first_measure) as f32;
            (index_in_line + progress_in_measure) / measure_count as f32
        };

        let from = self.offset_for_line(line);
        let to = self.offset_for_line(line + 1);
        from + progress_in_line * (to - from)
    }

    /// Scroll offset that puts `line` in the second visible slot.
    fn offset_for_line(&self, line: u32) -> f32 {
        if line < 2 {
            return 0.0;
        }
        let scroll_lines = (line - 2) as usize;
        INNER_PADDING + self.line_heights.iter().take(scroll_lines).sum::<f32>()
    }

    pub fn scroll_offset_for_measure(&self, measure_id: usize) -> Option<f32> {
        let focus_line = self.line_tracker.get_line(measure_id);
        if focus_line < 2 {
            return None;
        }
        Some(self.offset_for_line(focus_line))
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

    /// First measure of a line and how many measures it holds.
    fn line_span(&self, line: u32) -> (usize, usize) {
        let first = self.measure_to_line.partition_point(|&l| l < line);
        let end = self.measure_to_line.partition_point(|&l| l <= line);
        (first, end - first)
    }

    /// Number of lines, or `None` when there is no measure at all.
    fn line_count(&self) -> Option<usize> {
        self.measure_to_line.last().map(|&last| last as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::playback_order::compute_playback_order;
    use crate::parser::song_parser_tests::parse_gp_file;

    fn load_tablature(width: f32) -> Tablature {
        let song = Rc::new(parse_gp_file("test-files/Demo v5.gp5").unwrap());
        let order = compute_playback_order(&song.measure_headers);
        let mut tab = Tablature::new(song, 0, Id::new("test-scroll"), &order);
        tab.update_container_width(width);
        tab
    }

    #[test]
    fn measures_of_a_line_share_their_height() {
        let tab = load_tablature(800.0);
        // staves must align within a line, so every measure on it keeps the
        // same annotation rows
        let mut per_line: BTreeMap<u32, f32> = BTreeMap::new();
        for cm in &tab.canvas_measures {
            let line = tab.line_tracker.get_line(cm.measure_id);
            let height = *per_line.entry(line).or_insert(cm.vertical_measure_height);
            assert!(
                (height - cm.vertical_measure_height).abs() < f32::EPSILON,
                "measure {} breaks the height of line {line}",
                cm.measure_id
            );
        }
        // and the rows are allocated per line, not globally
        let distinct: Vec<f32> = {
            let mut heights: Vec<f32> = per_line.values().copied().collect();
            heights.sort_by(f32::total_cmp);
            heights.dedup();
            heights
        };
        assert!(
            distinct.len() > 1,
            "expected lines of different heights, got {distinct:?}"
        );
    }

    #[test]
    fn playback_scroll_glides_across_a_line() {
        let tab = load_tablature(800.0);
        // a line holding several measures, so there is a glide to observe
        let line = (1..)
            .find(|&l| tab.line_tracker.line_span(l).1 > 1 && tab.offset_for_line(l + 1) > 0.0)
            .expect("a line with several measures");
        let (first, count) = tab.line_tracker.line_span(line);
        let from = tab.offset_for_line(line);
        let to = tab.offset_for_line(line + 1);
        assert!(to > from, "the line should scroll: {from} -> {to}");

        // the glide starts exactly where the line starts
        let start_tick = tab.song.measure_headers[first].start;
        assert!((tab.playback_scroll_offset(start_tick) - from).abs() < 0.01);

        // and advances without going backwards or overshooting the next line
        let mut previous = f32::NEG_INFINITY;
        for i in 0..count {
            let header = &tab.song.measure_headers[first + i];
            for numerator in 0..4 {
                let tick = header.start + header.length() * numerator / 4;
                let offset = tab.playback_scroll_offset(tick);
                assert!(offset >= previous, "scroll went backwards at tick {tick}");
                assert!(
                    (from - 0.01..=to + 0.01).contains(&offset),
                    "offset {offset} outside {from}..{to}"
                );
                previous = offset;
            }
        }
    }

    #[test]
    fn playback_scroll_stays_within_its_line() {
        let tab = load_tablature(800.0);
        // walking the real playback order: while a measure plays, the scroll
        // stays inside the band between its line and the next
        for (&start_tick, &measure) in &tab.measure_per_tick {
            let measure = measure as usize;
            let line = tab.line_tracker.get_line(measure);
            let from = tab.offset_for_line(line);
            let to = tab.offset_for_line(line + 1);
            let length = tab.song.measure_headers[measure].length();
            let mut previous = f32::NEG_INFINITY;
            for numerator in 0..4 {
                let offset = tab.playback_scroll_offset(start_tick + length * numerator / 4);
                assert!(
                    (from - 0.01..=to + 0.01).contains(&offset),
                    "measure {measure} on line {line}: {offset} outside {from}..{to}"
                );
                assert!(offset >= previous, "scroll went backwards in measure {measure}");
                previous = offset;
            }
        }
    }

    #[test]
    fn playback_scroll_has_no_jump_between_lines() {
        let tab = load_tablature(800.0);
        // where playback moves forward onto the next line, the glide must
        // already have arrived: no jump left to make
        let entries: Vec<(u32, usize)> = tab
            .measure_per_tick
            .iter()
            .map(|(&tick, &m)| (tick, m as usize))
            .collect();
        let mut checked = 0;
        for pair in entries.windows(2) {
            let [(tick, measure), (_, next_measure)] = pair else {
                continue;
            };
            let line = tab.line_tracker.get_line(*measure);
            let next_line = tab.line_tracker.get_line(*next_measure);
            // skip repeat jumps: those legitimately move the view back
            if *next_measure != measure + 1 || next_line == line {
                continue;
            }
            let length = tab.song.measure_headers[*measure].length();
            let at_line_end = tab.playback_scroll_offset(tick + length - 1);
            let next_line_start = tab.offset_for_line(next_line);
            assert!(
                (at_line_end - next_line_start).abs() < 2.0,
                "line {line} ends at {at_line_end}, line {next_line} starts at {next_line_start}"
            );
            checked += 1;
        }
        assert!(checked > 0, "no forward line change found to check");
    }

    #[test]
    fn line_heights_drive_the_scroll_offset() {
        let tab = load_tablature(800.0);
        // the first two lines stay in view
        assert_eq!(tab.scroll_offset_for_measure(0), None);
        // deeper lines scroll by the summed height of the lines above
        let deep = tab
            .canvas_measures
            .iter()
            .rev()
            .find(|cm| tab.line_tracker.get_line(cm.measure_id) > 2)
            .expect("a measure past the second line");
        let line = tab.line_tracker.get_line(deep.measure_id);
        let expected: f32 = tab
            .line_heights
            .iter()
            .take(line as usize - 2)
            .sum::<f32>()
            + INNER_PADDING;
        let offset = tab.scroll_offset_for_measure(deep.measure_id).unwrap();
        assert!((offset - expected).abs() < f32::EPSILON);
    }

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
