use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use bmap::{AsyncSeekForward, Bmap, Discarder, SeekForward};
use flate2::read::GzDecoder;
use hyper::body::HttpBody;
use hyper::{Body, Client, Response, Uri};
use nix::unistd::ftruncate;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use structopt::StructOpt;
use tempfile::tempfile;
use tokio::io::{AsyncRead};

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
#[async_trait]
trait AsyncReadSeekForward: AsyncSeekForward + AsyncRead {}
impl<T: AsyncRead + AsyncSeekForward> AsyncReadSeekForward for T {}
struct AsyncDecoder {
    inner: Box<dyn AsyncReadSeekForward>,
}
impl AsyncDecoder {
    fn new<T: AsyncReadSeekForward + 'static>(inner: T) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }
}
impl AsyncRead for AsyncDecoder {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.poll_read(cx, buf)
    }
}
impl AsyncSeekForward for AsyncDecoder {
    fn async_seek_forward< 'life0, 'async_trait>(& 'life0 mut self,offset:u64) ->  core::pin::Pin<Box<dyn core::future::Future<Output = std::io::Result<()> > + core::marker::Send+ 'async_trait> >where 'life0: 'async_trait,Self: 'async_trait {
        self.async_seek_forward(offset)
    }
}

async fn fetch_url_http(url: hyper::Uri) -> Result<Response<Body>, hyper::Error> {
    let client = Client::new();
    client.get(url.clone()).await
}
async fn setup_remote_input(url: Uri, fpath: &Path) -> Result<Input> {
    if fpath.extension().unwrap() != "gz" {
        bail!("Image file format not implemented")
    }
    let input = Input::Remote(match url.scheme_str() {
        Some("https") =>panic!("This feature doesn't work because it needs implementing async enviroment and fetch_url_https function"),
        Some("http") => fetch_url_http(url).await?,
        _ => {
            bail!("url is not http or https")
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
        Some(_) => setup_remote_input(url.clone(), &c.image).await?,
        None => {
            if !c.image.exists() {
                bail!("Image file doesn't exist")
            }
            setup_local_input(&c.image)?
        }
    };

    let bmap = match input_type {
        Input::Local(_) => find_bmap(&c.image).ok_or_else(|| anyhow!("Couldn't find bmap file"))?,
        Input::Remote(_) => {
            let img_path = url.path();
            find_bmap(Path::new(&img_path[6..]))
                .ok_or_else(|| anyhow!("Couldn't find bmap file"))?
        }
    };

    println!("Found bmap file: {}", bmap.display());

    let mut b = File::open(&bmap).context("Failed to open bmap file")?;
    let mut xml = String::new();
    b.read_to_string(&mut xml)?;

    let bmap = Bmap::from_xml(&xml)?;

    match input_type {
        Input::Local(mut input) => {
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(c.dest)?;

            ftruncate(output.as_raw_fd(), bmap.image_size() as i64)
                .context("Failed to truncate file")?;
            bmap::copy(&mut input, &mut output, &bmap)?;
            println!("Done: Syncing...");
            output.sync_all().expect("Sync failure");
        }
        Input::Remote(mut res) => {
            let mut output = tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(c.dest)
                .await?;

            ftruncate(output.as_raw_fd(), bmap.image_size() as i64)
                .context("Failed to truncate file")?;
            while let Some(next) = res.data().await {
                let chunk = next?;
                let mut f = tempfile()?;
                f.write_all(&chunk)?;

                let mut chunk = match c.image.extension().and_then(OsStr::to_str) {
                    Some("gz") => {
                        let gz = GzDecoder::new(f);
                        AsyncDecoder::new(Discarder::new(gz))
                    }
                    _ => AsyncDecoder::new(f),
                };
                bmap::copy_async(&mut chunk, &mut output, &bmap).await?;
            }
            println!("Done: Syncing...");
            output.sync_all().await.expect("Sync failure");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Opts::from_args();

    match opts.command {
        Command::Copy(c) => copy(c).await,
    }
}
