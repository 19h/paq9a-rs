//! Power-of-two byte array hash table with B-byte buckets and LFU replacement,
//! following the paq9a design.
//!
//! Layout per bucket (B >= 2):
//!   [0] = checksum (u8)
//!   [1] = priority (u8, 0 means empty, larger is more recently used)
//!   [2..B-1] = payload data
//!
//! Probing: 3 locations in a cache-line range: i, i^B, i^(2B)
//! Replacement: choose the one with smallest priority.

pub struct HashTable<const B: usize> {
    t: Vec<u8>,
    nbytes: usize,
}

impl<const B: usize> HashTable<B> {
    pub fn new(nbytes: usize) -> Self {
        assert!(B >= 2 && (B & (B - 1)) == 0);
        assert!(nbytes >= B * 4 && (nbytes & (nbytes - 1)) == 0);
        let t = vec![0u8; nbytes];
        Self { t, nbytes }
    }

    #[inline(always)]
    fn bucket(&mut self, i: usize) -> &mut [u8] {
        &mut self.t[i..i + B]
    }

    /// Lookup bucket for logical index `idx` and return the starting index
    /// (byte offset) into `t`.
    pub fn lookup(&mut self, idx: u32) -> usize {
        let mut i = idx.wrapping_mul(123_456_791);
        i = i.rotate_left(16);
        i = i.wrapping_mul(234_567_891);
        let chk = (i >> 24) as u8;
        let mut offs = ((i as usize) * B) & (self.nbytes - B);

        // probe 3 adjacent locations
        if self.t[offs] == chk {
            return offs;
        }
        let offs1 = offs ^ B;
        if self.t[offs1] == chk {
            return offs1;
        }
        let offs2 = offs ^ (B * 2);
        if self.t[offs2] == chk {
            return offs2;
        }

        // replacement policy: lowest priority (byte 1)
        let p0 = self.t[offs + 1];
        let p1 = self.t[offs1 + 1];
        let p2 = self.t[offs2 + 1];

        offs = if p0 > p1 || p0 > p2 { offs ^ B } else { offs };
        let po = self.t[offs + 1];
        offs = if po > self.t[(offs ^ (B * 2)) + 1] { offs ^ (B * 2) } else { offs };

        // replace
        {
            let b = self.bucket(offs);
            for x in b.iter_mut() {
                *x = 0;
            }
            b[0] = chk;
        }
        offs
    }

    #[inline(always)]
    pub fn get_byte_mut(&mut self, base: usize, offset: usize) -> &mut u8 {
        &mut self.t[base + offset]
    }

    #[inline(always)]
    pub fn base_slice_mut(&mut self, base: usize) -> &mut [u8] {
        &mut self.t[base..base + B]
    }
}
