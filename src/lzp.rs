//! LZP stage: predicts next byte by matching current context to a rotating buffer.
//! Uses a hash table of pointers into the buffer to find matches. The prediction
//! probability is refined with StateMap and APM mixers exactly as in paq9a.
//!
//! Improvement: adaptive LZP gating. The minimum match length threshold is
//! selected dynamically via an EMA of recent match accuracy, choosing among
//! {12, 10, 8}. This remains fully symmetric (encoder/decoder) and format-
//! compatible because it depends only on past decoded bytes.

use crate::mix::APM;
use crate::statemap::StateMap;
use crate::util::MemLevel;
use crate::arithmetic::clamp_i32;

const MINLEN_BASE: usize = 12;
const EMA_SCALE: i32 = 4096;     // 1.0
const EMA_SHIFT: i32 = 8;        // alpha = 1/256

pub struct LZP {
    n: usize,        // buffer size (MEM/8)
    hsize: usize,    // hash table size (MEM/32)
    mask: usize,     // n - 1, for ring wrap
    buf: Vec<u8>,
    t: Vec<i32>,     // positions (i32 suffices)
    pub word0: u32,
    pub word1: u32,
    match_pos: isize,
    len: usize,
    pos: usize,
    h: usize,
    h1: u32,
    h2: u32,
    sm1: StateMap,
    a1: APM,
    a2: APM,
    a3: APM,
    literals: usize,
    matches: usize,
    // EMA of match-flag success to adapt the gate
    ema_hits: i32, // scaled by EMA_SCALE
}

impl LZP {
    pub fn new(mem: MemLevel) -> Self {
        let mem_bytes = mem.bytes();
        let n = mem_bytes / 8;
        let hsize = mem_bytes / 32;
        let buf = vec![0u8; n];
        let t = vec![0i32; hsize];
        Self {
            n,
            hsize,
            mask: n - 1,
            buf,
            t,
            word0: 0,
            word1: 0,
            match_pos: -1,
            len: 0,
            pos: 0,
            h: 0,
            h1: 0,
            h2: 0,
            sm1: StateMap::new(0x200),
            a1: APM::new(0x10000),
            a2: APM::new(0x40000),
            a3: APM::new(0x100000),
            literals: 0,
            matches: 0,
            // start neutral; adaptive threshold settles by traffic
            ema_hits: (EMA_SCALE * 3) / 4,
        }
    }

    #[inline(always)]
    fn minlen(&self) -> usize {
        // Map EMA to thresholds; start near 8..10 by default
        if self.ema_hits >= (EMA_SCALE * 3) / 4 {
            8
        } else if self.ema_hits >= (EMA_SCALE / 2) {
            10
        } else {
            MINLEN_BASE
        }
    }

    #[inline(always)]
    pub fn c(&self) -> i32 {
        if self.len >= self.minlen() {
            let idx = (self.match_pos as usize) & self.mask;
            self.buf[idx] as i32
        } else {
            -1
        }
    }

    #[inline(always)]
    pub fn c_back(&self, i: usize) -> u8 {
        let idx = (self.pos.wrapping_sub(i)) & self.mask;
        self.buf[idx]
    }

    #[inline(always)]
    pub fn c4(&self) -> u32 {
        self.h2
    }

    #[inline(always)]
    pub fn c8(&self) -> u32 {
        self.h1
    }

    /// Probability (0..4095) that c() is next
    #[inline(always)]
    pub fn p(&mut self) -> i32 {
        if self.len < self.minlen() {
            return 0;
        }
        let mut cxt = self.len;
        if self.len > 28 {
            cxt = 28 + (self.len >= 32) as usize + (self.len >= 64) as usize + (self.len >= 128) as usize;
        }
        let pc = self.c() as usize; // 0..255
        let mut pr = self.sm1.p(cxt);
        let pr_st = crate::arithmetic::stretch(pr);
        let cx1 = ((self.h2 << 8) | (pc as u32 & 0xFF)) as usize & 0xFFFF;
        pr = self.a1.pp(2048, pr_st * 2, cx1) * 3 + pr >> 2;
        pr = clamp_i32(pr, 0, 4095);

        let cx2 = (((self.h1.wrapping_mul(11 << 6)) + (pc as u32)) & 0x3FFFF) as usize;
        pr = self.a2.pp(2048, crate::arithmetic::stretch(pr) * 2, cx2) * 3 + pr >> 2;
        pr = clamp_i32(pr, 0, 4095);

        let cx3 = (((self.h1.wrapping_mul(7 << 4)) + (pc as u32)) & 0xFFFFF) as usize;
        pr = self.a3.pp(2048, crate::arithmetic::stretch(pr) * 2, cx3) * 3 + pr >> 2;
        pr = clamp_i32(pr, 0, 4095);

        crate::arithmetic::squash(pr)
    }

    /// Update model with actual byte ch (0..255)
    #[inline(always)]
    pub fn update(&mut self, ch: u8) {
        let gate = self.minlen();
        let y = if self.len >= gate && self.c() == ch as i32 { 1 } else { 0 };

        // update EMA for adaptive gating
        let target = if y != 0 { EMA_SCALE } else { 0 };
        self.ema_hits += (target - self.ema_hits) >> EMA_SHIFT;

        // update context hashes
        self.h1 = self.h1.wrapping_mul(3 << 4).wrapping_add(ch as u32).wrapping_add(1);
        self.h2 = (self.h2 << 8) | (ch as u32);
        self.h = self.h.wrapping_mul(5 << 2).wrapping_add(ch as usize + 1) & (self.hsize - 1);

        if self.len >= gate {
            self.sm1.update(y, 255);
            self.a1.update(y);
            self.a2.update(y);
            self.a3.update(y);
        }

        // update word hashes (case-insensitive letters)
        if ch.is_ascii_alphabetic() {
            self.word0 = self.word0.wrapping_mul(29 << 2).wrapping_add(ch.to_ascii_lowercase() as u32);
        } else if self.word0 != 0 {
            self.word1 = self.word0;
            self.word0 = 0;
        }

        // write byte to ring buffer
        self.buf[self.pos & self.mask] = ch;
        self.pos = self.pos.wrapping_add(1);

        if y != 0 {
            self.len += 1;
            self.match_pos += 1;
            self.matches += 1;
        } else {
            self.literals += 1;
            self.len = 1;
            let mut m = self.t[self.h] as isize;
            if ((m ^ self.pos as isize) & (self.n as isize - 1)) == 0 {
                m -= 1;
            }
            // grow match
            let mut l = 1usize;
            while l <= 128 {
                let b1 = self.buf[((m as usize).wrapping_sub(l)) & self.mask];
                let b2 = self.buf[(self.pos.wrapping_sub(l)) & self.mask];
                if b1 != b2 {
                    break;
                }
                l += 1;
            }
            self.match_pos = m;
            self.len = l - 1;
        }

        self.t[self.h] = self.pos as i32;
    }
}
