use crate::audio::midi_event::MidiEvent;
use crate::parser::song_parser::QUARTER_TIME;
use std::time::Instant;

pub struct MidiSequencer {
    last_tick: u32,                // last Midi tick
    tick_position: f64,            // exact tick position; the current tick is its integer part
    needs_init: bool,              // true until the first advance after a reset or seek
    last_time: Instant,            // last time in milliseconds
    sorted_events: Vec<MidiEvent>, // sorted Midi events
}

impl MidiSequencer {
    pub fn new(sorted_events: Vec<MidiEvent>) -> Self {
        // events are sorted by tick
        assert!(
            sorted_events
                .as_slice()
                .windows(2)
                .all(|w| w[0].tick <= w[1].tick)
        );
        Self {
            last_tick: 0,
            tick_position: 0.0,
            needs_init: true,
            last_time: Instant::now(),
            sorted_events,
        }
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn events(&self) -> &[MidiEvent] {
        &self.sorted_events
    }

    pub fn set_tick(&mut self, tick: u32) {
        // set last_tick before the target so get_next_events includes events at target tick
        // mark for init so the next advance() bumps by 1 instead of using a stale clock
        let adjusted = tick.saturating_sub(1);
        self.last_tick = adjusted;
        self.tick_position = f64::from(adjusted);
        self.needs_init = true;
    }

    pub fn reset_last_time(&mut self) {
        self.last_time = Instant::now();
    }

    pub fn reset_ticks(&mut self) {
        self.set_tick(0);
    }

    pub const fn get_tick(&self) -> u32 {
        self.tick_position as u32
    }

    pub const fn get_last_tick(&self) -> u32 {
        self.last_tick
    }

    pub fn get_next_events(&self) -> Option<&[MidiEvent]> {
        let current_tick = self.get_tick();
        // do not return events if tick did not change
        if self.last_tick == current_tick {
            return Some(&[]);
        }

        assert!(self.last_tick <= current_tick);

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
            .binary_search_by_key(&current_tick, |event| event.tick)
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

    pub fn advance(&mut self, tempo: u32) {
        // init sequencer if first advance after a reset or seek
        if self.needs_init {
            self.needs_init = false;
            // tick_position is integral here (set from a u32), so the bump is exact
            self.tick_position += 1.0;
            self.last_time = Instant::now();
            return;
        }
        // check how many ticks have passed since last advance
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time);
        self.last_time = now;
        self.advance_by(tempo, elapsed.as_secs_f64());
    }

    fn advance_by(&mut self, tempo: u32, elapsed_secs: f64) {
        self.last_tick = self.get_tick();
        self.tick_position += tick_increase(tempo, elapsed_secs);
    }

    #[cfg(test)]
    pub fn advance_tick(&mut self, tick: u32) {
        self.needs_init = false;
        self.last_tick = self.get_tick();
        self.tick_position += f64::from(tick);
    }
}

fn tick_increase(tempo_bpm: u32, elapsed_seconds: f64) -> f64 {
    let tempo_bps = f64::from(tempo_bpm) / 60.0;
    f64::from(QUARTER_TIME) * tempo_bps * elapsed_seconds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::midi_builder::MidiBuilder;
    use crate::audio::midi_event::MidiEventType;
    use crate::parser::song_parser_tests::parse_gp_file;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn test_tick_increase() {
        let tempo = 100;
        let elapsed = Duration::from_millis(32);
        let result = tick_increase(tempo, elapsed.as_secs_f64());
        assert!((result - 51.2).abs() < 1e-9);
    }

    #[test]
    fn test_tick_increase_bis() {
        let tempo = 120;
        let elapsed = Duration::from_millis(100);
        let result = tick_increase(tempo, elapsed.as_secs_f64());
        assert!((result - 192.0).abs() < 1e-9);
    }

    #[test]
    fn fractional_ticks_accumulate_across_advances() {
        let mut sequencer = MidiSequencer::new(vec![]);
        // first advance after reset bumps to tick 1
        sequencer.advance(120);
        assert_eq!(sequencer.get_tick(), 1);

        // simulate 1000 audio callbacks of 5.8 ms each at 120 BPM
        // (256 frames at 44.1 kHz), i.e. 11.136 ticks per callback
        for _ in 0..1000 {
            sequencer.advance_by(120, 0.0058);
        }

        // exact total: 1 + 11.136 * 1000 = 11137 ticks
        // truncating per callback would yield 11001 (~1.2% slow)
        let expected = 1 + 11_136;
        assert!((i64::from(sequencer.get_tick()) - expected).abs() <= 1);
    }

    #[test]
    fn sub_tick_advances_do_not_retrigger_init() {
        let mut sequencer = MidiSequencer::new(vec![]);
        sequencer.advance(120);
        assert_eq!(sequencer.get_tick(), 1);

        // 0.4 ms at 120 BPM is 0.768 ticks: no whole tick passes,
        // so current_tick stalls at 1 with last_tick == current_tick
        sequencer.advance_by(120, 0.0004);
        assert_eq!(sequencer.get_tick(), 1);
        assert_eq!(sequencer.get_last_tick(), 1);

        // the next sub-tick advance must accumulate to a whole tick,
        // not fall back into the init path (which would reset the position)
        sequencer.advance_by(120, 0.0004);
        assert_eq!(sequencer.get_tick(), 2);
        assert_eq!(sequencer.get_last_tick(), 1);
    }
    #[test]
    fn test_sequence_demo_song() {
        const FILE_PATH: &str = "test-files/Demo v5.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);
        let events_len = 4682;
        assert_eq!(events.len(), events_len);
        assert_eq!(events[0].tick, 1);
        let mut sequencer = MidiSequencer::new(events.clone());

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

    #[test]
    fn set_tick_includes_events_at_target() {
        // events at ticks 100, 200, 300
        let events = vec![
            MidiEvent {
                tick: 100,
                event: MidiEventType::NoteOn(0, 60, 95),
                track: Some(0),
            },
            MidiEvent {
                tick: 200,
                event: MidiEventType::NoteOn(0, 62, 95),
                track: Some(0),
            },
            MidiEvent {
                tick: 300,
                event: MidiEventType::NoteOn(0, 64, 95),
                track: Some(0),
            },
        ];
        let mut sequencer = MidiSequencer::new(events);

        // seek to tick 200 — set_tick sets last_tick and tick_position to 199
        sequencer.set_tick(200);
        // first advance takes the init path: last_tick stays 199, current tick becomes 200
        sequencer.advance(120);
        let batch = sequencer.get_next_events().unwrap();

        // should include the event at tick 200
        assert!(
            batch.iter().any(|e| e.tick == 200),
            "set_tick should include events at the target tick, got: {batch:?}"
        );
        // should NOT include event at tick 100 (before target)
        assert!(
            !batch.iter().any(|e| e.tick == 100),
            "set_tick should not include events before target tick"
        );
    }

    #[test]
    fn set_tick_on_song_with_repeats() {
        // verify seeking works correctly with repeat-expanded events
        const FILE_PATH: &str = "test-files/John Petrucci - Damage Control (ver 6 by Feio666).gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let playback_order =
            crate::audio::playback_order::compute_playback_order(&song.measure_headers);

        // build measure_playback_ticks (same logic as AudioPlayer::new)
        let measure_count = song.measure_headers.len();
        let mut measure_playback_ticks = vec![0_u32; measure_count];
        let mut seen = vec![false; measure_count];
        for &(measure_index, tick_offset) in &playback_order {
            if !seen[measure_index] {
                seen[measure_index] = true;
                let header = &song.measure_headers[measure_index];
                measure_playback_ticks[measure_index] =
                    (i64::from(header.start) + tick_offset) as u32;
            }
        }

        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);
        let mut sequencer = MidiSequencer::new(events.clone());

        // seek to measure 5 (index 4)
        let target_measure = 4;
        let target_tick = measure_playback_ticks[target_measure];
        assert!(
            target_tick > 0,
            "Measure 5 should have a non-zero playback tick"
        );

        sequencer.set_tick(target_tick);
        sequencer.advance(120);
        let batch = sequencer.get_next_events().unwrap();

        // verify we get events at or near the target tick, not from earlier measures
        if !batch.is_empty() {
            let min_tick = batch.iter().map(|e| e.tick).min().unwrap();
            assert!(
                min_tick >= target_tick,
                "After seeking to tick {target_tick}, got events at tick {min_tick}"
            );
        }
    }
}
