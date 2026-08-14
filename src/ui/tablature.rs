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
    viewport_height: f32,   // visible height of the tablature container
    page_top_line: u32,     // line currently shown at the top of the view
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
            viewport_height: 0.0,
            page_top_line: 1,
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

    pub fn update_container_size(&mut self, width: f32, height: f32) {
        self.viewport_height = height;
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
            // a tick before the first measure means playback has not reached
            // it yet: the cursor belongs on that first measure
            .or_else(|| self.measure_per_tick.iter().next())
            .map(|(&event_tick, &m_id)| (event_tick, m_id as usize))
            .unwrap_or_else(|| {
                log::warn!("No measure index found for tick:{tick}");
                (0, 0)
            });

        // compute tick offset between playback position and original measure position
        let original_start = self.song.measure_headers[measure_index].start;
        let tick_offset = i64::from(playback_start) - i64::from(original_start);
        let original_tick = (i64::from(tick) - tick_offset).max(i64::from(original_start)) as u32;
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
    /// is driven separately by [`Self::page_scroll_offset`].
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

    /// Scroll needed to bring `measure_id` into view, a page at a time: the
    /// view holds still while the measure is on the page being read, then
    /// turns to the page starting on its line.
    pub fn page_scroll_offset(&mut self, measure_id: usize) -> Option<f32> {
        let line = self.line_tracker.get_line(measure_id);
        let last_visible = self.page_top_line + self.visible_lines(self.page_top_line);
        if line >= self.page_top_line && line < last_visible {
            return None;
        }
        self.page_top_line = line;
        Some(self.offset_for_line_top(line))
    }

    /// Scroll offset putting `line` at the top of the view.
    fn offset_for_line_top(&self, line: u32) -> f32 {
        if line <= 1 {
            return 0.0;
        }
        let lines_above = (line - 1) as usize;
        INNER_PADDING + self.line_heights.iter().take(lines_above).sum::<f32>()
    }

    /// How many lines fit in the view when `from_line` is at the top. Always
    /// at least one, so a line taller than the view still turns the page.
    fn visible_lines(&self, from_line: u32) -> u32 {
        let mut used = 0.0;
        let mut count = 0;
        for height in self.line_heights.iter().skip((from_line - 1) as usize) {
            if used + height > self.viewport_height {
                break;
            }
            used += height;
            count += 1;
        }
        count.max(1)
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

    fn load_tablature(width: f32, height: f32) -> Tablature {
        let song = Rc::new(parse_gp_file("test-files/Demo v5.gp5").unwrap());
        let order = compute_playback_order(&song.measure_headers);
        let mut tab = Tablature::new(song, 0, Id::new("test-scroll"), &order);
        tab.update_container_size(width, height);
        tab
    }

    #[test]
    fn measures_of_a_line_share_their_height() {
        let tab = load_tablature(800.0, 400.0);
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
    fn page_holds_still_while_being_read() {
        let mut tab = load_tablature(800.0, 400.0);
        let visible = tab.visible_lines(1);
        assert!(visible > 1, "the view should hold several lines");
        // every measure of the opening page leaves the view alone
        for cm_id in 0..tab.measure_count() {
            if tab.line_tracker.get_line(cm_id) > visible {
                break;
            }
            assert_eq!(
                tab.page_scroll_offset(cm_id),
                None,
                "measure {cm_id} should not move the page"
            );
        }
    }

    #[test]
    fn page_turns_when_playback_leaves_it() {
        let mut tab = load_tablature(800.0, 400.0);
        let visible = tab.visible_lines(1);
        // the first measure past the page turns it, and lands on top
        let first_off_page = (0..tab.measure_count())
            .find(|&id| tab.line_tracker.get_line(id) > visible)
            .expect("a measure past the first page");
        let line = tab.line_tracker.get_line(first_off_page);
        let offset = tab
            .page_scroll_offset(first_off_page)
            .expect("the page should turn");
        assert!((offset - tab.offset_for_line_top(line)).abs() < f32::EPSILON);

        // the new page then holds still in turn
        assert_eq!(tab.page_scroll_offset(first_off_page), None);
    }

    #[test]
    fn page_turns_back_when_seeking_backwards() {
        let mut tab = load_tablature(800.0, 400.0);
        let last = tab.measure_count() - 1;
        tab.page_scroll_offset(last).expect("the page should turn");
        // seeking back to the start turns the page back to the top
        assert_eq!(tab.page_scroll_offset(0), Some(0.0));
    }

    #[test]
    fn a_line_taller_than_the_view_still_turns_the_page() {
        let mut tab = load_tablature(800.0, 1.0);
        // one line at a time, but never zero: playback must keep advancing
        assert_eq!(tab.visible_lines(1), 1);
        let next_line_measure = (0..tab.measure_count())
            .find(|&id| tab.line_tracker.get_line(id) == 2)
            .expect("a second line");
        assert!(tab.page_scroll_offset(next_line_measure).is_some());
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
