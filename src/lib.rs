//! paq9a_rs: Safe Rust port of Matt Mahoney's paq9a (2007).
//!
//! This library preserves the on-disk archive format and the compression model,
//! including the LZP stage, context-mixing predictor, arithmetic coder, and
//! block framing. The public API exposes high-level archive operations.

mod arithmetic;
mod statetable;
mod statemap;
mod mix;
pub mod hashtable;
pub mod lzp;
pub mod predictor;
mod encoder;
mod archive;
pub mod util;

pub use archive::{create_archive, extract_archive, list_archive, ArchiveOptions};
pub use util::{MemLevel, Progress};
