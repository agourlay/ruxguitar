use encoding_rs::WINDOWS_1252;
use nom::combinator::{flat_map, map};
use nom::{IResult, Parser, bytes, number};

pub fn parse_i8(i: &[u8]) -> IResult<&[u8], i8> {
    number::complete::le_i8(i)
}

pub fn parse_u8(i: &[u8]) -> IResult<&[u8], u8> {
    number::complete::le_u8(i)
}

pub fn parse_int(i: &[u8]) -> IResult<&[u8], i32> {
    number::complete::le_i32(i)
}

pub fn parse_bool(i: &[u8]) -> IResult<&[u8], bool> {
    map(number::complete::le_u8, |b| b == 1).parse(i)
}

pub fn parse_short(i: &[u8]) -> IResult<&[u8], i16> {
    number::complete::le_i16(i)
}

/// Skip `n` bytes, stopping at the end of input.
///
/// Lenient on truncated input: skipping past the end yields an empty input
/// instead of a panic, so the next actual read reports a proper parse error.
pub fn skip(i: &[u8], n: usize) -> &[u8] {
    log::debug!("skip: {n}");
    i.get(n..).unwrap_or(&[])
}

/// Materialize properly encoded String
fn make_string(i: &[u8]) -> String {
    let (cow, encoding_used, had_errors) = WINDOWS_1252.decode(i);
    if had_errors {
        log::debug!("Error parsing string with {encoding_used:?}");
        match std::str::from_utf8(i) {
            Ok(s) => s.to_string(),
            Err(e) => {
                log::debug!("Error UTF-8 string parsing:{e}");
                String::new()
            }
        }
    } else {
        cow.to_string()
    }
}

/// Parse string of length `len`; a negative length reads nothing.
fn parse_string(len: i32) -> impl FnMut(&[u8]) -> IResult<&[u8], String> {
    let len = usize::try_from(len).unwrap_or(0);
    parse_string_field(len, len)
}

/// Parse string field of length `string_len` with total size to consume `field_size`
fn parse_string_field(
    field_size: usize,
    string_len: usize,
) -> impl FnMut(&[u8]) -> IResult<&[u8], String> {
    move |i: &[u8]| {
        log::debug!("Parsing string field: field_size={field_size}, string_len={string_len}");

        // Read exactly the field size
        let (rest, field) = bytes::complete::take(field_size)(i)?;

        log::debug!("Raw field raw={field:02X?}");

        // Decode only the meaningful string bytes
        let string = make_string(&field[..std::cmp::min(string_len, field_size)]);

        Ok((rest, string))
    }
}

/// Size of string encoded as Int.
/// [i32 string_len][size bytes field]
pub fn parse_int_sized_string(i: &[u8]) -> IResult<&[u8], String> {
    flat_map(parse_int, parse_string).parse(i)
}

/// Size of Strings provided
/// `size`:   real string length
/// `length`: optional provided length (in case of blank chars after the string)
pub fn parse_byte_size_string(size: usize) -> impl FnMut(&[u8]) -> IResult<&[u8], String> {
    move |i: &[u8]| {
        let (i, length) = parse_u8(i)?;
        log::debug!("Parsing byte sized string of length {length} for String size {size}");
        parse_string_field(size, length as usize)(i)
    }
}

/// String whose field size is encoded as an Int (plus one), followed by its
/// length as a byte: [i32 field_size + 1][u8 string_len][field_size bytes].
///
/// Like TuxGuitar's `readStringByteSizeOfInteger`, when the Int gives no field
/// size (0 or 1) the field is as long as the string.
pub fn parse_int_byte_sized_string(i: &[u8]) -> IResult<&[u8], String> {
    flat_map(parse_int, |len| {
        flat_map(parse_u8, move |str_len| {
            log::debug!("Parsing int byte sized string int_len={len} u8_len={str_len}");
            let field_size = usize::try_from(len.saturating_sub(1))
                .ok()
                .filter(|&size| size > 0)
                .unwrap_or(usize::from(str_len));
            parse_string_field(field_size, usize::from(str_len))
        })
    })
    .parse(i)
}

#[cfg(test)]
mod tests {
    use crate::parser::gp345::primitive_parser::{
        parse_byte_size_string, parse_int_byte_sized_string, parse_int_sized_string, skip,
    };

    /// An int byte sized string: the Int, the length byte, then the field.
    fn int_byte_sized(int: i32, str_len: u8, field: &[u8]) -> Vec<u8> {
        let mut data = int.to_le_bytes().to_vec();
        data.push(str_len);
        data.extend_from_slice(field);
        data
    }

    #[test]
    fn an_int_byte_sized_string_reads_its_whole_field() {
        // a field of 4 bytes holding a 3 char string, then the next value
        let data = int_byte_sized(5, 3, b"abc\0\x7F");
        let (rest, string) = parse_int_byte_sized_string(&data).unwrap();
        assert_eq!(string, "abc");
        assert_eq!(rest, [0x7F]);
    }

    #[test]
    fn an_int_byte_sized_string_without_field_size_reads_its_length() {
        // like TuxGuitar, an Int of 1 or 0 leaves the size to the length byte
        for int in [1, 0, -3] {
            let data = int_byte_sized(int, 3, b"abc\x7F");
            let (rest, string) = parse_int_byte_sized_string(&data).unwrap();
            assert_eq!(string, "abc", "int {int}");
            assert_eq!(rest, [0x7F], "int {int}");
        }
    }

    #[test]
    fn an_int_byte_sized_string_longer_than_its_field_is_cut() {
        let data = int_byte_sized(3, 5, b"ab\x7F");
        let (rest, string) = parse_int_byte_sized_string(&data).unwrap();
        assert_eq!(string, "ab");
        assert_eq!(rest, [0x7F]);
    }

    #[test]
    fn a_negative_int_sized_string_is_empty() {
        let mut data = (-1_i32).to_le_bytes().to_vec();
        data.push(0x7F);
        let (rest, string) = parse_int_sized_string(&data).unwrap();
        assert_eq!(string, "");
        assert_eq!(rest, [0x7F]);
    }

    #[test]
    fn skip_is_lenient_on_truncated_input() {
        let data = [1_u8, 2, 3];
        assert_eq!(skip(&data, 2), &[3]);
        assert_eq!(skip(&data, 3), &[] as &[u8]); // exact boundary
        assert_eq!(skip(&data, 4), &[] as &[u8]); // truncated input: no panic
        assert_eq!(skip(&[], 4), &[] as &[u8]);
    }

    #[test]
    fn test_read_byte_size_string() {
        let data: Vec<u8> = vec![
            0x18, 0x46, 0x49, 0x43, 0x48, 0x49, 0x45, 0x52, 0x20, 0x47, 0x55, 0x49, 0x54, 0x41,
            0x52, 0x20, 0x50, 0x52, 0x4f, 0x20, 0x76, 0x33, 0x2e, 0x30, 0x30, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00,
        ];
        let (_rest, res) = parse_byte_size_string(30)(&data).unwrap();
        assert_eq!(res, "FICHIER GUITAR PRO v3.00");
    }
}
