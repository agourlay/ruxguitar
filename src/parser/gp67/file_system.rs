//! GPX (Guitar Pro 6) container decompression.
//!
//! A `.gpx` file is a small in-memory filesystem holding named entries, most
//! importantly `score.gpif` (the XML score). The container comes in two flavours:
//!
//! - `BCFS`: an uncompressed sector-based filesystem.
//! - `BCFZ`: a custom bit-level LZ compression that decompresses into a `BCFS` image.
//!
//! Port of Tuxguitar's `v6/GPXFileSystem`.

use crate::RuxError;
use crate::parser::gp67::bit_reader::BitReader;

/// `"BCFS"` little-endian magic.
const HEADER_BCFS: u32 = 0x5346_4342;
/// `"BCFZ"` little-endian magic.
const HEADER_BCFZ: u32 = 0x5A46_4342;

const SECTOR_SIZE: usize = 0x1000;
/// Marker value identifying a file entry sector in a BCFS image.
const FILE_ENTRY_MARKER: u32 = 2;
const MAX_FILE_NAME_LEN: usize = 127;

/// A named entry extracted from a GPX container.
struct GpxFile {
    name: String,
    contents: Vec<u8>,
}

pub struct GpxFileSystem {
    files: Vec<GpxFile>,
}

impl GpxFileSystem {
    /// Load and fully decode a GPX container from its raw bytes.
    pub fn load(data: &[u8]) -> Result<Self, RuxError> {
        let header = read_u32_le(data, 0)
            .ok_or_else(|| RuxError::ParsingError("GPX file too short for header".to_string()))?;
        let rest = &data[4..];

        let mut fs = GpxFileSystem { files: Vec::new() };
        match header {
            HEADER_BCFS => fs.load_bcfs(rest),
            HEADER_BCFZ => {
                let bcfs = decompress_bcfz(rest)?;
                // The decompressed image is itself a BCFS container: skip its 4-byte header.
                let inner = bcfs.get(4..).ok_or_else(|| {
                    RuxError::ParsingError("decompressed BCFZ too short".to_string())
                })?;
                fs.load_bcfs(inner);
            }
            _ => {
                return Err(RuxError::ParsingError("This is not a GPX file".to_string()));
            }
        }
        Ok(fs)
    }

    /// Return the raw contents of the named entry, if present.
    pub fn file_contents(&self, name: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.contents.as_slice())
    }

    /// Walk a BCFS image and collect its file entries.
    fn load_bcfs(&mut self, bytes: &[u8]) {
        let mut offset = 0;
        loop {
            offset += SECTOR_SIZE;
            if offset + 3 >= bytes.len() {
                break;
            }
            if read_u32_le(bytes, offset) != Some(FILE_ENTRY_MARKER) {
                continue;
            }

            let index_file_name = offset + 4;
            let index_file_size = offset + 0x8C;
            let index_of_block = offset + 0x94;

            let mut file_bytes = Vec::new();
            let mut block_count = 0;
            loop {
                let block = read_u32_le(bytes, index_of_block + (4 * block_count)).unwrap_or(0);
                block_count += 1;
                if block == 0 {
                    break;
                }
                let block_offset = (block as usize) * SECTOR_SIZE;
                // valid entries never reference sectors outside the image; a
                // zero-free block table would otherwise inflate file_bytes
                if block_offset >= bytes.len() {
                    break;
                }
                // The reference advances the scan cursor to the last data block
                // read; keep it monotonic so a backwards block pointer cannot
                // rescan the same file entry forever.
                offset = offset.max(block_offset);
                file_bytes
                    .extend_from_slice(slice_padded(bytes, block_offset, SECTOR_SIZE).as_slice());
            }

            let file_size = read_u32_le(bytes, index_file_size).unwrap_or(0) as usize;
            if file_bytes.len() >= file_size {
                let name = read_c_string(bytes, index_file_name, MAX_FILE_NAME_LEN);
                file_bytes.truncate(file_size);
                self.files.push(GpxFile {
                    name,
                    contents: file_bytes,
                });
            }
        }
    }
}

/// Decompressed BCFS images are a few MB in practice; reject absurd declared
/// sizes before allocating (the length field is attacker-controlled).
const MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024;

/// Decompress a BCFZ bitstream into the underlying BCFS image.
fn decompress_bcfz(data: &[u8]) -> Result<Vec<u8>, RuxError> {
    let mut reader = BitReader::new(data);
    let expected_length = read_u32_le(&reader.read_bytes(4), 0)
        .ok_or_else(|| RuxError::ParsingError("BCFZ stream too short".to_string()))?
        as usize;
    if expected_length > MAX_DECOMPRESSED_SIZE {
        return Err(RuxError::ParsingError(format!(
            "BCFZ declares implausible decompressed size: {expected_length} bytes"
        )));
    }

    let mut out: Vec<u8> = Vec::with_capacity(expected_length);
    // The output length bound deviates from TuxGuitar, which only bounds the
    // input offset: a back-reference can double the output for ~35 bits of
    // input, so a crafted stream could otherwise expand without limit.
    while !reader.end() && reader.byte_offset() < expected_length && out.len() < expected_length {
        let flag = reader.read_bits(1);
        if flag == 1 {
            // Back-reference into the already-decompressed output.
            let bits = reader.read_bits(4);
            let offs = reader.read_bits_reversed(bits) as usize;
            let size = reader.read_bits_reversed(bits) as usize;

            if offs == 0 || offs > out.len() {
                break;
            }
            let pos = out.len() - offs;
            let copy = size.min(offs).min(expected_length - out.len());
            for i in 0..copy {
                let byte = out[pos + i];
                out.push(byte);
            }
        } else {
            // Literal run.
            let size = reader.read_bits_reversed(2);
            for _ in 0..size {
                out.push(reader.read_bits(8) as u8);
            }
        }
    }
    Ok(out)
}

/// Read a little-endian `u32` at `offset`, or `None` if out of bounds.
fn read_u32_le(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Copy `length` bytes from `offset`, zero-padding past the end of `data`.
fn slice_padded(data: &[u8], offset: usize, length: usize) -> Vec<u8> {
    let mut out = vec![0u8; length];
    for (i, slot) in out.iter_mut().enumerate() {
        if let Some(b) = data.get(offset + i) {
            *slot = *b;
        }
    }
    out
}

/// Read a NUL-terminated latin1 string of at most `max_len` bytes.
fn read_c_string(data: &[u8], offset: usize, max_len: usize) -> String {
    let mut s = String::new();
    for i in 0..max_len {
        match data.get(offset + i) {
            Some(0) | None => break,
            Some(&b) => s.push(b as char),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only fixtures committed to the repo (test-files/ is otherwise gitignored).
    const FIXTURES: &[&str] = &["test-files/Tyr - Evening Star.gpx"];

    /// MSB-first bit writer mirroring the `BitReader` layout, to craft BCFZ streams.
    struct BitWriter {
        bytes: Vec<u8>,
        bit: usize,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                bit: 0,
            }
        }

        fn push_bit(&mut self, bit: u32) {
            if self.bit == 0 {
                self.bytes.push(0);
            }
            *self.bytes.last_mut().unwrap() |= ((bit & 1) as u8) << (7 - self.bit);
            self.bit = (self.bit + 1) % 8;
        }

        /// Counterpart of `BitReader::read_bits`.
        fn push_bits(&mut self, value: u32, count: u32) {
            for i in (0..count).rev() {
                self.push_bit((value >> i) & 1);
            }
        }

        /// Counterpart of `BitReader::read_bits_reversed`.
        fn push_bits_reversed(&mut self, value: u32, count: u32) {
            for i in 0..count {
                self.push_bit((value >> i) & 1);
            }
        }

        fn push_literal_run(&mut self, bytes: &[u8]) {
            self.push_bit(0);
            self.push_bits_reversed(bytes.len() as u32, 2);
            for &b in bytes {
                self.push_bits(u32::from(b), 8);
            }
        }

        fn push_back_reference(&mut self, offs: u32, size: u32) {
            self.push_bit(1);
            self.push_bits(4, 4); // 4-bit wide offs/size fields
            self.push_bits_reversed(offs, 4);
            self.push_bits_reversed(size, 4);
        }
    }

    #[test]
    fn bcfz_rejects_implausible_declared_size() {
        // u32::MAX declared size must error out instead of allocating 4 GiB
        let data = [0xFF, 0xFF, 0xFF, 0xFF];
        assert!(decompress_bcfz(&data).is_err());
    }

    #[test]
    fn bcfz_output_is_bounded_by_declared_size() {
        // declared size 14, but doubling back-references try to copy past it
        let mut w = BitWriter::new();
        w.push_bits(14, 8); // expected_length = 14, little-endian
        w.push_bits(0, 8);
        w.push_bits(0, 8);
        w.push_bits(0, 8);
        w.push_literal_run(b"ab");
        w.push_literal_run(b"cd");
        // each back-reference doubles the output (offs = whole output so far);
        // the second one would reach 16 bytes without the output clamp
        w.push_back_reference(4, 15);
        w.push_back_reference(8, 15);

        let out = decompress_bcfz(&w.bytes).unwrap();
        assert_eq!(out, b"abcdabcdabcdab");
    }

    #[test]
    fn extracts_score_gpif_from_all_fixtures() {
        for path in FIXTURES {
            let data = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
            let fs = GpxFileSystem::load(&data).unwrap_or_else(|e| panic!("load {path}: {e}"));

            let score = fs
                .file_contents("score.gpif")
                .unwrap_or_else(|| panic!("no score.gpif in {path}"));

            // The score is a GPIF XML document.
            assert!(!score.is_empty(), "empty score.gpif in {path}");
            let text = String::from_utf8_lossy(score);
            assert!(
                text.contains("<GPIF") || text.contains("<?xml"),
                "score.gpif in {path} does not look like XML: starts with {:?}",
                &text[..text.len().min(64)]
            );
        }
    }
}
