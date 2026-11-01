use crate::audio::midi_event::MidiEvent;
use crate::parser::song_parser::QUARTER_TIME;
use std::ops::Range;

/// Events to play for one step of the playback, as index ranges into
/// `MidiSequencer::events`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Events reached by this step, `None` once the sequence is exhausted.
    pub events: Option<Range<usize>>,
    /// Tick reached by `events`.
    pub tick: u32,
    /// Set when the step crossed the loop end: playback restarted at the
    /// loop start, and these events of the new pass follow `events`.
    pub restart: Option<Range<usize>>,
}

pub struct MidiSequencer {
    last_tick: u32,
    /// Exact tick position; the current tick is its integer part.
    tick_position: f64,
    /// True until the first advance after a reset or a seek.
    needs_init: bool,
    sorted_events: Vec<MidiEvent>,
}

impl MidiSequencer {
    pub fn new(sorted_events: Vec<MidiEvent>) -> Self {
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

    pub fn reset_ticks(&mut self) {
        self.set_tick(0);
    }

    pub const fn get_tick(&self) -> u32 {
        self.tick_position as u32
    }

    #[cfg(test)]
    pub const fn get_last_tick(&self) -> u32 {
        self.last_tick
    }

    /// Advance like `advance`, looping over the playback ticks `[start, end)`
    /// when a loop is given. Crossing the loop end plays the events left
    /// before it, then restarts at the loop start with the time that ran
    /// past the end, so every pass keeps the beat.
    pub fn step(&mut self, tempo: u32, elapsed_secs: f64, loop_range: Option<(u32, u32)>) -> Step {
        self.advance(tempo, elapsed_secs);
        let Some((start, end)) = loop_range.filter(|&(start, end)| start < end) else {
            return self.pending_step();
        };
        if self.tick_position < f64::from(end) {
            return self.pending_step();
        }
        let (events, tick, overshoot) = if self.last_tick < end {
            let overshoot = (self.tick_position - f64::from(end)) % f64::from(end - start);
            // stop right before the loop end: what is left of this pass
            self.tick_position = f64::from(end - 1);
            (self.next_events_range(), end - 1, overshoot)
        } else {
            // a seek landed past the loop end: nothing is left of this pass
            (Some(Range::default()), self.last_tick, 0.0)
        };
        self.last_tick = start.saturating_sub(1);
        self.tick_position = f64::from(start) + overshoot;
        self.needs_init = false;
        Step {
            events,
            tick,
            restart: Some(self.next_events_range().unwrap_or_default()),
        }
    }

    /// The events reached so far, without advancing.
    pub fn pending_step(&self) -> Step {
        Step {
            events: self.next_events_range(),
            tick: self.get_tick(),
            restart: None,
        }
    }

    #[cfg(test)]
    pub fn get_next_events(&self) -> Option<&[MidiEvent]> {
        self.next_events_range()
            .map(|range| &self.sorted_events[range])
    }

    /// Index range of the events with tick in `(last_tick, current_tick]`,
    /// `None` once every event was played.
    fn next_events_range(&self) -> Option<Range<usize>> {
        let current_tick = self.get_tick();
        // do not return events if tick did not change
        if self.last_tick == current_tick {
            return Some(Range::default());
        }

        assert!(self.last_tick <= current_tick);

        // all events with tick in (last_tick, current_tick]
        // TODO could be improved by saving `end_index` to the next `start_index`
        let start_index = self
            .sorted_events
            .partition_point(|event| event.tick <= self.last_tick);
        // exit if end reached
        if start_index == self.sorted_events.len() {
            return None;
        }
        let len =
            self.sorted_events[start_index..].partition_point(|event| event.tick <= current_tick);
        Some(start_index..start_index + len)
    }

    /// Advance by the audio time about to be rendered, keeping event
    /// scheduling sample-locked to the output instead of a wall clock.
    pub fn advance(&mut self, tempo: u32, elapsed_secs: f64) {
        // init sequencer if first advance after a reset or seek
        if self.needs_init {
            self.needs_init = false;
            // tick_position is integral here (set from a u32), so the bump is exact
            self.tick_position += 1.0;
            return;
        }
        // cap the elapsed time so an oversized buffer request cannot
        // teleport playback far ahead; audio callbacks render ~0.1s
        const MAX_ELAPSED_SECS: f64 = 0.5;
        let elapsed_secs = elapsed_secs.min(MAX_ELAPSED_SECS);
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

pub fn tick_increase(tempo_bpm: u32, elapsed_seconds: f64) -> f64 {
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

    /// Ticks of the events in a step, the restarted pass after a `|`.
    fn step_ticks(sequencer: &MidiSequencer, step: &Step) -> Vec<String> {
        let ticks = |range: &Range<usize>| {
            sequencer.events()[range.clone()]
                .iter()
                .map(|event| event.tick.to_string())
                .collect::<Vec<_>>()
        };
        let mut out = step.events.as_ref().map(ticks).unwrap_or_default();
        if let Some(restart) = &step.restart {
            out.push("|".to_string());
            out.extend(ticks(restart));
        }
        out
    }

    // 120 BPM over 0.1s: 192 ticks per step
    const TEMPO: u32 = 120;
    const STEP_SECS: f64 = 0.1;

    #[test]
    fn loop_plays_every_event_once_per_pass() {
        let events = [1000, 1100, 1500, 1990, 2000, 2500]
            .map(|tick| MidiEvent::new_tempo_change(tick, 120))
            .to_vec();
        let mut sequencer = MidiSequencer::new(events);
        let loop_range = Some((1000, 2000));
        sequencer.set_tick(1000);

        let mut played = Vec::new();
        for _ in 0..12 {
            let step = sequencer.step(TEMPO, STEP_SECS, loop_range);
            played.extend(step_ticks(&sequencer, &step));
        }
        // first step only lands on the start (seek init), then 192 ticks a step;
        // the event at the loop end and after it never play
        assert_eq!(
            played,
            [
                "1000", "1100", "1500", "1990", "|", "1000", "1100", "1500", "1990", "|", "1000",
                "1100"
            ]
        );
    }

    #[test]
    fn loop_restart_carries_the_overshoot() {
        let mut sequencer = MidiSequencer::new(vec![MidiEvent::new_tempo_change(5000, 120)]);
        let loop_range = Some((1000, 2000));
        sequencer.set_tick(1000);
        sequencer.step(TEMPO, STEP_SECS, loop_range); // init: tick 1000
        for _ in 0..5 {
            let step = sequencer.step(TEMPO, STEP_SECS, loop_range);
            assert_eq!(step.restart, None);
        }
        assert_eq!(sequencer.get_tick(), 1960);
        // 1960 + 192 = 2152: 152 ticks past the end carry into the next pass
        let step = sequencer.step(TEMPO, STEP_SECS, loop_range);
        assert_eq!(step.tick, 1999);
        assert!(step.restart.is_some());
        assert_eq!(sequencer.get_tick(), 1152);
        assert_eq!(sequencer.get_last_tick(), 999);
    }

    #[test]
    fn loop_after_seek_past_its_end_restarts_at_start() {
        let mut sequencer = MidiSequencer::new(vec![MidiEvent::new_tempo_change(1000, 120)]);
        sequencer.set_tick(3000);
        let step = sequencer.step(TEMPO, STEP_SECS, Some((1000, 2000)));
        assert_eq!(step_ticks(&sequencer, &step), ["|", "1000"]);
        assert_eq!(sequencer.get_tick(), 1000);
    }

    #[test]
    fn empty_loop_plays_through() {
        let mut sequencer = MidiSequencer::new(vec![MidiEvent::new_tempo_change(1000, 120)]);
        sequencer.set_tick(900);
        sequencer.step(TEMPO, STEP_SECS, Some((2000, 2000)));
        let step = sequencer.step(TEMPO, STEP_SECS, Some((2000, 2000)));
        assert_eq!(step.restart, None);
        assert_eq!(step_ticks(&sequencer, &step), ["1000"]);
    }

    #[test]
    fn fractional_ticks_accumulate_across_advances() {
        let mut sequencer = MidiSequencer::new(vec![]);
        // first advance after reset bumps to tick 1
        sequencer.advance(120, 0.0);
        assert_eq!(sequencer.get_tick(), 1);

        // simulate 1000 audio callbacks of 5.8 ms each at 120 BPM
        // (256 frames at 44.1 kHz), i.e. 11.136 ticks per callback
        for _ in 0..1000 {
            sequencer.advance(120, 0.0058);
        }

        // exact total: 1 + 11.136 * 1000 = 11137 ticks
        // truncating per callback would yield 11001 (~1.2% slow)
        let expected = 1 + 11_136;
        assert!((i64::from(sequencer.get_tick()) - expected).abs() <= 1);
    }

    #[test]
    fn sub_tick_advances_do_not_retrigger_init() {
        let mut sequencer = MidiSequencer::new(vec![]);
        sequencer.advance(120, 0.0);
        assert_eq!(sequencer.get_tick(), 1);

        // 0.4 ms at 120 BPM is 0.768 ticks: no whole tick passes,
        // so current_tick stalls at 1 with last_tick == current_tick
        sequencer.advance(120, 0.0004);
        assert_eq!(sequencer.get_tick(), 1);
        assert_eq!(sequencer.get_last_tick(), 1);

        // the next sub-tick advance must accumulate to a whole tick,
        // not fall back into the init path (which would reset the position)
        sequencer.advance(120, 0.0004);
        assert_eq!(sequencer.get_tick(), 2);
        assert_eq!(sequencer.get_last_tick(), 1);
    }
    #[test]
    fn clock_jumps_are_clamped() {
        let mut sequencer = MidiSequencer::new(vec![]);
        sequencer.advance(120, 0.0); // init: tick 1

        // an hour-long elapsed request advances playback by at most 0.5s of
        // ticks: 120 BPM = 2 quarters/s * 960 * 0.5 = 960
        sequencer.advance(120, 3600.0);
        assert_eq!(sequencer.get_tick(), 961);
    }

    #[test]
    fn test_sequence_demo_song() {
        const FILE_PATH: &str = "test-files/Demo v5.gp5";
        let song = parse_gp_file(FILE_PATH).unwrap();
        let song = Rc::new(song);
        let builder = MidiBuilder::new();
        let events = builder.build_for_song(&song);
        let events_len = 5676;
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
    fn duplicate_ticks_are_delivered_exactly_once() {
        // chords produce several events at the same tick; batch boundaries
        // landing exactly on such a tick must neither drop nor replay events
        let event = |tick, key| MidiEvent {
            tick,
            event: MidiEventType::NoteOn(0, key, 95),
            track: Some(0),
        };
        let events = vec![
            event(5, 60),
            // chord at tick 10
            event(10, 60),
            event(10, 64),
            event(10, 67),
            // chord at tick 20
            event(20, 60),
            event(20, 64),
        ];
        let mut sequencer = MidiSequencer::new(events.clone());

        // first batch ends exactly on the chord tick: all three notes included
        sequencer.advance_tick(10); // last:0 current:10
        let batch = sequencer.get_next_events().unwrap();
        assert_eq!(batch, &events[0..4]);

        // next batch starts from the duplicated tick: nothing replayed
        sequencer.advance_tick(10); // last:10 current:20
        let batch = sequencer.get_next_events().unwrap();
        assert_eq!(batch, &events[4..6]);

        // all events consumed
        sequencer.advance_tick(10); // last:20 current:30
        assert!(sequencer.get_next_events().is_none());
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
        sequencer.advance(120, 0.0);
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
        let measure_playback_ticks = crate::audio::playback_order::first_playback_ticks(
            &song.measure_headers,
            &playback_order,
        );

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
        sequencer.advance(120, 0.0);
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
