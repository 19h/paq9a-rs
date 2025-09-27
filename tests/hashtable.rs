use paq9a_rs::hashtable::HashTable;

#[test]
fn basic_hashtable_behavior() {
    let mut h = HashTable::<16>::new(1 << 16);
    let base = h.lookup(12345);
    // checksum set at [0]
    let chk = h.base_slice_mut(base)[0];
    assert!(chk != 0);
    // priority starts at 0, we can bump it
    *h.get_byte_mut(base, 1) = 10;
    assert_eq!(*h.get_byte_mut(base, 1), 10);
}
