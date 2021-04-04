use anyhow::{bail, Context, Result};
use bmap::Bmap;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
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

    bmap::copy(&mut input, &mut output, &bmap).unwrap();
    println!("Done: Syncing");
    output.sync_all();

    Ok(())
}

fn main() -> Result<()> {
    let opts = Opts::from_args();

    match opts.command {
        Command::Copy(c) => copy(c),
    }
}
