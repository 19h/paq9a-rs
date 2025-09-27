//! High-level archive operations: create, extract, list.
//!
//! Format identical to paq9a:
//!   Header: b"pQ9" + 0x01 (version), then memory-level byte '1'..'9'
//!   For each file:
//!     filename (UTF-8 bytes) + 0x00 terminator
//!     Then one or more blocks, each:
//!       0x00, mode ('s' store or 'c' compress), usize (BE u32), csize (BE u32), payload
//!
//! Archives are solid: model state persists across files.

use crate::lzp::LZP;
use crate::predictor::Predictor;
use crate::util::{MemLevel, Progress};
use crate::encoder::{Encoder, BUFSIZE};
use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write, Seek, BufReader, BufWriter, SeekFrom, BufRead};
use std::path::{Path, PathBuf};

pub struct ArchiveOptions {
    pub mem: MemLevel,
    pub store: bool,     // -s (store) or -c (compress)
    pub progress: Progress,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            mem: crate::util::MemLevel::new(7).unwrap(),
            store: false,
            progress: Progress::none(),
        }
    }
}

const HEADER_MAGIC: &[u8; 4] = b"pQ9\x01";

pub fn create_archive<P: AsRef<Path>>(archive: P, opts: ArchiveOptions, files: &[PathBuf]) -> Result<()> {
    // Cannot overwrite existing archive
    if archive.as_ref().exists() {
        bail!("Cannot overwrite archive {}", archive.as_ref().display());
    }

    let mut out = BufWriter::with_capacity(1 << 20, File::create(&archive)?);
    // header
    out.write_all(HEADER_MAGIC)?;
    out.write_all(&[opts.mem.header_char()])?;

    // initialize model
    let mut lzp = LZP::new(opts.mem);
    let mut predictor = Predictor::new(opts.mem.bytes());

    for f in files {
        if f.is_dir() {
            opts.progress.log(format!("Skipping directory: {}", f.display()));
            continue;
        }
        let mut r = match File::open(&f) {
            Ok(fh) => BufReader::with_capacity(1 << 20, fh),
            Err(_) => {
                eprintln!("File not found: {}", f.display());
                continue;
            }
        };

        // filename
        let fname = f.to_string_lossy();
        let name_bytes = fname.as_bytes();
        out.write_all(name_bytes)?;
        out.write_all(&[0u8])?;
        opts.progress.log(format!("{:-40} ", fname));

        if opts.store {
            store_file(&mut r, &mut out)?;
        } else {
            // compress using paq9a block encoder
            compress_stream(&mut r, &mut out, &mut predictor, &mut lzp, &opts.progress)?;
        }

        opts.progress.log("\n".to_string());
    }

    out.flush()?;
    Ok(())
}

fn store_file<R: Read, W: Write>(in_r: &mut R, out_w: &mut W) -> Result<()> {
    const BLOCK: usize = 0x10_0000; // 1 MiB
    let mut buf = vec![0u8; BLOCK];
    let mut first = true;
    loop {
        let n = in_r.read(&mut buf)?;
        if !first && n == 0 { break; }
        out_w.write_all(&[0u8, b's'])?;
        out_w.write_all(&(n as u32).to_be_bytes())?;
        out_w.write_all(&(n as u32).to_be_bytes())?;
        if n != 0 {
            out_w.write_all(&buf[..n])?;
        }
        first = false;
    }
    Ok(())
}

fn compress_stream<R: Read, W: Write>(
    in_r: &mut R,
    out_w: &mut W,
    predictor: &mut Predictor,
    lzp: &mut LZP,
    progress: &Progress,
) -> Result<()> {

    let mut enc: Encoder<'_, R, W> = Encoder::new_compress(progress.clone());

    // Read source in large chunks to minimize syscall/branch overhead
    const READBUF: usize = 1 << 20; // 1 MiB
    let mut rbuf = vec![0u8; READBUF];

    loop {
        let n = in_r.read(&mut rbuf)?;
        if n == 0 { break; }
        for &c in &rbuf[..n] {
            let cp = lzp.c();
            if cp >= 0 && (c as i32) == cp {
                enc.code(predictor, lzp, 1)?;
            } else {
                enc.code(predictor, lzp, 0)?;
                for i in (0..8).rev() {
                    let b = (c >> i) & 1;
                    enc.code(predictor, lzp, b)?;
                }
            }
            enc.count_byte();
            lzp.update(c);
            if enc.pending_bytes() > BUFSIZE - 256 {
                enc.flush(out_w)?;
            }
        }
    }
    enc.flush(out_w)?;
    Ok(())
}

pub fn list_archive<P: AsRef<Path>>(archive: P, mut out: impl Write) -> Result<()> {
    let mut f = BufReader::with_capacity(1 << 20, File::open(&archive)
        .with_context(|| format!("Cannot find archive {}", archive.as_ref().display()))?);
    // check header
    let mut header = [0u8; 4];
    f.read_exact(&mut header)?;
    if &header != HEADER_MAGIC {
        bail!("{}: Not a paq9a archive", archive.as_ref().display());
    }
    let mut mem_byte = [0u8; 1];
    f.read_exact(&mut mem_byte)?;
    let mem_level = crate::util::MemLevel::from_header_char(mem_byte[0])?;
    writeln!(out, "\npaq9a {}", mem_level)?;

    let mut cur_name: Option<String> = None;
    let mut usum_file: f64 = 0.0;
    let mut csum_file: f64 = 0.0;
    let mut utotal: f64 = 0.0;
    let mut ctotal: f64 = 5.0; // header bytes (magic 4 + mem 1)

    loop {
        // Read filename (NUL-terminated) or EOF
        let mut name_bytes = Vec::<u8>::new();
        let mut b = [0u8; 1];
        match f.read(&mut b) {
            Ok(0) => break,
            Ok(_) => {
                if b[0] != 0 {
                    name_bytes.push(b[0]);
                    f.read_until(0, &mut name_bytes)?; // includes 0
                    if let Some(&0) = name_bytes.last() { name_bytes.pop(); }
                } else {
                    // continuation block (no new file name)
                }
            }
            Err(e) => bail!("I/O error while reading archive: {}", e),
        }

        if !name_bytes.is_empty() {
            // finalize previous file entry (if any)
            if let Some(prev) = cur_name.take() {
                writeln!(out, "{:10.0} -> {:10.0} c {}", usum_file, csum_file, prev)?;
                utotal += usum_file;
                ctotal += csum_file;
                usum_file = 0.0;
                csum_file = 0.0;
            }
            cur_name = Some(String::from_utf8_lossy(&name_bytes).to_string());
        }

        // read block header
        // If we just read a new filename, the next byte is a block-start marker 0x00.
        let mut m = [0u8; 1];
        f.read_exact(&mut m)?;
        if !name_bytes.is_empty() {
            // We expect to see the 0x00 block-start marker right after the filename NUL.
            if m[0] != 0 {
                bail!(
                    "Archive corrupted: expected block marker 0 after filename at {}",
                    f.get_ref().stream_position()? - 1
                );
            }
            f.read_exact(&mut m)?; // now read the actual mode byte
        }
        let mode = m[0];
        let usize = read_u32_be(&mut f)? as usize;
        let csize = read_u32_be(&mut f)? as usize;

        usum_file += usize as f64;
        csum_file += csize as f64 + 10.0;

        if mode != b'c' && mode != b's' {
            bail!("Archive corrupted: usize={} csize={} mode={} at {}", usize, csize, mode, f.get_ref().stream_position()?);
        }

        // skip payload efficiently
        f.seek(SeekFrom::Current(csize as i64))?;
    }

    if let Some(last) = cur_name.take() {
        writeln!(out, "{:10.0} -> {:10.0} c {}", usum_file, csum_file, last)?;
        utotal += usum_file;
        ctotal += csum_file;
    }
    writeln!(out, "{:10.0} -> {:10.0} total", utotal, ctotal)?;
    Ok(())
}

pub fn extract_archive<P: AsRef<Path>>(archive: P, outnames: &[PathBuf]) -> Result<()> {
    let mut f = BufReader::with_capacity(1 << 20, File::open(&archive)
        .with_context(|| format!("Cannot find archive {}", archive.as_ref().display()))?);
    // header
    let mut header = [0u8; 4];
    f.read_exact(&mut header)?;
    if &header != HEADER_MAGIC {
        bail!("{}: Not a paq9a archive", archive.as_ref().display());
    }
    let mut mem_byte = [0u8; 1];
    f.read_exact(&mut mem_byte)?;
    let mem_level = crate::util::MemLevel::from_header_char(mem_byte[0])?;
    let mem_bytes = (1usize) << (22 + (mem_level.level as usize));
    // predictor/lzp
    let mut lzp = LZP::new(mem_level);
    let mut predictor = crate::predictor::Predictor::new(mem_bytes);

    let mut remaining_outnames = outnames.iter();
    let mut current_out: Option<BufWriter<File>> = None;
    let mut filename = Vec::<u8>::new();

    loop {
        // filename
        filename.clear();
        let mut b = [0u8; 1];
        match f.read(&mut b) {
            Ok(0) => break,
            Ok(_) => {
                if b[0] != 0 {
                    filename.push(b[0]);
                    f.read_until(0, &mut filename)?;
                    if let Some(&0) = filename.last() { filename.pop(); }
                } else {
                    // zero already - new block continues current file
                }
            }
            Err(e) => bail!("I/O error while reading archive: {}", e),
        }

        if !filename.is_empty() {
            // open output file, possibly renamed
            if let Some(ref mut w) = current_out {
                w.flush()?;
            }
            current_out = None;
            let default_name = String::from_utf8_lossy(&filename).to_string();
            let outname = if let Some(user) = remaining_outnames.next() {
                user.clone()
            } else {
                PathBuf::from(default_name)
            };
            // no overwrite
            if outname.exists() {
                eprintln!("\nCannot overwrite file, skipping: {}", outname.display());
            } else {
                match OpenOptions::new().write(true).create_new(true).open(&outname) {
                    Ok(fh) => {
                        current_out = Some(BufWriter::with_capacity(1 << 20, fh));
                        eprint!("\n{} ", outname.display());
                    }
                    Err(_) => eprint!("\nCannot create file: {} ", outname.display()),
                }
            }
        }

        // read block header
        let mut m = [0u8; 1];
        f.read_exact(&mut m)?;
        let mode = if !filename.is_empty() {
            // After a new filename, the next byte must be the block-start marker 0x00.
            if m[0] != 0 {
                bail!(
                    "Archive corrupted: expected block marker 0 after filename at {}",
                    f.get_ref().stream_position()? - 1
                );
            }
            // Read the actual mode byte now.
            f.read_exact(&mut m)?;
            m[0]
        } else {
            m[0]
        };
        let usize = read_u32_be(&mut f)? as usize;
        let csize = read_u32_be(&mut f)? as usize;
        match mode {
            b's' => {
                // write via a single &mut dyn Write (sink if None)
                let mut sink = io::sink();
                let w: &mut dyn Write = if let Some(ref mut out) = current_out {
                    out
                } else {
                    &mut sink
                };
                unstore(&mut f, w, usize, csize)?;
            }
            b'c' => {
                // Decompress exactly usize bytes using range coder
                let mut in_remaining = csize;
                // Prepare decoder state: read first 4 bytes
                let mut x: u32 = 0;
                {
                    let mut hdr = [0u8; 4];
                    f.read_exact(&mut hdr)?;
                    in_remaining -= 4;
                    for b in hdr {
                        x = (x << 8) | b as u32;
                    }
                }
                let mut x1: u32 = 0;
                let mut x2: u32 = 0xFFFF_FFFF;

                let read_byte = |f: &mut BufReader<File>| -> Result<u8> {
                    let mut bb = [0u8; 1];
                    f.read_exact(&mut bb)?;
                    Ok(bb[0])
                };

                let mut outleft = usize;
                // writer selection
                let mut sink = io::sink();
                let w: &mut dyn Write = if let Some(ref mut out) = current_out {
                    out
                } else {
                    &mut sink
                };

                while outleft > 0 {
                    // bit
                    let p = predictor.p(&mut lzp);
                    let p_adj = p + ((p < 2048) as i32);
                    let range = x2.wrapping_sub(x1);
                    let xmid = x1
                        .wrapping_add((range >> 12).wrapping_mul(p_adj as u32)
                        .wrapping_add(((range & 0xFFF).wrapping_mul(p_adj as u32)) >> 12));
                    let y = if x <= xmid { 1 } else { 0 };
                    if y != 0 { x2 = xmid; } else { x1 = xmid.wrapping_add(1); }
                    predictor.update(y, &mut lzp);
                    while ((x1 ^ x2) & 0xFF00_0000) == 0 {
                        let b = read_byte(&mut f)?;
                        x = (x << 8) | b as u32;
                        in_remaining -= 1;
                        x1 <<= 8;
                        x2 = (x2 << 8) | 255;
                    }

                    let c = if y == 0 {
                        let mut c: i32 = 1;
                        for _ in 0..8 {
                            let p = predictor.p(&mut lzp);
                            let p_adj = p + ((p < 2048) as i32);
                            let range = x2.wrapping_sub(x1);
                            let xmid = x1
                                .wrapping_add((range >> 12).wrapping_mul(p_adj as u32)
                                .wrapping_add(((range & 0xFFF).wrapping_mul(p_adj as u32)) >> 12));
                            let yb = if x <= xmid { 1 } else { 0 };
                            if yb != 0 { x2 = xmid; } else { x1 = xmid.wrapping_add(1); }
                            predictor.update(yb, &mut lzp);
                            while ((x1 ^ x2) & 0xFF00_0000) == 0 {
                                let b = read_byte(&mut f)?;
                                x = (x << 8) | b as u32;
                                in_remaining -= 1;
                                x1 <<= 8;
                                x2 = (x2 << 8) | 255;
                            }
                            c = (c << 1) + (yb as i32);
                        }
                        (c & 255) as u8
                    } else {
                        lzp.c() as u8
                    };

                    w.write_all(&[c])?;
                    lzp.update(c);
                    outleft -= 1;
                }

                // consume any leftover compressed bytes (should be zero)
                if in_remaining > 0 {
                    f.seek(SeekFrom::Current(in_remaining as i64))?;
                }
            }
            _ => bail!("Unsupported block mode {} at {}", mode, f.get_ref().stream_position()?),
        }
    }

    if let Some(mut w) = current_out.take() {
        w.flush()?;
    }
    eprint!("\n");
    Ok(())
}

#[inline(always)]
fn read_u32_be<R: Read>(r: &mut R) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn unstore<R: Read>(r: &mut R, out: &mut dyn Write, usize: usize, csize: usize) -> Result<()> {
    if usize != csize {
        bail!("Bad archive format: usize={} csize={}", usize, csize);
    }
    let remaining = csize as u64;
    // Limit reader to exactly csize bytes and copy to writer
    let mut limited = r.take(remaining);
    let copied = io::copy(&mut limited, out)?;
    if copied != remaining {
        bail!("Short read in store block: expected {} got {}", remaining, copied);
    }
    Ok(())
}
