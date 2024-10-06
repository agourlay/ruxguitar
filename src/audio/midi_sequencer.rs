use crate::audio::midi_event::MidiEvent;
use std::time::Instant;

const QUARTER_TIME: i32 = 960; // 1 quarter note = 960 ticks

pub struct MidiSequencer {
    current_tick: usize,           // current Midi tick
    last_tick: usize,              // last Midi tick
    last_time: Instant,            // last time in milliseconds
    sorted_events: Vec<MidiEvent>, // sorted Midi events
}

impl MidiSequencer {
    pub fn new(sorted_events: Vec<MidiEvent>) -> Self {
        // events are sorted by tick
        assert!(sorted_events
            .as_slice()
            .windows(2)
            .all(|w| w[0].tick <= w[1].tick));
        Self {
            current_tick: 0,
            last_tick: 0,
            last_time: Instant::now(),
            sorted_events,
        }
    }

    pub fn events(&self) -> &[MidiEvent] {
        &self.sorted_events
    }

    pub fn set_tick(&mut self, tick: usize) {
        self.last_tick = tick;
        self.current_tick = tick;
    }

    pub fn reset_last_time(&mut self) {
        self.last_time = Instant::now();
    }

    pub fn reset_ticks(&mut self) {
        self.current_tick = 0;
        self.last_tick = 0;
    }

    pub fn get_tick(&self) -> usize {
        self.current_tick
    }

    pub fn get_last_tick(&self) -> usize {
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

    pub fn advance(&mut self, tempo: i32) {
        // init sequencer if first advance after reset
        if self.current_tick == self.last_tick {
            self.current_tick += 1;
            self.last_time = Instant::now();
            return;
        }
        // check how many ticks have passed since last advance
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time);
        let elapsed_secs = elapsed.as_secs_f64();
        let tick_increase = tick_increase(tempo, elapsed_secs);
        self.last_time = now;
        self.last_tick = self.current_tick;
        self.current_tick += tick_increase;
    }

    #[cfg(test)]
    pub fn advance_tick(&mut self, tick: usize) {
        self.last_tick = self.current_tick;
        self.current_tick += tick;
    }
}

fn tick_increase(tempo_bpm: i32, elapsed_seconds: f64) -> usize {
    let tempo_bps = tempo_bpm as f64 / 60.0;
    let bump = QUARTER_TIME as f64 * tempo_bps * elapsed_seconds;
    bump as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::midi_builder::MidiBuilder;
    use crate::parser::song_parser_tests::parse_gp_file;
    use std::collections::HashSet;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn test_tick_increase() {
        let tempo = 100;
        let elapsed = Duration::from_millis(32);
        let result = tick_increase(tempo, elapsed.as_secs_f64());
        assert_eq!(result, 51);
    }

    #[test]
    fn test_tick_increase_bis() {
        let tempo = 120;
        let elapsed = Duration::from_millis(100);
        let result = tick_increase(tempo, elapsed.as_secs_f64());
        assert_eq!(result, 192);
    }
    #[test]
    fn test_sequence_song() {
        const FILE_PATH: &str = "test-files/Demo v5.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);
        let events_len = 4431;
        assert_eq!(events.len(), events_len);
        assert_eq!(events[0].tick, 1);
        assert_eq!(events.iter().last().unwrap().tick, 189_120);
        let mut sequencer = MidiSequencer::new(events.clone());

        // keep track of elements seen to detect duplicates
        let mut seen = HashSet::new();

        // last_tick:0 current_tick:0
        let batch = sequencer.get_next_events().unwrap();
        assert_eq!(batch.len(), 0);

        // advance time by 1 tick
        sequencer.advance_tick(1);

        // last_tick:0 current_tick:1
        let batch = sequencer.get_next_events().unwrap();
        // assert no duplicates
        for b in batch {
            assert!(seen.insert(b.clone()), "duplicate event {:?}", b);
        }
        let count_1 = batch.len();
        assert_eq!(&events[0..count_1], batch);
        assert!(batch.iter().all(|e| e.is_midi_message()));

        let mut pos = count_1;
        loop {
            let prev_tick = sequencer.get_tick();
            // advance time by 112 tick
            sequencer.advance_tick(112);
            let next_tick = sequencer.get_tick();
            assert_eq!(next_tick - prev_tick, 112);

            if let Some(batch) = sequencer.get_next_events() {
                // assert no duplicates
                for (id, b) in batch.iter().enumerate() {
                    assert!(seen.insert(b.clone()), "duplicate event at {} {:?}", id, b);
                }

                let count = batch.len();
                assert_eq!(&events[pos..pos + count], batch);
                pos += count;
            } else {
                break;
            }
        }
        assert_eq!(pos, events.len());
        assert_eq!(seen.len(), events_len);
    }
}
