use anyhow::{anyhow, bail, Context, Result};
use bmap::{Bmap, Discarder, SeekForward};
use flate2::read::GzDecoder;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct Copy {
    image: PathBuf,
    dest: PathBuf,
}

#[derive(StructOpt, Debug)]
enum Command {
    Copy(Copy),
}

#[derive(Debug, StructOpt)]
struct Opts {
    #[structopt(subcommand)]
    command: Command,
}

fn append(path: PathBuf) -> PathBuf {
    let mut p = path.into_os_string();
    p.push(".bmap");
    p.into()
}

fn find_bmap(img: &Path) -> Option<PathBuf> {
    let mut bmap = img.to_path_buf();
    loop {
        bmap = append(bmap);
        if bmap.exists() {
            return Some(bmap);
        }

        // Drop .bmap
        bmap.set_extension("");
        bmap.extension()?;
        // Drop existing orignal extension part
        bmap.set_extension("");
    }
}

trait ReadSeekForward: SeekForward + Read {}
impl<T: Read + SeekForward> ReadSeekForward for T {}

struct Decoder {
    inner: Box<dyn ReadSeekForward>,
}

impl Decoder {
    fn new<T: ReadSeekForward + 'static>(inner: T) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }
}

impl Read for Decoder {
    fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(data)
    }
}

impl SeekForward for Decoder {
    fn seek_forward(&mut self, forward: u64) -> std::io::Result<()> {
        self.inner.seek_forward(forward)
    }
}

fn setup_input(path: &Path) -> Result<Decoder> {
    let f = File::open(path)?;
    match path.extension().and_then(OsStr::to_str) {
        Some("gz") => {
            let gz = GzDecoder::new(f);
            Ok(Decoder::new(Discarder::new(gz)))
        }
        _ => Ok(Decoder::new(f)),
    }
}

fn copy(c: Copy) -> Result<()> {
    if !c.image.exists() {
        bail!("Image file doesn't exist")
    }

    let bmap = find_bmap(&c.image).ok_or_else(|| anyhow!("Couldn't find bmap file"))?;
    println!("Found bmap file: {}", bmap.display());

    let mut b = File::open(&bmap).context("Failed to open bmap file")?;
    let mut xml = String::new();
    b.read_to_string(&mut xml)?;

    let bmap = Bmap::from_xml(&xml)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(c.dest)?;

    let mut input = setup_input(&c.image)?;
    bmap::copy(&mut input, &mut output, &bmap)?;
    println!("Done: Syncing...");
    output.sync_all().expect("Sync failure");

    Ok(())
}

fn main() -> Result<()> {
    let opts = Opts::from_args();

    match opts.command {
        Command::Copy(c) => copy(c),
    }
}
