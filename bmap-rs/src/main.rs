use anyhow::{anyhow, bail, Context, Result};
use bmap::helpers::{find_bmap, setup_input, setup_output};
use bmap::Bmap;
use std::fs::File;
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
    if !c.image.exists() {
        bail!("Image file doesn't exist")
    }

    let bmap = find_bmap(&c.image).ok_or_else(|| anyhow!("Couldn't find bmap file"))?;
    println!("Found bmap file: {}", bmap.display());

    let mut b = File::open(&bmap).context("Failed to open bmap file")?;
    let mut xml = String::new();
    b.read_to_string(&mut xml)?;

    let bmap = Bmap::from_xml(&xml)?;
    let mut output = setup_output(&c.dest, &bmap)?;
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
