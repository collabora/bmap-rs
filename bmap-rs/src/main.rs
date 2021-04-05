use anyhow::{Context, Result};
use bmap::Bmap;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use structopt::StructOpt;
use nix::unistd::ftruncate;

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

fn copy(c: Copy) -> Result<()> {
    let mut bmap = c.image.clone();
    bmap.set_extension("img.bmap");
    println!("{:?}", bmap);
    let mut b = File::open(bmap).context("Failed to open bmap file")?;
    let mut xml = String::new();
    b.read_to_string(&mut xml)?;

    let bmap = Bmap::from_xml(&xml)?;
    println!("{:?}", b);
    let mut input = File::open(c.image)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(c.dest)?;

    ftruncate(output.as_raw_fd(), bmap.image_size() as i64).context("Failed to truncate file")?;
    bmap::copy(&mut input, &mut output, &bmap).unwrap();
    println!("Done: Syncing");
    output.sync_all().expect("Sync failure");

    Ok(())
}

fn main() -> Result<()> {
    let opts = Opts::from_args();

    match opts.command {
        Command::Copy(c) => copy(c),
    }
}
