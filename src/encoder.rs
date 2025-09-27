//! Arithmetic range coder that encodes one bit at a time using 12-bit probabilities.
//! Encodes in blocks with headers: mode ('c'), usize (4B BE), csize (4B BE).

use crate::predictor::Predictor;
use crate::lzp::LZP;
use crate::util::Progress;
use std::io::{Read, Write};
use anyhow::Result;

pub enum Mode {
    Compress,
    #[allow(dead_code)]
    Decompress,
}

// Increased to 2 MiB to reduce block overhead and improve throughput.
pub const BUFSIZE: usize = 0x200_000; // 2 MiB

/// Encoder encapsulates range coder and block buffering.
pub struct Encoder<'a, R: Read, W: Write> {
    mode: Mode,
    r: Option<&'a mut R>,
    #[allow(dead_code)]
    w: Option<&'a mut W>,
    x1: u32,
    x2: u32,
    x: u32,
    // block buffers
    buf: Vec<u8>,
    usize_in_blk: u32,
    csize_in_blk: usize,
    // global accumulators
    usum: f64,
    csum: f64,
    progress: Progress,
}

impl<'a, R: Read, W: Write> Encoder<'a, R, W> {
    #[inline(always)]
    pub fn new_compress(progress: Progress) -> Self {
        Self {
            mode: Mode::Compress,
            r: None,
            w: None, // external writer is passed to flush()
            x1: 0,
            x2: 0xFFFF_FFFF,
            x: 0,
            buf: Vec::with_capacity(BUFSIZE),
            usize_in_blk: 0,
            csize_in_blk: 0,
            usum: 0.0,
            csum: 0.0,
            progress,
        }
    }

    #[allow(dead_code)]
    pub fn new_decompress(r: &'a mut R) -> Result<Self> {
        let mut x = 0u32;
        // Read first 4 bytes of bitstream for x
        let mut hdr = [0u8; 4];
        r.read_exact(&mut hdr)?;
        for b in hdr {
            x = (x << 8) | (b as u32);
        }
        Ok(Self {
            mode: Mode::Decompress,
            r: Some(r),
            w: None,
            x1: 0,
            x2: 0xFFFF_FFFF,
            x,
            buf: Vec::new(),
            usize_in_blk: 0,
            csize_in_blk: 4,
            usum: 0.0,
            csum: 0.0,
            progress: Progress::none(),
        })
    }

    /// Accessor for pending compressed bytes in the current block buffer.
    #[inline(always)]
    pub fn pending_bytes(&self) -> usize {
        self.csize_in_blk
    }

    /// Compress bit `y` or return decompressed bit.
    #[inline(always)]
    pub fn code(&mut self, predictor: &mut Predictor, lzp: &mut LZP, y_in: u8) -> Result<u8> {
        let p = predictor.p(lzp); // 0..4095
        let p_adj = p + ((p < 2048) as i32);
        let range = self.x2.wrapping_sub(self.x1);
        let xmid = self.x1
            .wrapping_add((range >> 12).wrapping_mul(p_adj as u32)
            .wrapping_add(((range & 0xFFF).wrapping_mul(p_adj as u32)) >> 12));
        let y_out = match self.mode {
            Mode::Compress => y_in,
            Mode::Decompress => {
                if self.x <= xmid { 1 } else { 0 }
            }
        };

        if y_out != 0 {
            self.x2 = xmid;
        } else {
            self.x1 = xmid.wrapping_add(1);
        }
        predictor.update(y_out, lzp);

        // Renormalize, output leading bytes
        while ((self.x1 ^ self.x2) & 0xFF00_0000) == 0 {
            match self.mode {
                Mode::Compress => {
                    self.buf.push((self.x2 >> 24) as u8);
                    self.csize_in_blk += 1;
                }
                Mode::Decompress => {
                    let mut bb = [0u8; 1];
                    self.r.as_mut().unwrap().read_exact(&mut bb)?;
                    self.x = (self.x << 8) | bb[0] as u32;
                }
            }
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 255;
        }

        Ok(y_out)
    }

    /// Count one byte compressed (for flush threshold)
    #[inline(always)]
    pub fn count_byte(&mut self) {
        self.usize_in_blk += 1;
    }

    /// Flush block in compress mode (called at end of file or when threshold reached).
    pub fn flush(&mut self, out: &mut dyn Write) -> Result<()> {
        match self.mode {
            Mode::Decompress => Ok(()),
            Mode::Compress => {
                // emit trailer: force encoder to output bytes
                self.buf.push((self.x1 >> 24) as u8);
                self.buf.push(255);
                self.buf.push(255);
                self.buf.push(255);

                // Block header: 0, 'c', usize, csize
                out.write_all(&[0u8, b'c'])?;
                out.write_all(&self.usize_in_blk.to_be_bytes())?;
                let csize = self.csize_in_blk as u32 + 4; // added trailer above
                out.write_all(&csize.to_be_bytes())?;
                out.write_all(&self.buf)?;

                self.usum += self.usize_in_blk as f64;
                self.csum += csize as f64 + 10.0; // + header bytes
                if self.progress.verbose {
                    self.progress.log(format!("{:15.0} -> {:15.0}", self.usum, self.csum));
                }

                // reset
                self.x1 = 0;
                self.x2 = 0xFFFF_FFFF;
                self.x = 0;
                self.usize_in_blk = 0;
                self.csize_in_blk = 0;
                self.buf.clear();
                Ok(())
            }
        }
    }
}
