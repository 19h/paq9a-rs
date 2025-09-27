use std::fmt;

/// Memory level: '1'..'9' as in the original program.
/// MEM bytes = 1 << (22 + level)
#[derive(Clone, Copy, Debug)]
pub struct MemLevel {
    pub level: u8, // 1..=9
}

impl MemLevel {
    pub fn new(level: u8) -> anyhow::Result<Self> {
        anyhow::ensure!((1..=9).contains(&level), "mem level must be 1..=9");
        Ok(Self { level })
    }

    /// Returns total bytes of MEM = 1 << (22 + level)
    pub fn bytes(self) -> usize {
        let shift = 22u32 + self.level as u32;
        1usize << shift
    }

    /// Returns the header byte to be stored in the archive ('1'..'9').
    pub fn header_char(self) -> u8 {
        b'0' + self.level
    }

    pub fn from_header_char(c: u8) -> anyhow::Result<Self> {
        anyhow::ensure!((b'1'..=b'9').contains(&c), "invalid mem header byte");
        let level = (c - b'0') as u8;
        Self::new(level)
    }
}

impl fmt::Display for MemLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "-{}", self.level)
    }
}

/// Optional progress callback.
#[derive(Clone)]
pub struct Progress {
    pub verbose: bool,
}

impl Progress {
    pub fn none() -> Self {
        Self { verbose: false }
    }
    pub fn verbose() -> Self {
        Self { verbose: true }
    }
    pub fn log(&self, s: impl AsRef<str>) {
        if self.verbose {
            eprintln!("{}", s.as_ref());
        }
    }
}
