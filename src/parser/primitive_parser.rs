use encoding_rs::WINDOWS_1252;
use nom::combinator::{flat_map, map};
use nom::{bytes, number, IResult, Parser};

/// Parse signed bytes
pub fn parse_i8(i: &[u8]) -> IResult<&[u8], i8> {
    number::complete::le_i8(i)
}

/// Parse unsigned bytes
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

/// Skip `n` bytes.
pub fn skip(i: &[u8], n: usize) -> &[u8] {
    if i.is_empty() {
        return i;
    }
    &i[n..]
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

/// Parse string of length `len`.
fn parse_string(len: i32) -> impl FnMut(&[u8]) -> IResult<&[u8], String> {
    parse_string_field(len as usize, len as usize)
}

/// Parse string field of length `string_len` with total size to consume `field_size`
fn parse_string_field(
    field_size: usize,
    string_len: usize,
) -> impl FnMut(&[u8]) -> IResult<&[u8], String> {
    move |i: &[u8]| {
        log::debug!("Parsing string field: string_len={string_len}, field_size={field_size},");

        if field_size == 0 {
            return Ok((i, String::new()));
        }

        assert!(string_len <= field_size);

        // Read exactly the field size
        let (rest, field) = bytes::complete::take(field_size)(i)?;

        // Decode only the meaningful string bytes
        let string = make_string(&field[..string_len]);

        Ok((rest, string))
    }
}

/// Size of string encoded as Int.
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

/// Size of string encoded as Byte.
#[allow(unused)]
pub fn parse_byte_sized_string(i: &[u8]) -> IResult<&[u8], String> {
    flat_map(parse_u8, |str_len| parse_string(i32::from(str_len))).parse(i)
}

/// Size of string encoded as Int, but the size is encoded as a byte.
pub fn parse_int_byte_sized_string(i: &[u8]) -> IResult<&[u8], String> {
    log::debug!("Parsing int byte sized string");
    flat_map(parse_int, |len| {
        flat_map(parse_i8, move |str_len| {
            if str_len < 0 {
                log::info!("Negative string length: {str_len}");
                parse_string(len - 1)
            } else {
                assert_eq!(len - 1, i32::from(str_len), "String length mismatch");
                parse_string(i32::from(str_len))
            }
        })
    })
    .parse(i)
}

#[cfg(test)]
mod tests {
    use crate::parser::primitive_parser::parse_byte_size_string;

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
