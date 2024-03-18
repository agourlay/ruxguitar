use crate::parser::song_parser::Song;
use crate::ui::application::Message;
use crate::ui::canvas_measure::CanvasMeasure;
use crate::ui::iced_aw::wrap::Wrap;
use iced::widget::scrollable;
use iced::widget::scrollable::Id;
use iced::{Element, Length};
use std::rc::Rc;

pub struct Tablature {
    pub song: Rc<Song>,
    pub track_id: usize,
    pub canvas_measures: Vec<CanvasMeasure>,
    pub focuses_measure: usize,
    pub scroll_id: Id,
}

impl Tablature {
    pub fn new(song: Rc<Song>, track_id: usize, scroll_id: Id) -> Self {
        let measure_count = song.measure_headers.len();
        let mut tab = Self {
            song,
            track_id,
            canvas_measures: Vec::with_capacity(measure_count),
            focuses_measure: 0,
            scroll_id,
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
            let focused = self.focuses_measure == i;
            let measure = CanvasMeasure::new(i, self.track_id, self.song.clone(), focused);
            self.canvas_measures.push(measure);
        }
    }

    pub fn focus_on_tick(&mut self, tick: usize) {
        // TODO autoscroll if necessary
        let (new_measure_id, new_beat_id) =
            self.song.get_measure_beat_for_tick(self.track_id, tick);
        let current_focus_id = self.focuses_measure;
        let current_canvas = self.canvas_measures.get_mut(current_focus_id).unwrap();
        if current_focus_id != new_measure_id {
            // move to next measure
            current_canvas.toggle_focused();
            let next_focus_id = new_measure_id;
            if next_focus_id < self.canvas_measures.len() {
                self.focuses_measure = next_focus_id;
                let next_canvas = self.canvas_measures.get_mut(next_focus_id).unwrap();
                next_canvas.toggle_focused();
            }
        } else {
            // focus on beat id
            current_canvas.focus_beat(new_beat_id);
        }
    }

    pub fn focus_on_measure(&mut self, new_measure_id: usize) {
        let measure_headers = &self.song.measure_headers[new_measure_id];
        let tick = measure_headers.start;
        self.focus_on_tick(tick as usize);
    }

    pub fn view(&self) -> Element<Message> {
        let measure_elements = self
            .canvas_measures
            .iter()
            .map(|m| m.view())
            .collect::<Vec<Element<Message>>>();

        let column = Wrap::with_elements(measure_elements)
            .padding(10.0)
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
