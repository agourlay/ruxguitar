use crate::parser::song_parser::{MeasureHeader, QUARTER_TIME};
use std::collections::HashMap;

/// Tracks the state of repeat section navigation during playback order computation.
struct RepeatState {
    start_stack: Vec<usize>,    // stack of repeat_open indices (for nesting)
    visits: HashMap<usize, i8>, // how many times each repeat_close has been hit
    current_repetition: i8,     // 0-based: 0 = first play, 1 = first repeat, etc.
    jumping_back: bool,         // true when looping back to a repeat_open
}

impl RepeatState {
    fn new() -> Self {
        Self {
            start_stack: vec![0], // implicit start at measure 0
            visits: HashMap::new(),
            current_repetition: 0,
            jumping_back: false,
        }
    }

    /// Process a repeat_open marker. Pushes onto the stack on first entry,
    /// skips the push when looping back (to preserve the repetition counter).
    fn enter_repeat(&mut self, measure_index: usize) {
        if !self.jumping_back {
            self.current_repetition = 0;
            self.start_stack.push(measure_index);
        }
        self.jumping_back = false;
    }

    /// Check if the current repetition matches an alternative ending bitmask.
    fn matches_alternative(&self, repeat_alternative: u8) -> bool {
        // clamp to 7 to avoid shift overflow on u8
        let clamped = self.current_repetition.min(7);
        let bit = 1_u8 << clamped;
        repeat_alternative & bit != 0
    }

    /// Process a repeat_close marker. Returns the index to jump back to,
    /// or None if all repetitions are done.
    fn close_repeat(&mut self, measure_index: usize, repeat_close: i8) -> Option<usize> {
        let visits = self.visits.entry(measure_index).or_insert(0);
        if *visits < repeat_close {
            *visits += 1;
            self.current_repetition += 1;
            self.jumping_back = true;
            let repeat_start = *self.start_stack.last().unwrap_or(&0);
            // clear visit counts for inner repeats so they replay on next outer pass
            self.visits
                .retain(|&k, _| k <= repeat_start || k >= measure_index);
            Some(repeat_start)
        } else {
            // done repeating
            self.visits.remove(&measure_index);
            self.start_stack.pop();
            None
        }
    }
}

/// Compute the playback order of measures, expanding repeats and alternative endings.
///
/// Used by the MIDI builder to generate events at the correct ticks,
/// and by the tablature to map playback ticks back to visual measures.
/// Returns a Vec of (measure_index, tick_offset) pairs.
/// The tick_offset is the difference between the playback tick and the original measure tick.
pub fn compute_playback_order(headers: &[MeasureHeader]) -> Vec<(usize, i64)> {
    let mut order: Vec<(usize, i64)> = Vec::new();
    let mut running_tick: u32 = QUARTER_TIME; // same starting tick as parser
    let mut repeat = RepeatState::new();
    let mut i = 0;

    while i < headers.len() {
        let header = &headers[i];

        if header.repeat_open {
            repeat.enter_repeat(i);
        }

        // check alternative ending: skip this measure if it doesn't match current repetition
        if header.repeat_alternative != 0 && !repeat.matches_alternative(header.repeat_alternative)
        {
            // still check for repeat_close on this skipped measure
            if header.repeat_close > 0
                && let Some(jump_to) = repeat.close_repeat(i, header.repeat_close)
            {
                i = jump_to;
                continue;
            }
            i += 1;
            continue;
        }

        // add this measure to the playback order
        let tick_offset = i64::from(running_tick) - i64::from(header.start);
        order.push((i, tick_offset));
        running_tick += header.length();

        // handle repeat close
        if header.repeat_close > 0
            && let Some(jump_to) = repeat.close_repeat(i, header.repeat_close)
        {
            i = jump_to;
            continue;
        }

        i += 1;
    }

    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_header(start: u32, repeat_open: bool, repeat_close: i8) -> MeasureHeader {
        MeasureHeader {
            start,
            repeat_open,
            repeat_close,
            ..MeasureHeader::default()
        }
    }

    #[test]
    fn no_repeats() {
        let headers = vec![
            make_header(960, false, 0),
            make_header(4800, false, 0),
            make_header(8640, false, 0),
        ];
        let order = compute_playback_order(&headers);
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], (0, 0));
        assert_eq!(order[1], (1, 0));
        assert_eq!(order[2], (2, 0));
    }

    #[test]
    fn simple_repeat() {
        // |: M0 | M1 :|  M2
        // Plays: M0 M1 M0 M1 M2
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            make_header(960 + measure_len, false, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 1, 2]);

        // tick offsets: first pass is 0, second pass shifts by 2 measures
        assert_eq!(order[0].1, 0);
        assert_eq!(order[1].1, 0);
        assert_eq!(order[2].1, i64::from(measure_len) * 2);
        assert_eq!(order[3].1, i64::from(measure_len) * 2);
        assert_eq!(order[4].1, i64::from(measure_len) * 2);
    }

    #[test]
    fn repeat_three_times() {
        // |: M0 :| x3  M1
        // Plays: M0 M0 M0 M1
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 2),
            make_header(960 + measure_len, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 0, 0, 1]);
    }

    #[test]
    fn two_repeat_sections() {
        // |: M0 :|  |: M1 :|  M2
        // Plays: M0 M0 M1 M1 M2
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 1),
            make_header(960 + measure_len, true, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 0, 1, 1, 2]);
    }

    #[test]
    fn alternative_endings() {
        // |: M0 | M1[1.] | M2[2.] :|  M3
        // Plays: M0 M1 M0 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 1,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 2,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 2, 3]);
    }

    #[test]
    fn three_alternatives() {
        // |: M0 | M1[1.] | M2[2.] | M3[3.] :|x3  M4
        // Plays: M0 M1 M0 M2 M0 M3 M4
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 1,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 2,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 3,
                repeat_alternative: 4,
                repeat_close: 2,
                ..MeasureHeader::default()
            },
            make_header(960 + measure_len * 4, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 2, 0, 3, 4]);
    }

    #[test]
    fn nested_repeats() {
        // |: M0 |: M1 :| M2 :|  M3
        // Plays: M0 M1 M1 M2 M0 M1 M1 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            make_header(960 + measure_len, true, 1),
            make_header(960 + measure_len * 2, false, 1),
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 1, 2, 0, 1, 1, 2, 3]);
    }

    #[test]
    fn repeat_close_without_open() {
        // M0 | M1 :|  M2
        // Plays: M0 M1 M0 M1 M2
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, false, 0),
            make_header(960 + measure_len, false, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 1, 2]);
    }

    #[test]
    fn single_measure_repeat() {
        // |: M0 :|
        // Plays: M0 M0
        let headers = vec![make_header(960, true, 1)];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 0]);
    }

    #[test]
    fn tick_offsets_are_consistent() {
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            make_header(960 + measure_len, false, 1),
            make_header(960 + measure_len * 2, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let playback_ticks: Vec<i64> = order
            .iter()
            .map(|(idx, offset)| i64::from(headers[*idx].start) + offset)
            .collect();
        assert!(playback_ticks.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn alternative_on_last_pass_with_close() {
        // |: M0 | M1[1.+2.] :|
        // Plays: M0 M1 M0 M1
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 3,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 1]);
    }

    #[test]
    fn empty() {
        let headers: Vec<MeasureHeader> = vec![];
        let order = compute_playback_order(&headers);
        assert!(order.is_empty());
    }
}
