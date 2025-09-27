//! Bit-level Predictor: mixes multiple contexts, bit histories, and LZP signal
//! to produce a 12-bit probability (0..4095) for the next bit being 1.
//!
//! This module ports the paq9a Predictor while eliminating raw pointers by
//! storing address tuples for state cells.

use crate::arithmetic::{squash, stretch};
use crate::hashtable::HashTable;
use crate::mix::{APM, Mix};
use crate::statemap::StateMap;
use crate::statetable::nex;
use crate::lzp::LZP;

pub struct Predictor {
    c0: u8,           // last 0-7 bits with leading 1, 0 before LZP flag
    nibble: u8,       // last 0-3 bits with leading 1 (1..15)
    bcount: u8,       // number of bits in c0 (0..7)
    t: HashTable<16>, // context -> state buckets
    sm: [StateMap; 11],
    m: [Mix; 10],
    a1: APM,
    a2: APM,
    a3: APM,
    // order-1 contexts (t2 in original)
    t2: Vec<u8>, // 0x40000 bytes
    // cached state cell locations from last p()
    last_cells: [StateCell; 11],
}

#[derive(Clone, Copy)]
enum TableRef {
    T2 { base: usize },        // base offset in t2
    T { base: usize },         // base offset in hash table 't'
}

#[derive(Clone, Copy)]
struct StateCell {
    tref: TableRef,
    offset: u8, // which byte within the chosen bucket/region
}

impl Predictor {
    pub fn new(mem_bytes: usize) -> Self {
        let t = HashTable::<16>::new(mem_bytes / 2);
        let mk_sm = || StateMap::new(256);
        let sm = [
            mk_sm(), mk_sm(), mk_sm(), mk_sm(), mk_sm(), mk_sm(),
            mk_sm(), mk_sm(), mk_sm(), mk_sm(), mk_sm(),
        ];
        let mk_m = || Mix::new(512);
        let m = [
            mk_m(), mk_m(), mk_m(), mk_m(), mk_m(),
            mk_m(), mk_m(), mk_m(), mk_m(), mk_m(),
        ];
        Self {
            c0: 0,
            nibble: 1,
            bcount: 0,
            t,
            sm,
            m,
            a1: APM::new(0x10000),
            a2: APM::new(0x10000),
            a3: APM::new(0x10000),
            t2: vec![0u8; 0x40000],
            last_cells: [StateCell { tref: TableRef::T2 { base: 0 }, offset: 0 }; 11],
        }
    }

    #[inline(always)]
    pub fn update(&mut self, y: u8, _lzp: &mut LZP) {
        if self.c0 == 0 {
            // start-of-byte LZP flag handled in p()
            self.c0 = 1 - y;
        } else {
            // Walk and update bit histories for all contexts
            for i in 0..11 {
                let sc = self.last_cells[i];
                match sc.tref {
                    TableRef::T2 { base } => {
                        let idx = base + sc.offset as usize;
                        let st = self.t2[idx];
                        self.t2[idx] = nex(st, y);
                        self.sm[i].update(y, 255);
                        if i > 0 {
                            self.m[i - 1].update(y);
                        }
                    }
                    TableRef::T { base } => {
                        let st = *self.t.get_byte_mut(base, sc.offset as usize);
                        let ns = nex(st, y);
                        *self.t.get_byte_mut(base, sc.offset as usize) = ns;
                        self.sm[i].update(y, 255);
                        if i > 0 {
                            self.m[i - 1].update(y);
                        }
                    }
                }
            }

            // advance bit-wise contexts
            self.c0 = self.c0.wrapping_add(self.c0).wrapping_add(y);
            self.bcount += 1;
            if self.bcount == 8 {
                self.bcount = 0;
                self.c0 = 0;
            }
            self.nibble = self.nibble.wrapping_add(self.nibble).wrapping_add(y);
            if self.nibble >= 16 {
                self.nibble = 1;
            }
            self.a1.update(y);
            self.a2.update(y);
            self.a3.update(y);
        }
    }

    /// Probability for next bit (0..4095)
    pub fn p(&mut self, lzp: &mut LZP) -> i32 {
        if self.c0 == 0 {
            return lzp.p();
        }

        // pc = LZP predicted byte; r tests if current known bits equal pc's high bits
        let mut pc = lzp.c(); // -1 or 0..255
        let r = if pc >= 0 {
            // Assumption A1: r = (((pc + 256) >> (8 - bcount)) == c0) ? 1 : 0
            let rcalc = (((pc as i32 + 256) >> (8 - self.bcount) as i32) & 0xFF) as u8 == self.c0;
            if rcalc { 1 } else { 0 }
        } else {
            0
        };
        if r == 0 {
            pc = 0;
        }

        // contexts
        let c4 = lzp.c4();
        let c8 = (lzp.c8() << 4).wrapping_sub(1);

        // At nibble boundary, refresh base pointers
        if (self.bcount & 3) == 0 {
            if self.bcount == 0 {
                // byte boundary: update order-1 base pointers in t2
                let base0 = ((c4 >> 16) & 0xFF00) as usize;
                let base1 = ((c4 >> 8) & 0xFF00) as usize + 0x10000;
                let base2 = ((c4) & 0xFF00) as usize + 0x20000;
                let base3 = ((c4 << 8) & 0xFF00) as usize + 0x30000;
                self.last_cells[0].tref = TableRef::T2 { base: base0 };
                self.last_cells[1].tref = TableRef::T2 { base: base1 };
                self.last_cells[2].tref = TableRef::T2 { base: base2 };
                self.last_cells[3].tref = TableRef::T2 { base: base3 };
            }
            // hash-table contexts (will set base per context)
            let idx4 = (((c4 << 8) & 0xFFFF00) as usize).wrapping_sub(self.c0 as usize);
            let base4 = self.t.lookup(idx4 as u32);
            self.last_cells[4].tref = TableRef::T { base: base4 };

            let idx5 = ((((c4 << 8) & 0xFFFFFF00) as usize).wrapping_mul(3)).wrapping_add(self.c0 as usize);
            let base5 = self.t.lookup(idx5 as u32);
            self.last_cells[5].tref = TableRef::T { base: base5 };

            let idx6 = (c4 as usize).wrapping_mul(7).wrapping_add(self.c0 as usize);
            let base6 = self.t.lookup(idx6 as u32);
            self.last_cells[6].tref = TableRef::T { base: base6 };

            let idx7 = (((c8.wrapping_mul(5)) & 0xFFFFFC) as usize).wrapping_add(self.c0 as usize);
            let base7 = self.t.lookup(idx7 as u32);
            self.last_cells[7].tref = TableRef::T { base: base7 };

            let idx8 = (((c8.wrapping_mul(11)) & 0xFFFFFF0) as usize)
                .wrapping_add(self.c0 as usize)
                .wrapping_add((((pc as u32) & 0xFF).wrapping_mul(13)) as usize);
            let base8 = self.t.lookup(idx8 as u32);
            self.last_cells[8].tref = TableRef::T { base: base8 };

            let idx9 = (lzp.word0 as usize).wrapping_mul(5)
                .wrapping_add(self.c0 as usize)
                .wrapping_add((((pc as u32) & 0xFF).wrapping_mul(17)) as usize);
            let base9 = self.t.lookup(idx9 as u32);
            self.last_cells[9].tref = TableRef::T { base: base9 };

            let idx10 = (lzp.word1 as usize).wrapping_mul(7)
                .wrapping_add((lzp.word0 as usize).wrapping_mul(11))
                .wrapping_add(self.c0 as usize)
                .wrapping_add((((pc as u32) & 0xFF).wrapping_mul(37)) as usize);
            let base10 = self.t.lookup(idx10 as u32);
            self.last_cells[10].tref = TableRef::T { base: base10 };
        }

        // Compute prediction by mixing contexts
        self.last_cells[0].offset = self.c0;
        let st0 = match self.last_cells[0].tref {
            TableRef::T2 { base } => self.t2[base + self.c0 as usize],
            _ => unreachable!(),
        };
        let mut pr = stretch(self.sm[0].p(st0 as usize));
        for i in 1..11 {
            let off = if i < 4 { self.c0 } else { self.nibble };
            self.last_cells[i].offset = off;
            let st = match self.last_cells[i].tref {
                TableRef::T2 { base } => self.t2[base + off as usize],
                TableRef::T { base } => *self.t.get_byte_mut(base, off as usize),
            };
            let pr2 = stretch(self.sm[i].p(st as usize));
            let st_idx = (st as usize) + ((r as usize) << 8);
            pr = (self.m[i - 1].pp(pr, pr2, st_idx) * 3 + pr) >> 2;
        }

        // Adjust with APMs
        let pc_u = (pc as u32 & 0xFF) as usize;
        let cx_a1 = ((self.c0 as usize) + (pc_u << 8)) & 0xFFFF;
        pr = (self.a1.pp(512, pr * 2, cx_a1) * 3 + pr) >> 2;

        let cx_a2 = (((c4 << 8) & 0xFF00 | self.c0 as u32) as usize) & 0xFFFF;
        pr = (self.a2.pp(512, pr * 2, cx_a2) * 3 + pr) >> 2;

        let cx_a3 = ((c4 as usize) * 3 + self.c0 as usize) & 0xFFFF;
        pr = (self.a3.pp(512, pr * 2, cx_a3) * 3 + pr) >> 2;

        squash(pr)
    }
}
