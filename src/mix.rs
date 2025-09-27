//! Mix and APM mixers: combine stretched predictions with context-dependent weights.
//!
//! Fixed-point domains:
//! - Inputs p1,p2: stretched, 8-bit fractional (-2047..2047).
//! - Weights: 24-bit scaled i32 (initialized to 1/2), LQ adaptation with a small learning schedule.

use crate::arithmetic::squash;

pub struct Mix {
    n: usize,        // number of contexts
    wt: Vec<i32>,    // weights: two per context (w0, w1), 24-bit fixed
    x1: i32,
    x2: i32,
    cxt: usize,
    pr: i32,
}

impl Mix {
    pub fn new(n: usize) -> Self {
        let mut wt = vec![0i32; n * 2];
        for w in &mut wt {
            *w = 1 << 23; // 0.5 in 24-bit fixed
        }
        Self { n, wt, x1: 0, x2: 0, cxt: 0, pr: 0 }
    }

    #[inline(always)]
    pub fn pp(&mut self, p1: i32, p2: i32, cx: usize) -> i32 {
        debug_assert!(cx < self.n);
        self.cxt = cx * 2;
        self.x1 = p1;
        self.x2 = p2;
        let w0 = self.wt[self.cxt] >> 16;
        let w1 = self.wt[self.cxt + 1] >> 16;
        self.pr = ((self.x1 * w0 + self.x2 * w1 + 128) >> 8) as i32;
        self.pr
    }

    #[inline(always)]
    pub fn update(&mut self, y: u8) {
        let yv = y as i32;
        let err = yv - squash(self.pr);
        // little schedule via two low bits of wt[cxt]
        let mut adj = err;
        let gate = self.wt[self.cxt] & 3;
        if gate < 3 {
            let g = (4 - ((self.wt[self.cxt] + 1) & 3)) as i32;
            adj *= g;
        }
        // err = (err + 8) >> 4
        adj = (adj + 8) >> 4;
        // wt[cxt] += x1 * err (clear low 2 bits)
        let delta0 = (self.x1 * adj) & !3;
        self.wt[self.cxt] = self.wt[self.cxt].wrapping_add(delta0);
        // wt[cxt+1] += x2 * err
        let delta1 = self.x2 * adj;
        self.wt[self.cxt + 1] = self.wt[self.cxt + 1].wrapping_add(delta1);
    }
}

/// APM: Mix specialized with constant first input; initialize w0=0.
pub struct APM {
    inner: Mix,
}

impl APM {
    pub fn new(n: usize) -> Self {
        let mut m = Mix::new(n);
        // set w0=0 for each context
        for i in 0..n {
            m.wt[2 * i] = 0;
        }
        Self { inner: m }
    }

    #[inline(always)]
    pub fn pp(&mut self, p1_const: i32, p2: i32, cx: usize) -> i32 {
        self.inner.pp(p1_const, p2, cx)
    }

    #[inline(always)]
    pub fn update(&mut self, y: u8) {
        self.inner.update(y)
    }
}
