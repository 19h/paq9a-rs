use tempfile::tempdir;
use paq9a_rs::{create_archive, extract_archive, list_archive, ArchiveOptions, MemLevel, Progress};

#[test]
fn roundtrip_compress_extract_small_text() -> anyhow::Result<()> {
    let td = tempdir()?;
    let p1 = td.path().join("a.txt");
    let p2 = td.path().join("b.txt");

    std::fs::write(&p1, b"hello world\nhello world\nhello world\n")?;
    std::fs::write(&p2, b"abc ABC Abc aBC\n")?;

    let arch = td.path().join("foo.paq9a");
    let opts = ArchiveOptions {
        mem: MemLevel::new(1)?,
        store: false,
        progress: Progress::none(),
    };
    create_archive(&arch, opts, &[p1.clone(), p2.clone()])?;

    // list
    let mut listing = Vec::new();
    list_archive(&arch, &mut listing)?;
    let s = String::from_utf8(listing).unwrap();
    assert!(s.contains("paq9a -1"));

    // extract to new names
    let x1 = td.path().join("x.txt");
    let y1 = td.path().join("y.txt");
    extract_archive(&arch, &[x1.clone(), y1.clone()])?;

    assert_eq!(std::fs::read(&p1)?, std::fs::read(&x1)?);
    assert_eq!(std::fs::read(&p2)?, std::fs::read(&y1)?);

    Ok(())
}

#[test]
fn roundtrip_store_mode() -> anyhow::Result<()> {
    let td = tempdir()?;
    let p1 = td.path().join("bin1.bin");
    let p2 = td.path().join("bin2.bin");
    let data1 = (0..4096u32).flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
    let data2 = vec![0xAAu8; 100_000];
    std::fs::write(&p1, &data1)?;
    std::fs::write(&p2, &data2)?;

    let arch = td.path().join("s.paq9a");
    let opts = ArchiveOptions {
        mem: MemLevel::new(2)?,
        store: true,
        progress: Progress::none(),
    };
    create_archive(&arch, opts, &[p1.clone(), p2.clone()])?;

    // extract back
    let out1 = td.path().join("out1.bin");
    let out2 = td.path().join("out2.bin");
    extract_archive(&arch, &[out1.clone(), out2.clone()])?;

    assert_eq!(std::fs::read(&p1)?, std::fs::read(&out1)?);
    assert_eq!(std::fs::read(&p2)?, std::fs::read(&out2)?);
    Ok(())
}

#[test]
fn no_overwrite_on_extract() -> anyhow::Result<()> {
    let td = tempdir()?;
    let src = td.path().join("a.txt");
    std::fs::write(&src, b"data")?;

    let arch = td.path().join("t.paq9a");
    let opts = ArchiveOptions {
        mem: paq9a_rs::MemLevel::new(1)?,
        store: false,
        progress: paq9a_rs::Progress::none(),
    };
    create_archive(&arch, opts, &[src.clone()])?;

    let target = td.path().join("a.txt");
    // Pre-create to force skip
    std::fs::write(&target, b"preexisting")?;
    // Extract w/out renames (should try to write a.txt and skip)
    extract_archive(&arch, &[])?;
    // File unchanged
    assert_eq!(std::fs::read(&target)?, b"preexisting");

    Ok(())
}
