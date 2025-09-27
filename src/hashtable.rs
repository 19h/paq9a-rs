//! Power-of-two byte array hash table with B-byte buckets and LFU replacement,
//! following the paq9a design.
//!
//! Layout per bucket (B >= 2):
//!   [0] = checksum (u8)
//!   [1] = priority (u8, 0 means empty, larger is more recently/frequently used)
//!   [2..B-1] = payload data
//!
//! Probing: 3 locations in a cache-line range: i, i^B, i^(2B)
//! Replacement: choose the one with smallest priority.
//!
//! Implementation notes:
//! - Uses a single contiguous byte slab to preserve spatial locality.
//! - B and nbytes must be powers of two; buckets are B-byte aligned within the slab.
//! - Optional CPU prefetch (feature "prefetch") to hide DRAM latency on the 3 probes.

pub struct HashTable<const B: usize> {
    pub(crate) t: Vec<u8>,
    pub(crate) nbytes: usize,
}

impl<const B: usize> HashTable<B> {
    pub fn new(nbytes: usize) -> Self {
        assert!(B >= 2 && (B & (B - 1)) == 0);
        assert!(nbytes >= B * 4 && (nbytes & (nbytes - 1)) == 0);
        let t = vec![0u8; nbytes];
        Self { t, nbytes }
    }

    /// Lookup bucket for logical index `idx` and return the starting index
    /// (byte offset) into `t`. Priority is bumped on use (saturating).
    #[inline(always)]
    pub fn lookup(&mut self, idx: u32) -> usize {
        // Mix idx -> pseudo-random byte address within table
        let mut i = idx.wrapping_mul(123_456_791);
        i = i.rotate_left(16);
        i = i.wrapping_mul(234_567_891);

        let chk = (i >> 24) as u8;
        let mask_base = self.nbytes - B;
        // `offs*` always aligned to bucket size since we multiply by B
        let offs0 = ((i as usize) * B) & mask_base;
        let offs1 = offs0 ^ B;
        let offs2 = offs0 ^ (B * 2);

        #[cfg(all(feature="prefetch", any(target_arch="x86", target_arch="x86_64")))]
        unsafe {
            use core::arch::x86_64::_mm_prefetch;
            const _MM_HINT_T0: i32 = 3;
            _mm_prefetch(self.t.as_ptr().add(offs0) as *const i8, _MM_HINT_T0);
            _mm_prefetch(self.t.as_ptr().add(offs1) as *const i8, _MM_HINT_T0);
            _mm_prefetch(self.t.as_ptr().add(offs2) as *const i8, _MM_HINT_T0);
        }

        // Probe 3 adjacent buckets for checksum match.
        // Choose hit if any; otherwise pick the lowest-priority victim to replace.
        let t = &mut self.t;
        let hit_offs = if unsafe { *t.get_unchecked(offs0) } == chk {
            Some(offs0)
        } else if unsafe { *t.get_unchecked(offs1) } == chk {
            Some(offs1)
        } else if unsafe { *t.get_unchecked(offs2) } == chk {
            Some(offs2)
        } else {
            None
        };

        let chosen = if let Some(o) = hit_offs {
            o
        } else {
            // replacement policy: lowest priority (byte 1)
            let p0 = unsafe { *t.get_unchecked(offs0 + 1) };
            let p1 = unsafe { *t.get_unchecked(offs1 + 1) };
            let p2 = unsafe { *t.get_unchecked(offs2 + 1) };

            // choose among offs0, offs1, offs2 the smallest priority
            let (mut offs, mut pr) = (offs0, p0);
            if p1 < pr { offs = offs1; pr = p1; }
            if p2 < pr { offs = offs2; /* pr = p2; */ }

            // replace
            {
                let b = &mut t[offs..offs + B];
                for x in b.iter_mut() {
                    *x = 0;
                }
                b[0] = chk;
                // b[1] remains 0; we'll bump below to 1
            }
            offs
        };

        // bump priority (saturating) to reward use
        let pr = &mut t[chosen + 1];
        *pr = pr.saturating_add(1);

        chosen
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
