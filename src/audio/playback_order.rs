use crate::parser::song_parser::{MeasureHeader, QUARTER_TIME};

/// Compute the playback order of measures, expanding repeats and alternative endings.
///
/// Used by the MIDI builder to generate events at the correct ticks,
/// and by the tablature to map playback ticks back to visual measures.
/// Returns a Vec of (measure_index, tick_offset) pairs.
/// The tick_offset is the difference between the playback tick and the original measure tick.
///
/// Port of TuxGuitar's `MidiRepeatController` semantics:
/// - the first measure implicitly opens a repeat section
/// - a repeat_open marker restarts the section (Guitar Pro has no nested repeats)
///   and resets the counters only on its first pass
/// - the alternative ending bitmask latches from the first marked measure until
///   a repeat_close, so unmarked measures under a volta bracket inherit it
/// - a repeat_close on a skipped measure only clears the latch, it never jumps
/// - a repeat_close inside an alternative ending always jumps back; the section
///   ends by falling through an ending without a repeat_close
pub fn compute_playback_order(headers: &[MeasureHeader]) -> Vec<(usize, i64)> {
    let mut order: Vec<(usize, i64)> = Vec::new();
    // i64: keeps the accumulator itself from overflowing on absurd repeat
    // counts; downstream event ticks remain u32 (the practical timeline limit)
    let mut running_tick: i64 = i64::from(QUARTER_TIME); // same starting tick as parser

    let mut index: usize = 0;
    let mut last_played: i64 = -1; // highest measure index played so far
    let mut repeat_start_index: usize = 0;
    let mut repeat_open = true; // first measure implicitly opens a repeat
    let mut repeat_number: i8 = 0; // 0-based repetition counter
    let mut repeat_alternative: u8 = 0; // latched alternative ending bitmask

    while index < headers.len() {
        let header = &headers[index];
        let mut should_play = true;

        if header.repeat_open {
            repeat_start_index = index;
            repeat_open = true;
            // reset counters only on the first pass over this measure
            if index as i64 > last_played {
                repeat_number = 0;
                repeat_alternative = 0;
            }
        } else {
            // latch the alternative ending bitmask from the first marked measure
            if repeat_alternative == 0 {
                repeat_alternative = header.repeat_alternative;
            }
            // inside an alternative ending, the measure only plays if the
            // latched mask matches the current repetition
            if repeat_open
                && repeat_alternative > 0
                && repeat_alternative & repetition_bit(repeat_number) == 0
            {
                should_play = false;
                // the close of a skipped ending terminates the latch but never jumps
                if header.repeat_close > 0 {
                    repeat_alternative = 0;
                }
            }
        }

        if should_play {
            last_played = last_played.max(index as i64);
            let tick_offset = running_tick - i64::from(header.start);
            order.push((index, tick_offset));
            running_tick += i64::from(header.length());

            if repeat_open && header.repeat_close > 0 {
                if repeat_number < header.repeat_close || repeat_alternative > 0 {
                    repeat_number += 1;
                    repeat_alternative = 0;
                    index = repeat_start_index;
                    continue;
                }
                // done repeating
                repeat_open = false;
                repeat_number = 0;
                repeat_alternative = 0;
            }
        }
        index += 1;
    }

    order
}

/// Translate a tick from the original timeline into the expanded playback timeline.
pub fn playback_tick(original_tick: u32, tick_offset: i64) -> u32 {
    (i64::from(original_tick) + tick_offset) as u32
}

/// First playback tick of each measure, used for seeking.
///
/// Measures the playback never reaches (e.g. an alternative ending whose
/// repetition never occurs) fall back to the first playback tick of the
/// closest preceding played measure.
pub fn first_playback_ticks(headers: &[MeasureHeader], order: &[(usize, i64)]) -> Vec<u32> {
    let mut first_ticks: Vec<Option<u32>> = vec![None; headers.len()];
    for &(measure_index, tick_offset) in order {
        let slot = &mut first_ticks[measure_index];
        if slot.is_none() {
            *slot = Some(playback_tick(headers[measure_index].start, tick_offset));
        }
    }
    // carry forward over never-played measures
    let mut carried = 0;
    first_ticks
        .into_iter()
        .map(|tick| {
            if let Some(tick) = tick {
                carried = tick;
            }
            carried
        })
        .collect()
}

/// Bit for the given repetition in an alternative ending bitmask.
/// Repetitions beyond the 8th never match.
const fn repetition_bit(repetition: i8) -> u8 {
    if repetition >= 0 && repetition < 8 {
        1 << repetition
    } else {
        0
    }
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
        // |: M0 | M1[1.] :| M2[2.] | M3
        // Each ending carries its own close (as in real GP files); the last
        // ending has none and falls through.
        // Plays: M0 M1 M0 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 1,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 2,
                ..MeasureHeader::default()
            },
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 2, 3]);
    }

    #[test]
    fn first_playback_ticks_carry_over_never_played_measures() {
        // |: M0 | M1[1.] :| M2[3.] :| M3
        // One repeat only: ending "3." never plays.
        // Plays: M0 M1 M0 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 0b001,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 0b100,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 3]);

        // seeking to the never-played M2 falls back to M1's first playback tick
        let first_ticks = first_playback_ticks(&headers, &order);
        assert_eq!(first_ticks, vec![960, 4800, 4800, 12480]);
    }

    #[test]
    fn three_alternatives() {
        // |: M0 | M1[1.] :| M2[2.] :| M3[3.] | M4
        // Plays: M0 M1 M0 M2 M0 M3 M4
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 1,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 2,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 3,
                repeat_alternative: 4,
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
        // Guitar Pro has no nested repeats: the second open restarts the
        // section, and the outer close is ignored once the inner section
        // completed (TuxGuitar semantics).
        // Plays: M0 M1 M1 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            make_header(960 + measure_len, true, 1),
            make_header(960 + measure_len * 2, false, 1),
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 1, 2, 3]);
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
        // A close inside an alternative ending always jumps back, so M0 plays
        // once more before M1 (matching no further ending) falls through
        // (TuxGuitar semantics).
        // Plays: M0 M1 M0 M1 M0
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
        assert_eq!(indices, vec![0, 1, 0, 1, 0]);
    }

    #[test]
    fn empty() {
        let headers: Vec<MeasureHeader> = vec![];
        let order = compute_playback_order(&headers);
        assert!(order.is_empty());
    }

    #[test]
    fn trailing_alternative_after_close() {
        // |: M0 | M1[1.] :| M2[2.] | M3
        // The second ending lives after the closing measure; the repetition
        // counter must survive the repeat completing for M2 to match.
        // Plays: M0 M1 M0 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 0),
            MeasureHeader {
                start: 960 + measure_len,
                repeat_alternative: 1,
                repeat_close: 1,
                ..MeasureHeader::default()
            },
            MeasureHeader {
                start: 960 + measure_len * 2,
                repeat_alternative: 2,
                ..MeasureHeader::default()
            },
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 2, 3]);
    }

    #[test]
    fn completed_repeat_then_bare_close() {
        // |: M0 :|  M1 :|
        // A close without a new open after a completed section is ignored
        // (TuxGuitar semantics); it must not replay the song from measure 0.
        // Plays: M0 M0 M1
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, true, 1),
            make_header(960 + measure_len, false, 1),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 0, 1]);
    }

    #[test]
    fn bare_close_does_not_leak_into_next_repeat() {
        // M0 | M1 :|  |: M2 :|  M3
        // The jump back to measure 0 (which has no repeat_open) must not
        // corrupt the state of the later explicit repeat on M2.
        // Plays: M0 M1 M0 M1 M2 M2 M3
        let measure_len = 3840_u32;
        let headers = vec![
            make_header(960, false, 0),
            make_header(960 + measure_len, false, 1),
            make_header(960 + measure_len * 2, true, 1),
            make_header(960 + measure_len * 3, false, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 1, 2, 2, 3]);
    }

    #[test]
    fn sequential_sections_with_alternatives() {
        // |: M0 | M1[1.] :| M2[2.]  |: M3 | M4[1.] :| M5[2.]
        // The second section starts right after the first one; the repetition
        // counter must reset on its repeat_open for M4's first ending to match.
        // Plays: M0 M1 M0 M2 M3 M4 M3 M5
        let measure_len = 3840_u32;
        let alt = |idx: u32, repeat_alternative: u8, repeat_close: i8| MeasureHeader {
            start: 960 + measure_len * idx,
            repeat_alternative,
            repeat_close,
            ..MeasureHeader::default()
        };
        let headers = vec![
            make_header(960, true, 0),
            alt(1, 1, 1),
            alt(2, 2, 0),
            make_header(960 + measure_len * 3, true, 0),
            alt(4, 1, 1),
            alt(5, 2, 0),
        ];
        let order = compute_playback_order(&headers);
        let indices: Vec<usize> = order.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 0, 2, 3, 4, 3, 5]);
    }
}
