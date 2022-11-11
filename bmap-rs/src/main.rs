use anyhow::{anyhow, bail, Context, Result};
use bmap::{Bmap, Discarder, SeekForward};
use clap::{arg, command, Command};
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use nix::unistd::ftruncate;
use reqwest::Url;
use std::ffi::OsStr;
use std::fmt::Write;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug)]
enum Image {
    Path(PathBuf),
    Url(Url),
}

impl Image {
    fn path(self) -> Result<PathBuf> {
        if let Image::Path(c) = self {
            Ok(c)
        } else {
            bail!("Not a path")
        }
    }

    // Commented to avoid unused code warning
    //fn url(self) -> Result<Url> {
    //    if let Image::Url(d) = self { Ok(d) } else { bail!("Not a url") }
    //}
}

#[derive(Debug)]
struct Copy {
    image: Image,
    dest: PathBuf,
}

#[derive(Debug)]

enum Subcommand {
    Copy(Copy),
}

#[derive(Debug)]
struct Opts {
    command: Subcommand,
}

fn parser() -> Opts {
    let matches = command!()
        .propagate_version(true)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("copy")
                .about("Copy disk image to destiny")
                .arg(arg!([IMAGE]).required(true))
                .arg(arg!([DESTINY]).required(true)),
        )
        .get_matches();
    match matches.subcommand() {
        Some(("copy", sub_matches)) => Opts {
            command: Subcommand::Copy({
                Copy {
                    image: match Url::parse(sub_matches.get_one::<String>("IMAGE").unwrap()) {
                        Ok(url) => Image::Url(url),
                        Err(_) => Image::Path(PathBuf::from(
                            sub_matches.get_one::<String>("IMAGE").unwrap(),
                        )),
                    },
                    dest: PathBuf::from(sub_matches.get_one::<String>("DESTINY").unwrap()),
                }
            }),
        },
        _ => unreachable!("Exhausted list of subcommands and subcommand_required prevents `None`"),
    }
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
    let image = c.image.path()?;
    if !image.exists() {
        bail!("Image file doesn't exist")
    }

    let bmap = find_bmap(&image).ok_or_else(|| anyhow!("Couldn't find bmap file"))?;
    println!("Found bmap file: {}", bmap.display());

    let mut b = File::open(&bmap).context("Failed to open bmap file")?;
    let mut xml = String::new();
    b.read_to_string(&mut xml)?;

    let bmap = Bmap::from_xml(&xml)?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(c.dest)?;

    if output.metadata()?.is_file() {
        ftruncate(output.as_raw_fd(), bmap.image_size() as i64)
            .context("Failed to truncate file")?;
    }

    let mut input = setup_input(&image)?;
    let pb = ProgressBar::new(bmap.total_mapped_size());
    pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-"));
    bmap::copy(&mut input, &mut pb.wrap_write(&output), &bmap)?;
    pb.finish_and_clear();

    println!("Done: Syncing...");
    output.sync_all().expect("Sync failure");

    Ok(())
}

fn main() -> Result<()> {
    let opts = parser();

    match opts.command {
        Subcommand::Copy(c) => copy(c),
    }
}
