//! MSB-first bit reader over a byte slice.
//!
//! Port of Tuxguitar's `GPXByteBuffer`, used to decode the BCFZ bitstream.

const BITS_PER_BYTE: usize = 8;

pub struct BitReader<'a> {
    buffer: &'a [u8],
    /// Current read position, in bits.
    position: usize,
}

impl<'a> BitReader<'a> {
    pub const fn new(buffer: &'a [u8]) -> Self {
        BitReader {
            buffer,
            position: 0,
        }
    }

    /// Current read position expressed in bytes (rounded down).
    pub const fn byte_offset(&self) -> usize {
        self.position / BITS_PER_BYTE
    }

    /// Whether the read position has reached the end of the buffer.
    pub const fn end(&self) -> bool {
        self.byte_offset() >= self.buffer.len()
    }

    /// Read a single bit (most significant bit first).
    ///
    /// Returns `None` when reading past the end of the buffer, without advancing.
    fn read_bit(&mut self) -> Option<u32> {
        let byte_index = self.position / BITS_PER_BYTE;
        if byte_index >= self.buffer.len() {
            return None;
        }
        let byte_bit = (BITS_PER_BYTE - 1) - (self.position % BITS_PER_BYTE);
        let bit = (u32::from(self.buffer[byte_index]) >> byte_bit) & 0x01;
        self.position += 1;
        Some(bit)
    }

    /// Read `count` bits, most significant bit first.
    pub fn read_bits(&mut self, count: u32) -> u32 {
        let mut bits = 0;
        for i in (0..count).rev() {
            bits |= self.read_bit().unwrap_or(0) << i;
        }
        bits
    }

    /// Read `count` bits, least significant bit first.
    pub fn read_bits_reversed(&mut self, count: u32) -> u32 {
        let mut bits = 0;
        for i in 0..count {
            bits |= self.read_bit().unwrap_or(0) << i;
        }
        bits
    }

    /// Read `count` whole bytes from the bitstream.
    pub fn read_bytes(&mut self, count: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(count);
        for _ in 0..count {
            bytes.push(self.read_bits(8) as u8);
        }
        bytes
    }
}
