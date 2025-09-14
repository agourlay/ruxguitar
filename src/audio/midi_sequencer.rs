use crate::audio::{
    midi_event::MidiEvent,
    midi_player_params::{MidiPlayerParams, Repeat},
};
use std::time::Instant;

const QUARTER_TIME: f32 = 960.0; // 1 quarter note = 960 ticks

pub struct MidiSequencer {
    current_tick: u32,             // current Midi tick
    last_tick: u32,                // last Midi tick
    last_time: Instant,            // last time in milliseconds
    sorted_events: Vec<MidiEvent>, // sorted Midi events
    sorted_repeats: Vec<Repeat>,   // sorted measure repeats by end time
}

impl MidiSequencer {
    pub fn new(sorted_events: Vec<MidiEvent>, sorted_repeats: Vec<Repeat>) -> Self {
        // events are sorted by tick
        assert!(sorted_events
            .as_slice()
            .windows(2)
            .all(|w| w[0].tick <= w[1].tick));
        assert!(sorted_repeats
            .as_slice()
            .windows(2)
            .all(|w| w[0].back_to <= w[1].back_to));
        Self {
            current_tick: 0,
            last_tick: 0,
            last_time: Instant::now(),
            sorted_events,
            sorted_repeats,
        }
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn events(&self) -> &[MidiEvent] {
        &self.sorted_events
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn set_tick(&mut self, tick: u32) {
        self.last_tick = tick;
        self.current_tick = tick;
    }

    pub fn reset_last_time(&mut self) {
        self.last_time = Instant::now();
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn reset_ticks(&mut self) {
        self.current_tick = 0;
        self.last_tick = 0;
    }

    pub const fn get_tick(&self) -> u32 {
        self.current_tick
    }

    pub const fn get_last_tick(&self) -> u32 {
        self.last_tick
    }

    pub fn get_next_events(&self) -> Option<&[MidiEvent]> {
        // do not return events if tick did not change
        if self.last_tick == self.current_tick {
            return Some(&[]);
        }

        assert!(self.last_tick <= self.current_tick);

        // get all events between last tick and next tick using binary search
        // TODO could be improved by saving `end_index` to the next `start_index`
        let start_index = match self
            .sorted_events
            .binary_search_by_key(&self.last_tick, |event| event.tick)
        {
            Ok(position) => position + 1,
            Err(position) => {
                // exit if end reached
                if position == self.sorted_events.len() {
                    return None;
                }
                position
            }
        };

        let end_index = match self.sorted_events[start_index..]
            .binary_search_by_key(&self.current_tick, |event| event.tick)
        {
            Ok(next_position) => start_index + next_position,
            Err(next_position) => {
                if next_position == 0 {
                    // no matching elements
                    return Some(&[]);
                }
                // return slice until the last event
                start_index + next_position - 1
            }
        };
        Some(&self.sorted_events[start_index..=end_index])
    }

    pub fn get_current_repeat(&self) -> Option<&Repeat> {
        let repeat_index = match self
            .sorted_repeats
            .binary_search_by_key(&self.last_tick, |repeat| repeat.back_to)
        {
            Ok(position) => position,
            Err(position) => {
                // exit if end reached
                if position == self.sorted_repeats.len() {
                    return None;
                }
                position.saturating_sub(1)
            }
        };
        let repeat = &self.sorted_repeats[repeat_index];
        if repeat.back_to <= self.last_tick && repeat.end_time >= self.last_tick {
            Some(repeat)
        } else {
            None
        }
    }

    pub fn advance(&mut self, player_param: &mut MidiPlayerParams) {
        let tempo: u32 = player_param.adjusted_tempo();
        // init sequencer if first advance after reset
        if self.current_tick == self.last_tick {
            self.current_tick += 1;
            self.last_time = Instant::now();
            return;
        }

        // check how many ticks have passed since last advance
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time);
        let elapsed_secs = elapsed.as_secs_f32();
        let tick_increase = tick_increase(tempo, elapsed_secs);
        self.last_time = now;
        self.last_tick = self.current_tick;
        self.current_tick += tick_increase;

        // check if we have an ongoing repeat sequence
        if let Some(repeat) = player_param.get_repeat().cloned() {
            log::info!(
                "repeat.end_time:{} <= self.last_tick:{}",
                repeat.end_time,
                self.last_tick
            );
            if repeat.end_time <= self.last_tick {
                // decrease remaining iter and rollback in time
                if repeat.play_count > 1 {
                    // Ideally call MidiPlayer::focus_measure to handle all the details :s
                    // Dirty hack instead
                    self.current_tick = repeat.back_to;
                    self.last_tick = self.current_tick - tick_increase;
                }
                player_param.decrease_play_count();
            }
        } else {
            // check if there is a new repeat to enable
            if let Some(new_repeat) = self.get_current_repeat().cloned() {
                player_param.set_repeat(new_repeat);
            }
        }
    }

    #[cfg(test)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn advance_tick(&mut self, tick: u32) {
        self.last_tick = self.current_tick;
        self.current_tick += tick;
    }
}

fn tick_increase(tempo_bpm: u32, elapsed_seconds: f32) -> u32 {
    let tempo_bps = tempo_bpm as f32 / 60.0;
    let bump = QUARTER_TIME * tempo_bps * elapsed_seconds;
    bump as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::midi_builder::MidiBuilder;
    use crate::parser::song_parser_tests::parse_gp_file;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn test_tick_increase() {
        let tempo = 100;
        let elapsed = Duration::from_millis(32);
        let result = tick_increase(tempo, elapsed.as_secs_f32());
        assert_eq!(result, 51);
    }

    #[test]
    fn test_tick_increase_bis() {
        let tempo = 120;
        let elapsed = Duration::from_millis(100);
        let result = tick_increase(tempo, elapsed.as_secs_f32());
        assert_eq!(result, 192);
    }
    #[test]
    fn test_sequence_demo_song() {
        const FILE_PATH: &str = "test-files/Demo v5.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let (events, repeats) = builder.build_for_song(&song);
        let events_len = 4451;
        assert_eq!(events.len(), events_len);
        assert_eq!(events[0].tick, 1);
        assert_eq!(events.iter().last().unwrap().tick, 189_120);
        let mut sequencer = MidiSequencer::new(events.clone(), repeats);

        // last_tick:0 current_tick:0
        let batch = sequencer.get_next_events().unwrap();
        assert_eq!(batch.len(), 0);

        // advance time by 1 tick
        sequencer.advance_tick(1);

        // last_tick:0 current_tick:1
        let batch = sequencer.get_next_events().unwrap();
        let count_1 = batch.len();
        assert_eq!(&events[0..count_1], batch);
        assert!(batch.iter().all(MidiEvent::is_midi_message));

        let mut pos = count_1;
        loop {
            let prev_tick = sequencer.get_tick();
            // advance time by 112 tick
            sequencer.advance_tick(112);
            let next_tick = sequencer.get_tick();
            assert_eq!(next_tick - prev_tick, 112);

            if let Some(batch) = sequencer.get_next_events() {
                let count = batch.len();
                assert_eq!(&events[pos..pos + count], batch);
                pos += count;
            } else {
                break;
            }
        }
        assert_eq!(pos, events.len());
    }
}
