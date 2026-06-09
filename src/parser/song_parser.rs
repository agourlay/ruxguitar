use crate::RuxError;
use crate::parser::music_parser::MusicParser;
use crate::parser::primitive_parser::{
    parse_bool, parse_byte_size_string, parse_i8, parse_int, parse_int_byte_sized_string,
    parse_int_sized_string, parse_short, parse_u8, skip,
};
use nom::IResult;
use nom::Parser;
use nom::bytes::complete::take;
use nom::combinator::{cond, flat_map, map};
use nom::multi::count;
use nom::sequence::preceded;

// GP4 docs at <https://dguitar.sourceforge.net/GP4format.html>
// GP5 docs thanks to Tuxguitar and <https://github.com/slundi/guitarpro> for the help

pub use crate::parser::model::*;

/// Convert raw GP stroke byte to a duration value.
const fn to_stroke_value(raw: i8) -> u16 {
    match raw {
        1 | 2 => DURATION_SIXTY_FOURTH as u16,
        3 => DURATION_THIRTY_SECOND as u16,
        4 => DURATION_SIXTEENTH as u16,
        5 => DURATION_EIGHTH as u16,
        6 => QUARTER,
        _ => DURATION_SIXTY_FOURTH as u16,
    }
}

pub fn parse_chord(
    string_count: u8,
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], Chord> {
    move |i| {
        log::debug!("Parsing chords for {string_count} strings");
        let mut i = i;
        let mut chord = Chord {
            strings: vec![-1; string_count.into()],
            ..Default::default()
        };
        let (inner, chord_gp4_header) = parse_u8(i)?;
        i = inner;

        // chord header defines the version as well
        if (chord_gp4_header & 0x01) == 0 {
            log::debug!("Parsing simple chord");
            let (inner, chord_name) = parse_int_byte_sized_string(i)?;
            log::debug!("Chord name {chord_name}");
            i = inner;
            chord.name = chord_name;
            let (inner, first_fret) = parse_int(i)?;
            i = inner;
            log::debug!("Chord first fret {first_fret}");
            chord.first_fret = Some(first_fret as u32);
            if first_fret != 0 {
                for c in 0..6 {
                    let (inner, fret) = parse_int(i)?;
                    if c < string_count {
                        chord.strings[c as usize] = fret as i8;
                    }
                    i = inner;
                }
            }
        } else if version == GpVersion::GP3 {
            log::debug!("Parsing diagram style chord (GP3)");
            i = skip(i, 25);
            let (inner, chord_name) = parse_byte_size_string(34)(i)?;
            i = inner;
            log::debug!("Chord name {chord_name}");
            chord.name = chord_name;
            let (inner, first_fret) = parse_int(i)?;
            i = inner;
            log::debug!("Chord first fret {first_fret}");
            chord.first_fret = Some(first_fret as u32);
            for c in 0..6 {
                let (inner, fret) = parse_int(i)?;
                i = inner;
                log::debug!("Chord fret {c}:{fret}");
                if c < string_count {
                    chord.strings[c as usize] = fret as i8;
                }
            }
            i = skip(i, 36);
        } else {
            log::debug!("Parsing diagram style chord");
            i = skip(i, 16);
            let (inner, chord_name) = parse_byte_size_string(21)(i)?;
            i = inner;
            log::debug!("Chord name {chord_name}");
            chord.name = chord_name;
            i = skip(i, 4);
            let (inner, first_fret) = parse_int(i)?;
            i = inner;
            log::debug!("Chord first fret {first_fret}");
            chord.first_fret = Some(first_fret as u32);
            for c in 0..7 {
                let (inner, fret) = parse_int(i)?;
                i = inner;
                log::debug!("Chord fret {c}:{fret}");
                if c < string_count {
                    chord.strings[c as usize] = fret as i8;
                }
            }
            i = skip(i, 32);
        }
        Ok((i, chord))
    }
}

pub fn parse_note_effects(
    note: &mut Note,
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], ()> + '_ {
    move |i| {
        log::debug!("Parsing note effects");
        let mut i = i;

        // GP3 stores note effects in a single flags byte with a reduced feature set.
        if version == GpVersion::GP3 {
            let (inner, flags) = parse_u8(i)?;
            i = inner;
            note.effect.hammer = (flags & 0x02) != 0;
            note.effect.let_ring = (flags & 0x08) != 0;
            // GP3 slide is a plain boolean; map it to a shift slide.
            if (flags & 0x04) != 0 {
                note.effect.slide = Some(SlideType::ShiftSlideTo);
            }
            if (flags & 0x01) != 0 {
                let (inner, bend_effect) = parse_bend_effect(i)?;
                i = inner;
                note.effect.bend = Some(bend_effect);
            }
            if (flags & 0x10) != 0 {
                let (inner, grace_effect) = parse_grace_effect(version)(i)?;
                i = inner;
                note.effect.grace = Some(grace_effect);
            }
            return Ok((i, ()));
        }

        let (inner, (flags1, flags2)) = (parse_u8, parse_u8).parse(i)?;
        i = inner;
        note.effect.hammer = (flags1 & 0x02) == 0x02;
        note.effect.let_ring = (flags1 & 0x08) == 0x08;

        note.effect.staccato = (flags2 & 0x01) == 0x01;
        note.effect.palm_mute = (flags2 & 0x02) == 0x02;
        note.effect.vibrato = (flags2 & 0x40) == 0x40 || note.effect.vibrato;

        if (flags1 & 0x01) != 0 {
            let (inner, bend_effect) = parse_bend_effect(i)?;
            i = inner;
            note.effect.bend = Some(bend_effect);
        }

        if (flags1 & 0x10) != 0 {
            let (inner, grace_effect) = parse_grace_effect(version)(i)?;
            i = inner;
            note.effect.grace = Some(grace_effect);
        }

        if (flags2 & 0x04) != 0 {
            let (inner, tremolo_picking) = parse_tremolo_picking(i)?;
            i = inner;
            note.effect.tremolo_picking = Some(tremolo_picking);
        }

        if (flags2 & 0x08) != 0 {
            let (inner, slide_type) = parse_slide_type(i)?;
            i = inner;
            note.effect.slide = slide_type;
        }

        if (flags2 & 0x10) != 0 {
            let (inner, harmonic_effect) = parse_harmonic_effect(version)(i)?;
            i = inner;
            note.effect.harmonic = Some(harmonic_effect);
        }

        if (flags2 & 0x20) != 0 {
            let (inner, trill_effect) = parse_trill_effect(i)?;
            i = inner;
            note.effect.trill = Some(trill_effect);
        }

        Ok((i, ()))
    }
}

pub fn parse_trill_effect(i: &[u8]) -> IResult<&[u8], TrillEffect> {
    log::debug!("Parsing trill effect");
    let mut trill_effect = TrillEffect::default();
    let (inner, (fret, period)) = (parse_i8, parse_i8).parse(i)?;
    trill_effect.fret = fret;
    trill_effect.duration.value = TrillEffect::from_trill_period(period);
    Ok((inner, trill_effect))
}

pub fn parse_harmonic_effect(
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], HarmonicEffect> {
    move |i| {
        let mut i = i;
        let mut he = HarmonicEffect::default();
        let (inner, harmonic_type) = parse_i8(i)?;
        i = inner;
        log::debug!("Parsing harmonic effect {harmonic_type}");
        match harmonic_type {
            1 => he.kind = HarmonicType::Natural,
            2 => {
                he.kind = HarmonicType::Artificial;
                if version >= GpVersion::GP5 {
                    let (inner, (semitone, accidental, octave)) =
                        (parse_u8, parse_i8, parse_u8).parse(i)?;
                    i = inner;
                    he.pitch = Some(PitchClass::from(semitone as i8, Some(accidental), None));
                    he.octave = Some(Octave::get_octave(octave));
                }
            }
            3 => {
                he.kind = HarmonicType::Tapped;
                if version >= GpVersion::GP5 {
                    let (inner, fret) = parse_u8(i)?;
                    i = inner;
                    he.right_hand_fret = Some(fret as i8);
                }
            }
            4 => he.kind = HarmonicType::Pinch,
            5 => he.kind = HarmonicType::Semi,
            15 => {
                assert!(
                    version < GpVersion::GP5,
                    "Cannot read artificial harmonic type for GP4"
                );
                he.kind = HarmonicType::Artificial;
            }
            17 => {
                assert!(
                    version < GpVersion::GP5,
                    "Cannot read artificial harmonic type for GP4"
                );
                he.kind = HarmonicType::Artificial;
            }
            22 => {
                assert!(
                    version < GpVersion::GP5,
                    "Cannot read artificial harmonic type for GP4"
                );
                he.kind = HarmonicType::Artificial;
            }
            x => panic!("Cannot read harmonic type {x}"),
        };
        Ok((i, he))
    }
}

pub fn parse_slide_type(i: &[u8]) -> IResult<&[u8], Option<SlideType>> {
    map(parse_i8, |t| {
        log::debug!("Parsing slide type {t}");
        if (t & 0x01) == 0x01 {
            Some(SlideType::ShiftSlideTo)
        } else if (t & 0x02) == 0x02 {
            Some(SlideType::LegatoSlideTo)
        } else if (t & 0x04) == 0x04 {
            Some(SlideType::OutDownwards)
        } else if (t & 0x08) == 0x08 {
            Some(SlideType::OutUpWards)
        } else if (t & 0x10) == 0x10 {
            Some(SlideType::IntoFromBelow)
        } else if (t & 0x20) == 0x20 {
            Some(SlideType::IntoFromAbove)
        } else {
            None
        }
    })
    .parse(i)
}

pub fn parse_tremolo_picking(i: &[u8]) -> IResult<&[u8], TremoloPickingEffect> {
    log::debug!("Parsing tremolo picking");
    map(parse_u8, |tp| {
        let value = TremoloPickingEffect::from_tremolo_value(tp as i8);
        let mut tremolo_picking_effect = TremoloPickingEffect::default();
        tremolo_picking_effect.duration.value = value;
        tremolo_picking_effect
    })
    .parse(i)
}

pub fn parse_grace_effect(version: GpVersion) -> impl FnMut(&[u8]) -> IResult<&[u8], GraceEffect> {
    move |i| {
        log::debug!("Parsing grace effect");
        let mut i = i;
        let mut grace_effect = GraceEffect::default();

        // fret
        let (inner, fret) = parse_u8(i)?;
        i = inner;
        if version < GpVersion::GP5 {
            // Pre-GP5 encodes a dead grace note as fret 255.
            grace_effect.is_dead = fret == 255;
            grace_effect.fret = if grace_effect.is_dead { 0 } else { fret as i8 };
        } else {
            grace_effect.fret = fret as i8;
        }

        // velocity
        let (inner, velocity) = parse_u8(i)?;
        i = inner;
        grace_effect.velocity = convert_velocity(i16::from(velocity));

        // transition
        let (inner, transition) = parse_i8(i)?;
        i = inner;
        grace_effect.transition = GraceEffectTransition::get_grace_effect_transition(transition);

        // duration
        let (inner, duration) = parse_u8(i)?;
        i = inner;
        grace_effect.duration = duration;

        if version >= GpVersion::GP5 {
            // flags
            let (inner, flags) = parse_u8(i)?;
            i = inner;
            grace_effect.is_dead = (flags & 0x01) == 0x01;
            grace_effect.is_on_beat = (flags & 0x02) == 0x02;
        }

        Ok((i, grace_effect))
    }
}

pub fn parse_beat_effects<'a>(
    beat: &'a mut Beat,
    note_effect: &'a mut NoteEffect,
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], ()> + 'a {
    move |i| {
        log::debug!("Parsing beat effects");
        let mut i = i;

        // GP3 stores beat effects in a single flags byte and places harmonics here
        // (instead of at the note level like GP4+).
        if version == GpVersion::GP3 {
            let (inner, flags) = parse_u8(i)?;
            i = inner;
            note_effect.vibrato = (flags & 0x01) != 0 || (flags & 0x02) != 0;
            note_effect.fade_in = (flags & 0x10) != 0;

            if flags & 0x20 != 0 {
                let (inner, effect_type) = parse_u8(i)?;
                i = inner;
                if effect_type == 0 {
                    let (inner, tremolo_bar) = parse_tremolo_bar_gp3(i)?;
                    i = inner;
                    note_effect.tremolo_bar = Some(tremolo_bar);
                } else {
                    note_effect.slap = match effect_type {
                        1 => SlapEffect::Tapping,
                        2 => SlapEffect::Slapping,
                        3 => SlapEffect::Popping,
                        _ => SlapEffect::None,
                    };
                    i = skip(i, 4); // trailing int value
                }
            }

            if flags & 0x40 != 0 {
                // GP3 stores the down stroke first, then the up stroke.
                let (inner, (stroke_down, stroke_up)) = (parse_i8, parse_i8).parse(i)?;
                i = inner;
                if stroke_down > 0 {
                    beat.effect.stroke.value = to_stroke_value(stroke_down);
                    beat.effect.stroke.direction = BeatStrokeDirection::Down;
                } else if stroke_up > 0 {
                    beat.effect.stroke.value = to_stroke_value(stroke_up);
                    beat.effect.stroke.direction = BeatStrokeDirection::Up;
                }
            }

            if flags & 0x04 != 0 {
                note_effect.harmonic = Some(HarmonicEffect::default()); // natural
            }
            if flags & 0x08 != 0 {
                note_effect.harmonic = Some(HarmonicEffect {
                    kind: HarmonicType::Artificial,
                    ..Default::default()
                });
            }

            return Ok((i, ()));
        }

        let (inner, (flags1, flags2)) = (parse_u8, parse_u8).parse(i)?;
        i = inner;

        note_effect.fade_in = flags1 & 0x10 != 0;
        note_effect.vibrato = flags1 & 0x02 != 0;

        if flags1 & 0x20 != 0 {
            let (inner, effect) = parse_u8(i)?;
            i = inner;
            log::debug!("Parsing tapping effect {effect}");
            note_effect.slap = match effect {
                1 => SlapEffect::Tapping,
                2 => SlapEffect::Slapping,
                3 => SlapEffect::Popping,
                _ => SlapEffect::None,
            };
        }

        if flags2 & 0x04 != 0 {
            let (inner, effect) = parse_tremolo_bar(i)?;
            i = inner;
            note_effect.tremolo_bar = Some(effect);
        }

        if flags1 & 0x40 != 0 {
            log::debug!("Parsing stroke effect");
            let (inner, (first, second)) = (parse_i8, parse_i8).parse(i)?;
            i = inner;
            // GP5 stores the up stroke first, GP4 stores the down stroke first.
            let (stroke_up, stroke_down) = if version >= GpVersion::GP5 {
                (first, second)
            } else {
                (second, first)
            };
            if stroke_up > 0 {
                beat.effect.stroke.value = to_stroke_value(stroke_up);
                beat.effect.stroke.direction = BeatStrokeDirection::Up;
            } else if stroke_down > 0 {
                beat.effect.stroke.value = to_stroke_value(stroke_down);
                beat.effect.stroke.direction = BeatStrokeDirection::Down;
            }
        }

        if flags2 & 0x02 != 0 {
            i = skip(i, 1);
        }

        Ok((i, ()))
    }
}

pub fn parse_bend_effect(i: &[u8]) -> IResult<&[u8], BendEffect> {
    log::debug!("Parsing bend effect");
    let mut i = skip(i, 5);
    let mut bend_effect = BendEffect::default();
    let (inner, num_points) = parse_int(i)?;
    i = inner;
    for _ in 0..num_points {
        let (inner, (bend_position, bend_value, _vibrato)) =
            (parse_int, parse_int, parse_i8).parse(i)?;
        i = inner;

        let point_position =
            bend_position as f32 * BEND_EFFECT_MAX_POSITION_LENGTH / GP_BEND_POSITION;
        let point_value = bend_value as f32 * SEMITONE_LENGTH / GP_BEND_SEMITONE;
        bend_effect.points.push(BendPoint {
            position: point_position.round() as u8,
            value: point_value.round() as i8,
        });
    }
    Ok((i, bend_effect))
}

/// GP3 stores a tremolo bar as a single dip value from which a 3-point
/// dive-and-return curve is synthesized.
pub fn parse_tremolo_bar_gp3(i: &[u8]) -> IResult<&[u8], TremoloBarEffect> {
    log::debug!("Parsing tremolo bar (GP3)");
    let (i, value) = parse_int(i)?;
    let mut tremolo_bar_effect = TremoloBarEffect::default();
    tremolo_bar_effect.points.push(BendPoint {
        position: 0,
        value: 0,
    });
    tremolo_bar_effect.points.push(BendPoint {
        position: (BEND_EFFECT_MAX_POSITION_LENGTH / 2.0).round() as u8,
        value: (-(value as f32 / (GP_BEND_SEMITONE * 2.0))).round() as i8,
    });
    tremolo_bar_effect.points.push(BendPoint {
        position: BEND_EFFECT_MAX_POSITION_LENGTH as u8,
        value: 0,
    });
    Ok((i, tremolo_bar_effect))
}

pub fn parse_tremolo_bar(i: &[u8]) -> IResult<&[u8], TremoloBarEffect> {
    log::debug!("Parsing tremolo bar");
    let mut i = skip(i, 5);
    let mut tremolo_bar_effect = TremoloBarEffect::default();
    let (inner, num_points) = parse_int(i)?;
    i = inner;
    for _ in 0..num_points {
        let (inner, (position, value, _vibrato)) = (parse_int, parse_int, parse_i8).parse(i)?;
        i = inner;

        let point_position = position as f32 * BEND_EFFECT_MAX_POSITION_LENGTH / GP_BEND_POSITION;
        let point_value = value as f32 / GP_BEND_SEMITONE * 2.0f32;
        tremolo_bar_effect.points.push(BendPoint {
            position: point_position.round() as u8,
            value: point_value.round() as i8,
        });
    }
    Ok((i, tremolo_bar_effect))
}

/// Read beat duration.
/// Duration is composed of byte signifying duration and an integer that maps to `Tuplet`. The byte maps to following values:
///
/// * *-2*: whole note
/// * *-1*: half note
/// * *0*: quarter note
/// * *1*: eighth note
/// * *2*: sixteenth note
/// * *3*: thirty-second note
///
/// If flag at *0x20* is true, the tuplet is read
pub fn parse_duration(flags: u8) -> impl FnMut(&[u8]) -> IResult<&[u8], Duration> {
    move |i: &[u8]| {
        log::debug!("Parsing duration");
        let mut i = i;
        let mut d = Duration::default();
        let (inner, value) = parse_i8(i)?;
        i = inner;
        d.value = (2_u32.saturating_pow((value + 4) as u32) / 4) as u16;
        log::debug!("Duration value: {}", d.value);
        d.dotted = flags & 0x01 != 0;

        if (flags & 0x20) != 0 {
            let (inner, i_tuplet) = parse_int(i)?;
            i = inner;

            match i_tuplet {
                3 => {
                    d.tuplet_enters = i_tuplet as u8;
                    d.tuplet_times = 2;
                }
                5..=7 => {
                    d.tuplet_enters = i_tuplet as u8;
                    d.tuplet_times = 4;
                }
                9..=13 => {
                    d.tuplet_enters = i_tuplet as u8;
                    d.tuplet_times = 8;
                }
                x => log::debug!("Unknown tuplet: {x}"),
            }
        }

        Ok((i, d))
    }
}

pub fn parse_color(i: &[u8]) -> IResult<&[u8], i32> {
    log::debug!("Parsing RGB color");
    map(
        (parse_u8, parse_u8, parse_u8, parse_u8),
        |(r, g, b, _alpha)| (i32::from(r) << 16) | (i32::from(g) << 8) | i32::from(b),
    )
    .parse(i)
}

pub fn parse_marker(i: &[u8]) -> IResult<&[u8], Marker> {
    log::debug!("Parsing marker");
    map(
        (parse_int_byte_sized_string, parse_color),
        |(title, color)| Marker { title, color },
    )
    .parse(i)
}

pub fn parse_triplet_feel(i: &[u8]) -> IResult<&[u8], TripletFeel> {
    log::debug!("Parsing triplet feel");
    map(parse_i8, |triplet_feel| match triplet_feel {
        0 => TripletFeel::None,
        1 => TripletFeel::Eighth,
        2 => TripletFeel::Sixteenth,
        x => panic!("Unknown triplet feel: {x}"),
    })
    .parse(i)
}

/// Parse measure header.
/// the time signature is propagated to the next measure
pub fn parse_measure_header(
    previous_time_signature: TimeSignature,
    song_tempo: u32,
    song_version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], MeasureHeader> {
    move |i: &[u8]| {
        log::debug!("Parsing measure header");
        let (mut i, flags) = parse_u8(i)?;
        log::debug!("Flags: {flags:08b}");
        let mut mh = MeasureHeader::default();
        mh.tempo.value = song_tempo; // value updated later when parsing beats
        mh.repeat_open = (flags & 0x04) != 0;
        // propagate time signature
        mh.time_signature = previous_time_signature.clone();

        // Numerator of the (key) signature
        if (flags & 0x01) != 0 {
            log::debug!("Parsing numerator");
            let (inner, numerator) = parse_i8(i)?;
            i = inner;
            mh.time_signature.numerator = numerator as u8;
        }

        // Denominator of the (key) signature
        if (flags & 0x02) != 0 {
            log::debug!("Parsing denominator");
            let (inner, denominator_value) = parse_i8(i)?;
            i = inner;
            let denominator = Duration {
                value: denominator_value as u16,
                ..Default::default()
            };
            mh.time_signature.denominator = denominator;
        }

        if song_version >= GpVersion::GP5 {
            // Beginning of repeat
            if (flags & 0x08) != 0 {
                log::debug!("Parsing repeat close");
                let (inner, repeat_close) = parse_i8(i)?;
                i = inner;
                mh.repeat_close = repeat_close - 1; // GP5 specific logic
            }

            // Presence of a marker
            if (flags & 0x20) != 0 {
                let (inner, marker) = parse_marker(i)?;
                i = inner;
                mh.marker = Some(marker);
            }

            // Tonality of the measure
            if (flags & 0x40) != 0 {
                log::debug!("Parsing key signature");
                let (inner, key_signature) = parse_i8(i)?;
                mh.key_signature.key = key_signature;
                i = inner;
                let (inner, is_minor) = parse_i8(i)?;
                i = inner;
                mh.key_signature.is_minor = is_minor != 0;
            }

            if (flags & 0x01) != 0 || (flags & 0x02) != 0 {
                log::debug!("Skip 4");
                i = skip(i, 4);
            }

            // Number of alternate ending
            if (flags & 0x10) != 0 {
                log::debug!("Parsing repeat alternative");
                let (inner, alternative) = parse_u8(i)?;
                i = inner;
                mh.repeat_alternative = alternative;
            }

            if (flags & 0x10) == 0 {
                log::debug!("Skip one");
                i = skip(i, 1);
            }

            // Triplet feel
            let (inner, triplet_feel) = parse_triplet_feel(i)?;
            i = inner;
            mh.triplet_feel = triplet_feel;
        } else if song_version <= GpVersion::GP4_06 {
            // Beginning of repeat
            if (flags & 0x08) != 0 {
                log::debug!("Parsing repeat close");
                let (inner, repeat_close) = parse_i8(i)?;
                i = inner;
                mh.repeat_close = repeat_close;
            }

            // Number of alternate ending
            if (flags & 0x10) != 0 {
                log::debug!("Parsing repeat alternative");
                let (inner, alternative) = parse_u8(i)?;
                i = inner;
                mh.repeat_alternative = alternative;
            }

            // Presence of a marker
            if (flags & 0x20) != 0 {
                let (inner, marker) = parse_marker(i)?;
                i = inner;
                mh.marker = Some(marker);
            }

            // Tonality of the measure
            if (flags & 0x40) != 0 {
                log::debug!("Parsing key signature");
                let (inner, key_signature) = parse_i8(i)?;
                mh.key_signature.key = key_signature;
                i = inner;
                let (inner, is_minor) = parse_i8(i)?;
                i = inner;
                mh.key_signature.is_minor = is_minor != 0;
            }
        }

        log::debug!("{mh:?}");

        Ok((i, mh))
    }
}

pub fn parse_measure_headers(
    measure_count: i32,
    song_tempo: u32,
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], Vec<MeasureHeader>> {
    move |i: &[u8]| {
        log::debug!("Parsing {measure_count} measure headers");
        // parse first header to account for the byte in between each header in GP5
        let (mut i, first_header) =
            parse_measure_header(TimeSignature::default(), song_tempo, version)(i)?;
        let mut previous_time_signature = first_header.time_signature.clone();
        let mut headers = vec![first_header];
        for _ in 1..measure_count {
            let (rest, header) = preceded(
                cond(version >= GpVersion::GP5, parse_u8),
                parse_measure_header(previous_time_signature, song_tempo, version),
            )
            .parse(i)?;
            // propagate time signature
            previous_time_signature = header.time_signature.clone();
            i = rest;
            headers.push(header);
        }
        debug_assert_eq!(headers.len(), measure_count as usize);
        Ok((i, headers))
    }
}

pub fn parse_midi_channels(i: &[u8]) -> IResult<&[u8], Vec<MidiChannel>> {
    log::debug!("Parsing midi channels");
    let mut channels = Vec::with_capacity(64);
    let mut i = i;
    for channel_index in 0..64 {
        let (inner, channel) = parse_midi_channel(channel_index)(i)?;
        i = inner;
        channels.push(channel);
    }
    Ok((i, channels))
}

pub fn parse_midi_channel(channel_id: i32) -> impl FnMut(&[u8]) -> IResult<&[u8], MidiChannel> {
    move |i: &[u8]| {
        map(
            (
                parse_int, parse_i8, parse_i8, parse_i8, parse_i8, parse_i8, parse_i8, parse_u8,
                parse_u8,
            ),
            |(
                mut instrument,
                volume,
                balance,
                chorus,
                reverb,
                phaser,
                tremolo,
                _blank,
                _blank2,
            )| {
                let bank = if channel_id == 9 {
                    DEFAULT_PERCUSSION_BANK
                } else {
                    DEFAULT_BANK
                };
                if instrument < 0 {
                    instrument = 0;
                }
                MidiChannel {
                    channel_id: channel_id as u8,
                    effect_channel_id: 0, // filled at the track level
                    instrument,
                    volume,
                    balance,
                    chorus,
                    reverb,
                    phaser,
                    tremolo,
                    bank,
                }
            },
        )
        .parse(i)
    }
}

pub fn parse_page_setup(i: &[u8]) -> IResult<&[u8], PageSetup> {
    log::debug!("Parsing page setup");
    map(
        (
            parse_point,
            parse_padding,
            parse_int,
            parse_short,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
            parse_int_sized_string,
        ),
        |(
            page_size,
            page_margin,
            score_size_proportion,
            header_and_footer,
            title,
            subtitle,
            artist,
            album,
            words,
            music,
            word_and_music,
            copyright_1,
            copyright_2,
            page_number,
        )| PageSetup {
            page_size,
            page_margin,
            score_size_proportion: score_size_proportion as f32 / 100.0,
            header_and_footer,
            title,
            subtitle,
            artist,
            album,
            words,
            music,
            word_and_music,
            copyright: format!("{copyright_1}\n{copyright_2}"),
            page_number,
        },
    )
    .parse(i)
}

pub fn parse_point(i: &[u8]) -> IResult<&[u8], Point> {
    log::debug!("Parsing point");
    map((parse_int, parse_int), |(x, y)| Point { x, y }).parse(i)
}

pub fn parse_padding(i: &[u8]) -> IResult<&[u8], Padding> {
    log::debug!("Parsing padding");
    map(
        (parse_int, parse_int, parse_int, parse_int),
        |(right, top, left, bottom)| Padding {
            right,
            top,
            left,
            bottom,
        },
    )
    .parse(i)
}

pub fn parse_lyrics(i: &[u8]) -> IResult<&[u8], Lyrics> {
    log::debug!("Parsing lyrics");
    map(
        (parse_int, count((parse_int, parse_int_sized_string), 5)),
        |(track_choice, lines)| Lyrics {
            track_choice,
            lines,
        },
    )
    .parse(i)
}

/// Parse the version string from the file header.
///
/// 30 character string (not counting the byte announcing the real length of the string)
///
/// <https://dguitar.sourceforge.net/GP4format.html#VERSIONS>
pub fn parse_gp_version(i: &[u8]) -> IResult<&[u8], GpVersion> {
    log::debug!("Parsing GP version");
    let (rest, version_string) = parse_byte_size_string(30)(i)?;
    let version = match version_string.as_str() {
        "FICHIER GUITAR PRO v3.00" => GpVersion::GP3,
        "FICHIER GUITAR PRO v4.00" => GpVersion::GP4,
        "FICHIER GUITAR PRO v4.06" => GpVersion::GP4_06,
        "FICHIER GUITAR PRO v5.00" => GpVersion::GP5,
        "FICHIER GUITAR PRO v5.10" => GpVersion::GP5_10,
        _ => {
            log::warn!("Unsupported GP version: {version_string}");
            return Err(nom::Err::Error(nom::error::Error::new(
                i,
                nom::error::ErrorKind::Tag,
            )));
        }
    };
    Ok((rest, version))
}

fn parse_notices(i: &[u8]) -> IResult<&[u8], Vec<String>> {
    flat_map(parse_int, |notice_count| {
        log::debug!("Notice count: {notice_count}");
        count(parse_int_byte_sized_string, notice_count as usize)
    })
    .parse(i)
}

/// Par information about the piece of music.
/// <https://dguitar.sourceforge.net/GP4format.html#Information_About_the_Piece>
fn parse_info(version: GpVersion) -> impl FnMut(&[u8]) -> IResult<&[u8], SongInfo> {
    move |i: &[u8]| {
        log::debug!("Parsing song info");
        map(
            (
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                cond(version >= GpVersion::GP5, parse_int_byte_sized_string),
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_int_byte_sized_string,
                parse_notices,
            ),
            |(
                name,
                subtitle,
                artist,
                album,
                author,
                words,
                copyright,
                writer,
                instructions,
                notices,
            )| {
                SongInfo {
                    name,
                    subtitle,
                    artist,
                    album,
                    author,
                    words,
                    copyright,
                    writer,
                    instructions,
                    notices,
                }
            },
        )
        .parse(i)
    }
}

pub fn parse_gp_data(file_data: &[u8]) -> Result<Song, RuxError> {
    // GPX (Guitar Pro 6) and GP7 files are containers, not the flat binary
    // format below; dispatch on the container magic.
    if file_data.starts_with(b"BCFS") || file_data.starts_with(b"BCFZ") {
        return crate::parser::gpx::song_builder::parse_gpx_data(file_data);
    }
    if file_data.starts_with(b"PK\x03\x04") {
        return crate::parser::gpx::song_builder::parse_gp7_data(file_data);
    }

    let (rest, base_song) = flat_map(parse_gp_version, |version| {
        map(
            (
                parse_info(version),                                     // Song info
                cond(version < GpVersion::GP5, parse_bool),              // Triplet feel
                cond(version >= GpVersion::GP4, parse_lyrics),           // Lyrics
                cond(version >= GpVersion::GP5_10, take(19usize)),       // Skip RSE master effect
                cond(version >= GpVersion::GP5, parse_page_setup),       // Page setup
                cond(version >= GpVersion::GP5, parse_int_sized_string), // Tempo name
                parse_int,                                               // Tempo value
                cond(version > GpVersion::GP5, parse_bool),              // Tempo hide
                parse_i8,                                                // Key signature
                take(3usize),                                            // unknown
                cond(version > GpVersion::GP3, parse_i8),                // Octave
                parse_midi_channels,                                     // Midi channels
            ),
            move |(
                song_info,
                triplet_feel,
                lyrics,
                _master_effect,
                page_setup,
                tempo_name,
                tempo,
                hide_tempo,
                key_signature,
                _unknown,
                octave,
                midi_channels,
            )| {
                // init base song
                let tempo = Tempo::new(tempo as u32, tempo_name);
                Song {
                    version,
                    song_info,
                    triplet_feel,
                    lyrics,
                    page_setup,
                    tempo,
                    hide_tempo,
                    key_signature,
                    octave,
                    midi_channels,
                    measure_headers: vec![],
                    tracks: vec![],
                }
            },
        )
    })
    .parse(file_data)
    .map_err(|_err| {
        log::error!("Failed to parse GP data");
        RuxError::ParsingError("Failed to parse GP data".to_string())
    })?;

    // make parser and parse music data
    let mut parser = MusicParser::new(base_song);
    let (_rest, ()) = parser.parse_music_data(rest).map_err(|e| {
        log::error!("Failed to parse music data: {e:?}");
        RuxError::ParsingError("Failed to parse music data".to_string())
    })?;
    let mut song = parser.take_song();

    // For GP4 and earlier, triplet feel is defined at the song level.
    // Propagate it to all measure headers so the MIDI builder can apply it.
    if song.version < GpVersion::GP5
        && let Some(true) = song.triplet_feel
    {
        for header in &mut song.measure_headers {
            header.triplet_feel = TripletFeel::Eighth;
        }
    }

    Ok(song)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gp_ordering() {
        assert!(GpVersion::GP4 < GpVersion::GP5);
        assert!(GpVersion::GP5 >= GpVersion::GP5);
        assert!(GpVersion::GP3 < GpVersion::GP4);
        assert!(GpVersion::GP3 < GpVersion::GP5);
    }
}
