use anyhow::{anyhow, bail, Context, Result};
use bmap::{Bmap, Discarder};
use flate2::read::GzDecoder;
use nix::unistd::ftruncate;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
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

    ftruncate(output.as_raw_fd(), bmap.image_size() as i64).context("Failed to truncate file")?;

    let mut f = File::open(&c.image)?;
    match c.image.extension().map(OsStr::to_str).flatten() {
        Some("gz") => {
            let gz = GzDecoder::new(f);
            let mut input = Discarder::new(gz);
            bmap::copy(&mut input, &mut output, &bmap)?;
        }
        _ => bmap::copy(&mut f, &mut output, &bmap)?,
    }
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
