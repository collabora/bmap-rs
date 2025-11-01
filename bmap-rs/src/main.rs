use anyhow::{Context, Result, anyhow, bail, ensure};
use async_compression::futures::bufread::GzipDecoder;
use bmap_parser::{AsyncDiscarder, Bmap, Discarder, SeekForward};
use clap::{Arg, ArgAction, Command, arg, command};
use flate2::read::GzDecoder;
use futures::TryStreamExt;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use nix::unistd::ftruncate;
use reqwest::{Response, Url};
use std::ffi::OsStr;
use std::fmt::Write;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsFd;
use std::path::{Path, PathBuf};
use tokio_util::compat::TokioAsyncReadCompatExt;

#[derive(Debug)]
enum Image {
    Path(PathBuf),
    Url(Url),
}

#[derive(Debug)]
struct Copy {
    image: Image,
    dest: PathBuf,
    nobmap: bool,
}

#[derive(Debug)]

enum Subcommand {
    Copy(Copy),
}

#[derive(Debug)]
struct Opts {
    command: Subcommand,
}

impl Opts {
    fn parser() -> Opts {
        let matches = command!()
            .propagate_version(true)
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("copy")
                    .about("Copy image to block device or file")
                    .arg(arg!([IMAGE]).required(true))
                    .arg(arg!([DESTINATION]).required(true))
                    .arg(
                        Arg::new("nobmap")
                            .short('n')
                            .long("nobmap")
                            .action(ArgAction::SetTrue),
                    ),
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
                        dest: PathBuf::from(sub_matches.get_one::<String>("DESTINATION").unwrap()),
                        nobmap: sub_matches.get_flag("nobmap"),
                    }
                }),
            },
            _ => unreachable!(
                "Exhausted list of subcommands and subcommand_required prevents `None`"
            ),
        }
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

fn find_remote_bmap(mut url: Url) -> Result<Url> {
    let mut path = PathBuf::from(url.path());
    path.set_extension("bmap");
    url.set_path(path.to_str().unwrap());
    Ok(url)
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

fn setup_local_input(path: &Path) -> Result<Decoder> {
    let f = File::open(path)?;
    match path.extension().and_then(OsStr::to_str) {
        Some("gz") => {
            let gz = GzDecoder::new(f);
            Ok(Decoder::new(Discarder::new(gz)))
        }
        _ => Ok(Decoder::new(f)),
    }
}

async fn setup_remote_input(url: Url) -> Result<Response> {
    match PathBuf::from(url.path())
        .extension()
        .and_then(OsStr::to_str)
    {
        Some("gz") => reqwest::get(url).await.map_err(anyhow::Error::new),
        None => bail!("No file extension found"),
        _ => bail!("Image file format not implemented"),
    }
}

fn setup_progress_bar(bmap: &Bmap) -> ProgressBar {
    let pb = ProgressBar::new(bmap.total_mapped_size());
    pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-"));
    pb
}

fn setup_spinner() -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
    pb
}

fn setup_output<T: AsFd>(output: &T, bmap: &Bmap, metadata: std::fs::Metadata) -> Result<()> {
    if metadata.is_file() {
        ftruncate(output.as_fd(), bmap.image_size() as i64).context("Failed to truncate file")?;
    }
    Ok(())
}

async fn copy(c: Copy) -> Result<()> {
    if c.nobmap {
        return match c.image {
            Image::Path(path) => copy_local_input_nobmap(path, c.dest),
            Image::Url(url) => copy_remote_input_nobmap(url, c.dest).await,
        };
    }
    match c.image {
        Image::Path(path) => copy_local_input(path, c.dest),
        Image::Url(url) => copy_remote_input(url, c.dest).await,
    }
}

fn copy_local_input(source: PathBuf, destination: PathBuf) -> Result<()> {
    ensure!(source.exists(), "Image file doesn't exist");
    let bmap = find_bmap(&source).ok_or_else(|| anyhow!("Couldn't find bmap file"))?;
    println!("Found bmap file: {}", bmap.display());

    let mut b = File::open(&bmap).context("Failed to open bmap file")?;
    let mut xml = String::new();
    b.read_to_string(&mut xml)?;

    let bmap = Bmap::from_xml(&xml)?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)?;

    setup_output(&output, &bmap, output.metadata()?)?;

    let mut input = setup_local_input(&source)?;
    let pb = setup_progress_bar(&bmap);
    bmap_parser::copy(&mut input, &mut pb.wrap_write(&output), &bmap)?;
    pb.finish_and_clear();

    println!("Done: Syncing...");
    output.sync_all()?;

    Ok(())
}

async fn copy_remote_input(source: Url, destination: PathBuf) -> Result<()> {
    let bmap_url = find_remote_bmap(source.clone())?;

    let xml = reqwest::get(bmap_url.clone()).await?.text().await?;
    println!("Found bmap file: {}", bmap_url);

    let bmap = Bmap::from_xml(&xml)?;
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)
        .await?;

    setup_output(&output, &bmap, output.metadata().await?)?;

    let res = setup_remote_input(source).await?;
    let stream = res
        .bytes_stream()
        .map_err(std::io::Error::other)
        .into_async_read();
    let reader = GzipDecoder::new(stream);
    let mut input = AsyncDiscarder::new(reader);
    let pb = setup_progress_bar(&bmap);
    bmap_parser::copy_async(
        &mut input,
        &mut pb.wrap_async_write(&mut output).compat(),
        &bmap,
    )
    .await?;
    pb.finish_and_clear();

    println!("Done: Syncing...");
    output.sync_all().await?;
    Ok(())
}

fn copy_local_input_nobmap(source: PathBuf, destination: PathBuf) -> Result<()> {
    ensure!(source.exists(), "Image file doesn't exist");

    let output = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)?;

    let mut input = setup_local_input(&source)?;

    let pb = setup_spinner();
    bmap_parser::copy_nobmap(&mut input, &mut pb.wrap_write(&output))?;
    pb.finish_and_clear();

    println!("Done: Syncing...");
    output.sync_all().expect("Sync failure");

    Ok(())
}

async fn copy_remote_input_nobmap(source: Url, destination: PathBuf) -> Result<()> {
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)
        .await?;

    let res = setup_remote_input(source).await?;
    let stream = res
        .bytes_stream()
        .map_err(std::io::Error::other)
        .into_async_read();
    let reader = GzipDecoder::new(stream);
    let mut input = AsyncDiscarder::new(reader);
    let pb = setup_spinner();
    bmap_parser::copy_async_nobmap(&mut input, &mut pb.wrap_async_write(&mut output).compat())
        .await?;
    pb.finish_and_clear();

    println!("Done: Syncing...");
    output.sync_all().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Opts::parser();

    match opts.command {
        Subcommand::Copy(c) => copy(c).await,
    }
}
