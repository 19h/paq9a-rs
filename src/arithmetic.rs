//! Logistic squash/stretch tables and integer log approximations,
//! ported from paq9a with fixed-point domains preserved.
//!
//! - squash: d in stretched domain (-2047..2047) -> p in linear domain (0..4095)
//! - stretch: p in linear domain (0..4095) -> d in stretched domain (-2047..2047)
//! - ilog/llog: base-2 log approximation tables used by StateMap adaptation.

use std::sync::OnceLock;

#[inline(always)]
pub fn clamp_i32(x: i32, lo: i32, hi: i32) -> i32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

/// squash: returns p = 1/(1 + exp(-d)), d scaled by 8 bits, p by 12 bits.
/// d is -2047..2047 (representing -8..8), p is 0..4095 (representing 0..1).
pub struct Squash {
    tab: [i32; 4096],
}
/// stretch: inverse of squash, p -> d.
pub struct Stretch {
    tab: [i32; 4096],
}

/// Ilog table: ilog(x) ~ round(log2(x) * 16), 0 <= x < 64K
pub struct Ilog {
    tab: [u8; 65536],
}

static SQUASH: OnceLock<Squash> = OnceLock::new();
static STRETCH: OnceLock<Stretch> = OnceLock::new();
static ILOG: OnceLock<Ilog> = OnceLock::new();

fn build_squash() -> Squash {
    let mut s_tab = [0i32; 4096];
    let t: [i32; 33] = [
        1, 2, 3, 6, 10, 16, 27, 45, 73, 120, 194, 310, 488, 747, 1101, 1546, 2047,
        2549, 2994, 3348, 3607, 3785, 3901, 3975, 4022, 4050, 4068, 4079, 4085,
        4089, 4092, 4093, 4094,
    ];
    for i in -2048..2048 {
        let w = i & 127;
        let d = (i >> 7) + 16;
        let v = ((t[d as usize] * (128 - w) + t[(d + 1) as usize] * w + 64) >> 7) as i32;
        s_tab[(i + 2048) as usize] = v;
    }
    Squash { tab: s_tab }
}

fn build_stretch(squash: &Squash) -> Stretch {
    let mut str_tab = [0i32; 4096];
    let mut pi = 0usize;
    for x in -2047..=2047 {
        let i = squash.apply(x);
        for j in pi..=i as usize {
            str_tab[j] = x;
        }
        pi = (i + 1) as usize;
    }
    str_tab[4095] = 2047;
    Stretch { tab: str_tab }
}

fn build_ilog() -> Ilog {
    let mut ilog = [0u8; 65536];
    let mut x: u32 = 14_155_776;
    for i in 2..65536 {
        x = x.wrapping_add(774_541_002 / (i * 2 - 1) as u32);
        ilog[i] = (x >> 24) as u8;
    }
    Ilog { tab: ilog }
}

impl Squash {
    #[inline(always)]
    pub fn apply(&self, d: i32) -> i32 {
        let d2 = d + 2048;
        if d2 < 0 {
            0
        } else if d2 > 4095 {
            4095
        } else {
            self.tab[d2 as usize]
        }
    }
    #[inline(always)]
    pub fn global() -> &'static Squash {
        SQUASH.get_or_init(|| build_squash())
    }
}

impl Stretch {
    #[inline(always)]
    pub fn apply(&self, p: i32) -> i32 {
        debug_assert!((0..4096).contains(&p));
        self.tab[p as usize]
    }
    #[inline(always)]
    pub fn global() -> &'static Stretch {
        STRETCH.get_or_init(|| {
            let sq = Squash::global();
            build_stretch(sq)
        })
    }
}

impl Ilog {
    #[inline(always)]
    pub fn apply(&self, x: u16) -> u8 {
        self.tab[x as usize]
    }
    #[inline(always)]
    pub fn global() -> &'static Ilog {
        ILOG.get_or_init(|| build_ilog())
    }
}

/// llog(x) accepts 32-bit x
#[inline(always)]
#[allow(dead_code)]
pub fn llog(x: u32) -> i32 {
    let ilog = Ilog::global();
    if x >= 0x01_00_00_00 {
        256 + ilog.apply((x >> 16) as u16) as i32
    } else if x >= 0x0001_0000 {
        128 + ilog.apply((x >> 8) as u16) as i32
    } else {
        ilog.apply(x as u16) as i32
    }
}

/// Convenience wrappers
#[inline(always)]
pub fn squash(d: i32) -> i32 {
    Squash::global().apply(d)
}
#[inline(always)]
pub fn stretch(p: i32) -> i32 {
    Stretch::global().apply(p)
}
