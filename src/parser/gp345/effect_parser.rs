//! Parsers for chords and for note and beat effects.

use crate::parser::gp345::primitive_parser::{
    parse_byte_size_string, parse_i8, parse_int, parse_int_byte_sized_string, parse_u8, skip,
};
use nom::IResult;
use nom::Parser;
use nom::combinator::map;

use crate::parser::model::*;

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
                if !bend_effect.points.is_empty() {
                    note.effect.bend = Some(bend_effect);
                }
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
            if !bend_effect.points.is_empty() {
                note.effect.bend = Some(bend_effect);
            }
        }

        if (flags1 & 0x10) != 0 {
            let (inner, grace_effect) = parse_grace_effect(version)(i)?;
            i = inner;
            note.effect.grace = Some(grace_effect);
        }

        if (flags2 & 0x04) != 0 {
            let (inner, tremolo_picking) = parse_tremolo_picking(i)?;
            i = inner;
            note.effect.tremolo_picking = tremolo_picking;
        }

        if (flags2 & 0x08) != 0 {
            let (inner, slide_type) = parse_slide_type(version)(i)?;
            i = inner;
            note.effect.slide = slide_type;
        }

        if (flags2 & 0x10) != 0 {
            let (inner, harmonic_effect) = parse_harmonic_effect(version)(i)?;
            i = inner;
            note.effect.harmonic = harmonic_effect;
        }

        if (flags2 & 0x20) != 0 {
            let (inner, trill_effect) = parse_trill_effect(i)?;
            i = inner;
            note.effect.trill = trill_effect;
        }

        Ok((i, ()))
    }
}

pub fn parse_trill_effect(i: &[u8]) -> IResult<&[u8], Option<TrillEffect>> {
    log::debug!("Parsing trill effect");
    let (inner, (fret, period)) = (parse_i8, parse_i8).parse(i)?;
    // like TuxGuitar, an unknown period drops the trill
    let trill_effect = TrillEffect::from_trill_period(period).map(|value| {
        let mut trill_effect = TrillEffect {
            fret,
            ..Default::default()
        };
        trill_effect.duration.value = value;
        trill_effect
    });
    Ok((inner, trill_effect))
}

pub fn parse_harmonic_effect(
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], Option<HarmonicEffect>> {
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
            // pre-GP5 artificial harmonic variants, with the key offset
            // TuxGuitar assigns them (+24, +19 and +12 respectively)
            15 => {
                he.kind = HarmonicType::Artificial;
                he.data = 2;
            }
            17 => {
                he.kind = HarmonicType::Artificial;
                he.data = 3;
            }
            22 => he.kind = HarmonicType::Artificial,
            // like TuxGuitar, unknown types drop the harmonic
            x => {
                log::warn!("Unknown harmonic type {x}");
                return Ok((i, None));
            }
        };
        Ok((i, Some(he)))
    }
}

pub fn parse_slide_type(
    version: GpVersion,
) -> impl FnMut(&[u8]) -> IResult<&[u8], Option<SlideType>> {
    move |i| {
        map(parse_i8, |t| {
            log::debug!("Parsing slide type {t}");
            if version >= GpVersion::GP5 {
                // GP5 stores a bitmask
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
            } else {
                // GP4 stores an ordinal
                match t {
                    -2 => Some(SlideType::IntoFromAbove),
                    -1 => Some(SlideType::IntoFromBelow),
                    1 => Some(SlideType::ShiftSlideTo),
                    2 => Some(SlideType::LegatoSlideTo),
                    3 => Some(SlideType::OutDownwards),
                    4 => Some(SlideType::OutUpWards),
                    _ => None,
                }
            }
        })
        .parse(i)
    }
}

pub fn parse_tremolo_picking(i: &[u8]) -> IResult<&[u8], Option<TremoloPickingEffect>> {
    log::debug!("Parsing tremolo picking");
    map(parse_u8, |tp| {
        // like TuxGuitar, an unknown value drops the tremolo picking
        TremoloPickingEffect::from_tremolo_value(tp as i8).map(|value| {
            let mut tremolo_picking_effect = TremoloPickingEffect::default();
            tremolo_picking_effect.duration.value = value;
            tremolo_picking_effect
        })
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
            if !effect.points.is_empty() {
                note_effect.tremolo_bar = Some(effect);
            }
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

        // pick stroke direction
        if flags2 & 0x02 != 0 {
            let (inner, direction) = parse_u8(i)?;
            i = inner;
            if direction & 0x01 != 0 {
                beat.effect.pick_stroke = BeatStrokeDirection::Up;
            } else if direction & 0x02 != 0 {
                beat.effect.pick_stroke = BeatStrokeDirection::Down;
            }
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
        let point_value = value as f32 / (GP_BEND_SEMITONE * 2.0);
        tremolo_bar_effect.points.push(BendPoint {
            position: point_position.round() as u8,
            value: point_value.round() as i8,
        });
    }
    Ok((i, tremolo_bar_effect))
}
