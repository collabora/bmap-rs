use anyhow::{anyhow, bail, Context, Result};
use bmap::{Bmap, Discarder, SeekForward};
use flate2::read::GzDecoder;
use hyper::body::HttpBody;
use hyper::{Body, Client, Response, Uri};
use nix::unistd::ftruncate;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use structopt::StructOpt;
use tempfile::tempfile;

#[derive(StructOpt, Debug)]
struct Copy {
    image: PathBuf,
    dest: PathBuf,
}
enum Input {
    Remote(Response<Body>),
    Local(Decoder),
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
async fn fetch_url_http(url: hyper::Uri) -> Result<Response<Body>, hyper::Error> {
    let client = Client::new();
    client.get(url).await
}
async fn setup_remote_input(url: Uri, fpath: &Path) -> Result<Input> {
    if fpath.extension().unwrap() != "gz" {
        bail!("Image file format not implemented")
    }
    let input = Input::Remote(match url.scheme_str() {
        Some("https") =>panic!("This feature doesn't work because it needs implementing async enviroment and fetch_url_https function"),
        Some("http") => fetch_url_http(url).await?,
        _ => {
            panic!("url is not http or https");
        }
    });
    Ok(input)
}

fn setup_local_input(path: &Path) -> Result<Input> {
    let f = File::open(path)?;
    match path.extension().and_then(OsStr::to_str) {
        Some("gz") => {
            let gz = GzDecoder::new(f);
            Ok(Input::Local(Decoder::new(Discarder::new(gz))))
        }
        _ => Ok(Input::Local(Decoder::new(f))),
    }
}

async fn copy(c: Copy) -> Result<()> {
    let url = c
        .image
        .to_str()
        .expect("Fail to convert remote path to str")
        .parse::<Uri>()
        .expect("Fail to convert remote path to url");
    let input_type = match url.scheme() {
        Some(_) => setup_remote_input(url, &c.image).await?,
        None => {
            if !c.image.exists() {
                bail!("Image file doesn't exist")
            }
            setup_local_input(&c.image)?
        }
    };

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
    match input_type {
        Input::Local(mut input) => {
            bmap::copy(&mut input, &mut output, &bmap)?;
        }
        Input::Remote(mut res) => {
            while let Some(next) = res.data().await {
                let chunk = next?;
                let mut f = tempfile()?;
                f.write_all(&chunk)?;

                let mut chunk = match c.image.extension().and_then(OsStr::to_str) {
                    Some("gz") => {
                        let gz = GzDecoder::new(f);
                        Decoder::new(Discarder::new(gz))
                    }
                    _ => Decoder::new(f),
                };
                bmap::copy(&mut chunk, &mut output, &bmap)?;
            }
        }
    }

    println!("Done: Syncing...");
    output.sync_all().expect("Sync failure");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Opts::from_args();

    match opts.command {
        Command::Copy(c) => copy(c).await,
    }
}
