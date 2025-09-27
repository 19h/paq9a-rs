//! StateMap: context -> probability mapping with adaptive counts.
//! Implemented as in the original: a table of (p, n) packed in 32 bits,
//! with max count limit (default 255) and learning rates via a precomputed dt[].

use crate::arithmetic::{squash, clamp_i32};
use std::sync::OnceLock;

pub struct StateMap {
    n: usize,          // number of contexts
    cxt: usize,        // last context
    t: Vec<u32>,       // table: high 22 bits: prediction p (linear, 0..4095) scaled into 22 bits; low 10 bits: count n
}

impl StateMap {
    // dt[i] = 16K / (i+3)
    fn build_dt() -> [i32; 1024] {
        let mut dt = [0i32; 1024];
        for i in 0..1024 {
            dt[i] = 16384 / (i as i32 + i as i32 + 3);
        }
        dt
    }

    pub fn new(n: usize) -> Self {
        let mut t = vec![0u32; n];
        for v in &mut t {
            *v = 1u32 << 31; // p=0.5 in high bits, n=0
        }
        Self { n, cxt: 0, t }
    }

    #[inline(always)]
    pub fn p(&mut self, cx: usize) -> i32 {
        debug_assert!(cx < self.n);
        self.cxt = cx;
        (self.t[cx] >> 20) as i32
    }

    #[inline(always)]
    pub fn update(&mut self, y: u8, limit: u32) {
        debug_assert!(self.cxt < self.n);
        let idx = self.cxt;
        let n = self.t[idx] & 1023;
        let p = self.t[idx] >> 10; // 22-bit scaled
        let mut ny = n;
        if ny < limit { ny += 1; } else { ny = limit; }
        // error = y - squash(p >> 2) mapped to 12-bit linear
        let pr = (p >> 2) as i32;
        let pr_lin = squash(pr);  // 0..4095 linear
        let err = y as i32 - pr_lin;
        static DT: OnceLock<[i32; 1024]> = OnceLock::new();
        let dt = DT.get_or_init(|| Self::build_dt());
        // Prevent overflow: clamp err to reasonable bounds before shifting
        let clamped_err = clamp_i32(err, -1024, 1024);
        let delta_part = ((clamped_err as i64) << 22) - (p as i64);
        let delta_shifted = (delta_part >> 3) as i32;
        let delta = delta_shifted.saturating_mul(dt[n as usize]);
        let newp = ((p as i32).saturating_add(delta & 0xFFFFFC00u32 as i32) as u32) & 0xFFFFFC00u32;
        self.t[idx] = newp | ny;
    }
}
