use clap::{Parser, Subcommand, ArgAction};
use std::path::PathBuf;
use paq9a_rs::{ArchiveOptions, create_archive, extract_archive, list_archive, MemLevel, Progress};

#[derive(Parser)]
#[command(name="paq9a", version, about="Rust port of paq9a (2007)")]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create archive and compress named files
    A {
        /// Output archive path
        archive: PathBuf,
        /// Memory level (-1..-9), default -7
        #[arg(short='1', action=ArgAction::SetTrue)]
        l1: bool,
        #[arg(short='2', action=ArgAction::SetTrue)]
        l2: bool,
        #[arg(short='3', action=ArgAction::SetTrue)]
        l3: bool,
        #[arg(short='4', action=ArgAction::SetTrue)]
        l4: bool,
        #[arg(short='5', action=ArgAction::SetTrue)]
        l5: bool,
        #[arg(short='6', action=ArgAction::SetTrue)]
        l6: bool,
        #[arg(short='7', action=ArgAction::SetTrue)]
        l7: bool,
        #[arg(short='8', action=ArgAction::SetTrue)]
        l8: bool,
        #[arg(short='9', action=ArgAction::SetTrue)]
        l9: bool,

        /// Store (no compression)
        #[arg(short='s', action=ArgAction::SetTrue)]
        store: bool,
        /// Compress (default)
        #[arg(short='c', action=ArgAction::SetTrue)]
        compress: bool,

        /// Verbose progress
        #[arg(long="verbose", action=ArgAction::SetTrue)]
        verbose: bool,

        /// Input files
        files: Vec<PathBuf>,
    },

    /// Extract from archive (optionally renaming in order)
    X {
        archive: PathBuf,
        /// Optional target names to rename in order
        outnames: Vec<PathBuf>,
    },

    /// List contents
    L {
        archive: PathBuf,
    }
}

fn pick_mem_level(cli: &Commands) -> u8 {
    match cli {
        Commands::A { l1, l2, l3, l4, l5, l6, l7, l8, l9, .. } => {
            if *l1 {1} else if *l2 {2} else if *l3 {3} else if *l4 {4} else if *l5 {5}
            else if *l6 {6} else if *l7 {7} else if *l8 {8} else if *l9 {9} else {7}
        }
        _ => 7
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match &cli.cmd {
        Commands::A { archive, store, compress: _, verbose, files, .. } => {
            if files.is_empty() {
                anyhow::bail!("No input files provided");
            }
            let level = pick_mem_level(&cli.cmd);
            let opts = ArchiveOptions {
                mem: MemLevel::new(level)?,
                store: *store,
                progress: if *verbose { Progress::verbose() } else { Progress::none() },
            };
            create_archive(archive, opts, files)?;
        }
        Commands::X { archive, outnames } => {
            extract_archive(archive, outnames)?;
        }
        Commands::L { archive } => {
            let mut out = std::io::BufWriter::new(std::io::stdout());
            list_archive(archive, &mut out)?;
        }
    }
    Ok(())
}
