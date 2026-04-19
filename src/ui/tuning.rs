/// Returns a human-readable tuning label for a stringed track.
/// Returns `None` for tracks with no strings (non-string instruments).
pub fn tuning_label(strings: &[(i32, i32)]) -> Option<String> {
    if strings.is_empty() {
        return None;
    }
    let mut pitches: Vec<i32> = strings.iter().map(|(_, pitch)| *pitch).collect();
    pitches.sort_unstable();

    if let Some(preset) = preset_name(&pitches) {
        return Some(preset.to_string());
    }

    Some(
        pitches
            .iter()
            .map(|p| note_name(*p))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn preset_name(pitches_sorted: &[i32]) -> Option<&'static str> {
    match pitches_sorted {
        // 6-string guitar
        [40, 45, 50, 55, 59, 64] => Some("Standard E"),
        [39, 44, 49, 54, 58, 63] => Some("Half-step down"),
        [38, 43, 48, 53, 57, 62] => Some("Standard D"),
        [37, 42, 47, 52, 56, 61] => Some("Standard C#"),
        [36, 41, 46, 51, 55, 60] => Some("Standard C"),
        [35, 40, 45, 50, 54, 59] => Some("Standard B"),
        [34, 39, 44, 49, 53, 58] => Some("Standard A#"),
        [38, 45, 50, 55, 59, 64] => Some("Drop D"),
        [36, 43, 48, 53, 57, 62] => Some("Drop C"),
        [33, 40, 45, 50, 54, 59] => Some("Drop A"),
        [38, 45, 50, 55, 57, 62] => Some("DADGAD"),
        [38, 43, 50, 55, 59, 62] => Some("Open G"),
        [38, 45, 50, 53, 57, 64] => Some("Open D minor (high E)"),
        // 7-string guitar
        [35, 40, 45, 50, 55, 59, 64] => Some("Standard B"),
        [34, 39, 44, 49, 54, 58, 63] => Some("Half-step down"),
        [33, 38, 43, 48, 53, 57, 62] => Some("Standard A"),
        [33, 40, 45, 50, 55, 59, 64] => Some("Drop A"),
        [29, 34, 39, 44, 49, 53, 58] => Some("Standard F"),
        // 4-string bass
        [28, 33, 38, 43] => Some("Standard E"),
        [27, 32, 37, 42] => Some("Half-step down"),
        [26, 31, 36, 41] => Some("Standard D"),
        [26, 33, 38, 43] => Some("Drop D"),
        // 5-string bass
        [23, 28, 33, 38, 43] => Some("Standard B"),
        [22, 27, 32, 37, 42] => Some("Half-step down"),
        [21, 28, 33, 38, 43] => Some("Drop A"),
        // 6-string bass
        [23, 28, 33, 38, 43, 48] => Some("Standard B"),
        _ => None,
    }
}

fn note_name(midi_pitch: i32) -> String {
    const NOTES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let note = NOTES[midi_pitch.rem_euclid(12) as usize];
    let octave = midi_pitch / 12 - 1;
    format!("{note}{octave}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_e_guitar() {
        let strings = vec![(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard E"));
    }

    #[test]
    fn drop_d_guitar() {
        let strings = vec![(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 38)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Drop D"));
    }

    #[test]
    fn standard_d_guitar() {
        // D G C F A D
        let strings = vec![(1, 62), (2, 57), (3, 53), (4, 48), (5, 43), (6, 38)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard D"));
    }

    #[test]
    fn standard_c_sharp_guitar() {
        // C# F# B E G# C#
        let strings = vec![(1, 61), (2, 56), (3, 52), (4, 47), (5, 42), (6, 37)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard C#"));
    }

    #[test]
    fn standard_c_guitar() {
        // C F A# D# G C
        let strings = vec![(1, 60), (2, 55), (3, 51), (4, 46), (5, 41), (6, 36)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard C"));
    }

    #[test]
    fn standard_a_sharp_guitar() {
        // A# D# G# C# F A#
        let strings = vec![(1, 58), (2, 53), (3, 49), (4, 44), (5, 39), (6, 34)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard A#"));
    }

    #[test]
    fn standard_b_guitar() {
        // B E A D F# B
        let strings = vec![(1, 59), (2, 54), (3, 50), (4, 45), (5, 40), (6, 35)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard B"));
    }

    #[test]
    fn drop_a_7_string() {
        // A E A D G B E
        let strings = vec![
            (1, 64),
            (2, 59),
            (3, 55),
            (4, 50),
            (5, 45),
            (6, 40),
            (7, 33),
        ];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Drop A"));
    }

    #[test]
    fn standard_a_7_string() {
        // A D G C F A D
        let strings = vec![
            (1, 62),
            (2, 57),
            (3, 53),
            (4, 48),
            (5, 43),
            (6, 38),
            (7, 33),
        ];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard A"));
    }

    #[test]
    fn standard_f_7_string() {
        // F A# D# G# C# F A#
        let strings = vec![
            (1, 58),
            (2, 53),
            (3, 49),
            (4, 44),
            (5, 39),
            (6, 34),
            (7, 29),
        ];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard F"));
    }

    #[test]
    fn standard_bass() {
        let strings = vec![(1, 43), (2, 38), (3, 33), (4, 28)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard E"));
    }

    #[test]
    fn open_d_minor_high_e() {
        // D A D F A E
        let strings = vec![(1, 64), (2, 57), (3, 53), (4, 50), (5, 45), (6, 38)];
        assert_eq!(
            tuning_label(&strings).as_deref(),
            Some("Open D minor (high E)")
        );
    }

    #[test]
    fn drop_a_6_string() {
        // A E A D F# B
        let strings = vec![(1, 59), (2, 54), (3, 50), (4, 45), (5, 40), (6, 33)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Drop A"));
    }

    #[test]
    fn drop_a_5_string_bass() {
        // A E A D G
        let strings = vec![(1, 43), (2, 38), (3, 33), (4, 28), (5, 21)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Drop A"));
    }

    #[test]
    fn standard_d_bass() {
        // D G C F
        let strings = vec![(1, 41), (2, 36), (3, 31), (4, 26)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard D"));
    }

    #[test]
    fn standard_b_6_string_bass() {
        // B E A D G C
        let strings = vec![(1, 48), (2, 43), (3, 38), (4, 33), (5, 28), (6, 23)];
        assert_eq!(tuning_label(&strings).as_deref(), Some("Standard B"));
    }

    #[test]
    fn empty_strings_returns_none() {
        assert_eq!(tuning_label(&[]), None);
    }

    #[test]
    fn unknown_tuning_falls_back_to_notes() {
        // arbitrary 6-string tuning
        let strings = vec![(1, 65), (2, 60), (3, 56), (4, 51), (5, 46), (6, 41)];
        assert_eq!(
            tuning_label(&strings).as_deref(),
            Some("F2 A#2 D#3 G#3 C4 F4")
        );
    }

    #[test]
    fn note_name_e2() {
        assert_eq!(note_name(40), "E2");
    }

    #[test]
    fn note_name_middle_c() {
        assert_eq!(note_name(60), "C4");
    }
}
