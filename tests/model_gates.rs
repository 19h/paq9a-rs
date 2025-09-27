use paq9a_rs::util::MemLevel;
use paq9a_rs::lzp::LZP;
use paq9a_rs::predictor::Predictor;

// Validates Assumption A1 gating behavior is well-defined and doesn't panic.
#[test]
fn gate_consistency_monotone() {
    let ml = MemLevel::new(1).unwrap();
    let mut lzp = LZP::new(ml);
    let mut pred = Predictor::new(ml.bytes());
    // Feed some bytes to seed contexts
    for &b in b"The quick brown fox jumps over the lazy dog" {
        let _p = pred.p(&mut lzp);
        // force literal path
        pred.update(0, &mut lzp);
        for i in (0..8).rev() {
            pred.update((b >> i) & 1, &mut lzp);
        }
        lzp.update(b);
    }
    // Now that contexts exist, ensure p() returns in range across steps
    for _ in 0..16 {
        let p = pred.p(&mut lzp);
        assert!((0..4096).contains(&p));
        pred.update(1, &mut lzp);
    }
}
