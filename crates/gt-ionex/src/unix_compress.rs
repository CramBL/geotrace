//! Decoding of the Unix `compress` format, which CDDIS serves its legacy
//! IONEX files in.
//!
//! A stream is LZW over codes packed low bit first, starting nine bits wide
//! and growing to the limit its header declares. In block mode the encoder
//! restarts the string table with code 256, and every restart and every width
//! change is preceded by padding out to a whole multiple of the code width in
//! bytes.

use std::ops::RangeInclusive;

use thiserror::Error;

/// Bytes every stream starts with, followed by the flags byte.
const MAGIC: [u8; 2] = [0x1f, 0x9d];

const HEADER_LEN: usize = 3;

/// Flags bit set when the encoder may restart the table with [`CLEAR_CODE`].
const BLOCK_MODE_FLAG: u8 = 0x80;

/// Flags bits holding the widest code the stream reaches.
const CODE_WIDTH_LIMIT_MASK: u8 = 0x1f;

const INITIAL_CODE_WIDTH: u32 = 9;

/// Code widths the format defines. A stream declaring anything else was not
/// written by an encoder GeoTrace can read.
const CODE_WIDTH_LIMITS: RangeInclusive<u32> = 9..=16;

/// Codes standing for a single byte, which every table starts with.
const BYTE_CODES: u16 = 256;

/// Restarts the string table, in block mode only.
const CLEAR_CODE: u16 = 256;

/// Most bytes one stream may decode to.
///
/// A published IONEX day is under 1 MB decompressed. The format lets a short
/// stream expand into orders of magnitude more, so the decode stops at this
/// instead of growing the output until the allocator fails.
pub const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UnixCompressError {
    #[error("not a compress stream: the 0x1f 0x9d magic is missing")]
    NotCompressed,

    #[error(
        "the stream declares {max_code_width} bit codes, outside the 9 to 16 bits the format defines"
    )]
    UnsupportedCodeWidthLimit { max_code_width: u8 },

    #[error("code {code} is not in the string table")]
    UndefinedCode { code: u16 },

    #[error("the stream decodes to more than {MAX_DECOMPRESSED_BYTES} bytes")]
    TooLarge,
}

/// The bytes `compressed` stands for.
///
/// A stream that ends mid-string decodes to what it did hold: the format
/// records no length, so a truncated file is indistinguishable from a short
/// one. The caller's own parser is what rejects the partial content.
pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>, UnixCompressError> {
    let header: [u8; HEADER_LEN] = compressed
        .get(..HEADER_LEN)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(UnixCompressError::NotCompressed)?;
    let [magic_first, magic_second, flags] = header;
    if [magic_first, magic_second] != MAGIC {
        return Err(UnixCompressError::NotCompressed);
    }
    let max_code_width = u32::from(flags & CODE_WIDTH_LIMIT_MASK);
    if !CODE_WIDTH_LIMITS.contains(&max_code_width) {
        return Err(UnixCompressError::UnsupportedCodeWidthLimit {
            max_code_width: flags & CODE_WIDTH_LIMIT_MASK,
        });
    }
    let block_mode = flags & BLOCK_MODE_FLAG != 0;
    let payload = compressed.get(HEADER_LEN..).unwrap_or_default();

    let mut decoder = Decoder::new(block_mode, max_code_width);
    let mut previous: Option<u16> = None;
    let mut code_width = INITIAL_CODE_WIDTH;
    let mut bit_pos = 0_usize;
    let mut group_start_bits = 0_usize;
    let total_bits = payload.len().saturating_mul(8);

    loop {
        if code_width < max_code_width && decoder.table.next_code() > highest_code(code_width) {
            bit_pos = padded_past(bit_pos, group_start_bits, code_width);
            group_start_bits = bit_pos;
            code_width = code_width.saturating_add(1);
        }
        if bit_pos.saturating_add(code_width as usize) > total_bits {
            return Ok(decoder.output);
        }
        let code = code_at(payload, bit_pos, code_width);
        bit_pos = bit_pos.saturating_add(code_width as usize);

        if block_mode && code == CLEAR_CODE {
            bit_pos = padded_past(bit_pos, group_start_bits, code_width);
            group_start_bits = bit_pos;
            code_width = INITIAL_CODE_WIDTH;
            decoder.table.clear();
            previous = None;
            continue;
        }

        match previous {
            // The first code of a stream, and the first after a restart,
            // stands for a single byte.
            None => {
                if code >= BYTE_CODES {
                    return Err(UnixCompressError::UndefinedCode { code });
                }
                decoder.append_string(code)?;
            }
            Some(previous_code) => {
                let first_byte = if u32::from(code) == decoder.table.next_code() {
                    // The code names the string this step is about to add,
                    // which the encoder emitted before the decoder had it.
                    let first_byte = decoder.append_string(previous_code)?;
                    decoder.output.push(first_byte);
                    first_byte
                } else {
                    decoder.append_string(code)?
                };
                decoder.table.push(previous_code, first_byte);
            }
        }
        if decoder.output.len() > MAX_DECOMPRESSED_BYTES {
            return Err(UnixCompressError::TooLarge);
        }
        previous = Some(code);
    }
}

/// Widest code `code_width` bits address.
fn highest_code(code_width: u32) -> u32 {
    (1_u32 << code_width.min(u32::BITS.saturating_sub(1))).saturating_sub(1)
}

/// The bit position the next code group starts at, counting from
/// `group_start_bits`: the encoder writes codes eight at a time and pads the
/// last group out to `code_width` whole bytes before it changes width or
/// restarts the table.
fn padded_past(bit_pos: usize, group_start_bits: usize, code_width: u32) -> usize {
    let group_bits = (code_width as usize).saturating_mul(8);
    let within_group = bit_pos.saturating_sub(group_start_bits);
    group_start_bits.saturating_add(within_group.next_multiple_of(group_bits))
}

/// The `code_width` bits at `bit_pos`, low bit first. Bits past the end of
/// `payload` read as zero, which only the caller's end-of-stream check reaches.
fn code_at(payload: &[u8], bit_pos: usize, code_width: u32) -> u16 {
    let byte_index = bit_pos / 8;
    let bit_offset = bit_pos % 8;
    let mut window: u32 = 0;
    for (position, byte) in payload
        .get(byte_index..)
        .into_iter()
        .flatten()
        .take(3)
        .enumerate()
    {
        window |= u32::from(*byte) << (position * 8);
    }
    ((window >> bit_offset) & highest_code(code_width)) as u16
}

/// The output being built and the strings its codes stand for.
struct Decoder {
    table: StringTable,
    /// The string of the code being appended, last byte first.
    reversed: Vec<u8>,
    output: Vec<u8>,
}

impl Decoder {
    fn new(block_mode: bool, max_code_width: u32) -> Self {
        // Block mode reserves 256 for the restart, so its first string sits
        // one code higher.
        let first_code = if block_mode {
            BYTE_CODES.saturating_add(1)
        } else {
            BYTE_CODES
        };
        Self {
            table: StringTable::new(first_code, highest_code(max_code_width)),
            reversed: Vec::new(),
            output: Vec::new(),
        }
    }

    /// Append the string `code` stands for, and report its first byte.
    fn append_string(&mut self, code: u16) -> Result<u8, UnixCompressError> {
        self.table.expand(code, &mut self.reversed)?;
        let first_byte = *self
            .reversed
            .last()
            .ok_or(UnixCompressError::UndefinedCode { code })?;
        self.output.extend(self.reversed.iter().rev());
        Ok(first_byte)
    }
}

/// The strings the codes above the single bytes stand for, each one an earlier
/// code and the byte appended to it.
struct StringTable {
    entries: Vec<(u16, u8)>,
    first_code: u16,
    highest_code: u32,
}

impl StringTable {
    fn new(first_code: u16, highest_code: u32) -> Self {
        Self {
            entries: Vec::new(),
            first_code,
            highest_code,
        }
    }

    /// The code the next string added takes.
    fn next_code(&self) -> u32 {
        u32::from(self.first_code).saturating_add(self.entries.len() as u32)
    }

    /// Add the string `prefix` stands for with `suffix` appended, unless every
    /// code the declared width addresses is taken.
    fn push(&mut self, prefix: u16, suffix: u8) {
        if self.next_code() <= self.highest_code {
            self.entries.push((prefix, suffix));
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    /// Write the string `code` stands for into `into`, last byte first.
    fn expand(&self, code: u16, into: &mut Vec<u8>) -> Result<(), UnixCompressError> {
        into.clear();
        let mut current = code;
        // Every entry names a code added before it, so the walk is finite.
        // The bound holds even for a table that somehow says otherwise.
        for _ in 0..=self.entries.len() {
            if current < BYTE_CODES {
                into.push(current as u8);
                return Ok(());
            }
            let index = usize::from(current).checked_sub(usize::from(self.first_code));
            let &(prefix, suffix) = index
                .and_then(|index| self.entries.get(index))
                .ok_or(UnixCompressError::UndefinedCode { code })?;
            into.push(suffix);
            current = prefix;
        }
        Err(UnixCompressError::UndefinedCode { code })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// A stream of `codes`, packed as the format writes nine-bit ones, which
    /// is every code of a stream this short.
    fn nine_bit_stream(flags: u8, codes: &[u16]) -> Vec<u8> {
        let mut bytes = vec![MAGIC[0], MAGIC[1], flags];
        let mut accumulator: u32 = 0;
        let mut filled: u32 = 0;
        for &code in codes {
            accumulator |= u32::from(code) << filled;
            filled = filled.saturating_add(INITIAL_CODE_WIDTH);
            while filled >= 8 {
                bytes.push(accumulator as u8);
                accumulator >>= 8;
                filled = filled.saturating_sub(8);
            }
        }
        if filled > 0 {
            bytes.push(accumulator as u8);
        }
        bytes
    }

    const BLOCK_MODE_AT_SIXTEEN_BITS: u8 = BLOCK_MODE_FLAG | 16;

    #[rstest]
    #[case::empty(Vec::new(), UnixCompressError::NotCompressed)]
    #[case::only_the_magic(vec![0x1f, 0x9d], UnixCompressError::NotCompressed)]
    #[case::another_format(vec![0x1f, 0x8b, 0x08], UnixCompressError::NotCompressed)]
    #[case::below_the_narrowest_code(
        vec![0x1f, 0x9d, BLOCK_MODE_FLAG | 8],
        UnixCompressError::UnsupportedCodeWidthLimit { max_code_width: 8 }
    )]
    #[case::past_the_widest_code(
        vec![0x1f, 0x9d, BLOCK_MODE_FLAG | 17],
        UnixCompressError::UnsupportedCodeWidthLimit { max_code_width: 17 }
    )]
    fn a_stream_the_decoder_cannot_read_names_why(
        #[case] compressed: Vec<u8>,
        #[case] expected: UnixCompressError,
    ) {
        assert_eq!(decompress(&compressed), Err(expected));
    }

    #[test]
    fn a_stream_of_nothing_but_a_header_decodes_to_nothing() {
        assert_eq!(
            decompress(&[0x1f, 0x9d, BLOCK_MODE_AT_SIXTEEN_BITS]),
            Ok(Vec::new())
        );
    }

    /// The first code of a stream stands for a single byte: the table holds
    /// nothing else yet.
    #[test]
    fn a_first_code_above_the_single_bytes_is_rejected() {
        assert_eq!(
            decompress(&nine_bit_stream(BLOCK_MODE_AT_SIXTEEN_BITS, &[300])),
            Err(UnixCompressError::UndefinedCode { code: 300 })
        );
    }

    #[test]
    fn a_code_the_table_does_not_hold_is_rejected() {
        assert_eq!(
            decompress(&nine_bit_stream(
                BLOCK_MODE_AT_SIXTEEN_BITS,
                &[b'A'.into(), 400]
            )),
            Err(UnixCompressError::UndefinedCode { code: 400 })
        );
    }

    /// The encoder may emit the code of the string the decoder is about to
    /// add, which stands for the previous string with its own first byte
    /// appended.
    #[test]
    fn a_code_naming_the_string_being_added_repeats_its_first_byte() {
        assert_eq!(
            decompress(&nine_bit_stream(
                BLOCK_MODE_AT_SIXTEEN_BITS,
                &[b'A'.into(), 257]
            )),
            Ok(b"AAA".to_vec())
        );
    }

    /// Without block mode there is no restart code, so 256 is the first
    /// string the table holds.
    #[test]
    fn without_block_mode_the_first_string_takes_the_restart_code() {
        assert_eq!(
            decompress(&nine_bit_stream(16, &[b'A'.into(), 256])),
            Ok(b"AAA".to_vec())
        );
    }
}
