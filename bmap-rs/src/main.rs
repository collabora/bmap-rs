use anyhow::{Context, Result, anyhow, bail, ensure};
use async_compression::futures::bufread::{
    BzDecoder, GzipDecoder, Lz4Decoder, LzmaDecoder, XzDecoder, ZlibDecoder, ZstdDecoder,
};
use bmap_parser::{AsyncDiscarder, Bmap, Discarder, SeekForward};
use bzip2::read::BzDecoder as BzSyncDecoder;
use clap::{Arg, ArgAction, Command, arg, command};
use flate2::read::{GzDecoder, ZlibDecoder as ZlibSyncDecoder};
use futures::TryStreamExt;
use futures::io::{AsyncRead, AsyncReadExt};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use liblzma::read::XzDecoder as XzSyncDecoder;
use liblzma::stream::Stream as LzmaStream;
use lz4_flex::frame::FrameDecoder as Lz4SyncDecoder;
use nix::unistd::ftruncate;
use reqwest::{Response, Url};
use std::ffi::OsStr;
use std::fmt::Write;
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::io::AsFd;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tar::Header as TarHeader;
use tokio_util::compat::TokioAsyncReadCompatExt;
use zstd::stream::read::Decoder as ZstdSyncDecoder;

const TAR_BLOCK_SIZE: u64 = 512;

fn tar_padded_size(size: u64) -> u64 {
    size.div_ceil(TAR_BLOCK_SIZE) * TAR_BLOCK_SIZE
}

/// Compression algorithm applied to an image, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    Lzma,
    Lz4,
    Zlib,
}

/// The on-disk format of an image, as derived from its file name: the
/// compression algorithm used (if any) and whether the (decompressed)
/// content is wrapped in a tar archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Format {
    compression: Compression,
    tar: bool,
}

/// Determine the compression/archive format of an image from its file name.
/// Returns `None` if the file name doesn't have a recognized extension.
fn classify_format(path: &Path) -> Option<Format> {
    let name = path.file_name().and_then(OsStr::to_str)?;
    let lower = name.to_lowercase();
    const FORMATS: &[(&str, Compression, bool)] = &[
        (".tar.gz", Compression::Gzip, true),
        (".tgz", Compression::Gzip, true),
        (".tar.bz2", Compression::Bzip2, true),
        (".tbz2", Compression::Bzip2, true),
        (".tbz", Compression::Bzip2, true),
        (".tb2", Compression::Bzip2, true),
        (".tar.xz", Compression::Xz, true),
        (".txz", Compression::Xz, true),
        (".tar.zst", Compression::Zstd, true),
        (".tzst", Compression::Zstd, true),
        (".tar.lz4", Compression::Lz4, true),
        (".tar.lzma", Compression::Lzma, true),
        (".tar.zz", Compression::Zlib, true),
        (".tar", Compression::None, true),
        (".gz", Compression::Gzip, false),
        (".bz2", Compression::Bzip2, false),
        (".xz", Compression::Xz, false),
        (".zst", Compression::Zstd, false),
        (".lzma", Compression::Lzma, false),
        (".lz4", Compression::Lz4, false),
        (".zz", Compression::Zlib, false),
    ];
    FORMATS
        .iter()
        .find(|(suffix, ..)| lower.ends_with(suffix))
        .map(|(_, compression, tar)| Format {
            compression: *compression,
            tar: *tar,
        })
}

/// A reader that extracts the first regular file found in a tar stream,
/// skipping over any other entries (directories, GNU long name/pax headers,
/// ...) it may encounter first.
struct TarFileReader<R> {
    inner: R,
    remaining: u64,
}

impl<R: Read> TarFileReader<R> {
    fn new(mut inner: R) -> Result<Self> {
        loop {
            let mut block = [0u8; TAR_BLOCK_SIZE as usize];
            inner
                .read_exact(&mut block)
                .context("Failed to read tar header")?;
            if block.iter().all(|&b| b == 0) {
                bail!("No file found in tar archive");
            }
            let header = TarHeader::from_byte_slice(&block);
            let size = header.size().context("Failed to read tar entry size")?;
            if header.entry_type().is_file() && size > 0 {
                return Ok(Self {
                    inner,
                    remaining: size,
                });
            }
            io::copy(
                &mut (&mut inner).take(tar_padded_size(size)),
                &mut io::sink(),
            )
            .context("Failed to skip tar entry")?;
        }
    }
}

impl<R: Read> Read for TarFileReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let max = self.remaining.min(buf.len() as u64) as usize;
        if max == 0 {
            return Ok(0);
        }
        let n = self.inner.read(&mut buf[..max])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Truncated tar entry",
            ));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// Async equivalent of [`TarFileReader`].
struct AsyncTarFileReader<R> {
    inner: R,
    remaining: u64,
}

impl<R: AsyncRead + Unpin> AsyncTarFileReader<R> {
    async fn new(mut inner: R) -> Result<Self> {
        loop {
            let mut block = [0u8; TAR_BLOCK_SIZE as usize];
            inner
                .read_exact(&mut block)
                .await
                .context("Failed to read tar header")?;
            if block.iter().all(|&b| b == 0) {
                bail!("No file found in tar archive");
            }
            let header = TarHeader::from_byte_slice(&block);
            let size = header.size().context("Failed to read tar entry size")?;
            if header.entry_type().is_file() && size > 0 {
                return Ok(Self {
                    inner,
                    remaining: size,
                });
            }
            futures::io::copy(
                &mut (&mut inner).take(tar_padded_size(size)),
                &mut futures::io::sink(),
            )
            .await
            .context("Failed to skip tar entry")?;
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for AsyncTarFileReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.remaining == 0 {
            return Poll::Ready(Ok(0));
        }
        let max = self.remaining.min(buf.len() as u64) as usize;
        if max == 0 {
            return Poll::Ready(Ok(0));
        }
        match Pin::new(&mut self.inner).poll_read(cx, &mut buf[..max]) {
            Poll::Ready(Ok(0)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Truncated tar entry",
            ))),
            Poll::Ready(Ok(n)) => {
                self.remaining -= n as u64;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }
}

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
    let format = match classify_format(path) {
        Some(format) => format,
        None => return Ok(Decoder::new(f)),
    };
    let decompressed: Box<dyn Read> = match format.compression {
        Compression::None => Box::new(f),
        Compression::Gzip => Box::new(GzDecoder::new(f)),
        Compression::Bzip2 => Box::new(BzSyncDecoder::new(f)),
        Compression::Xz => Box::new(XzSyncDecoder::new(f)),
        Compression::Zstd => Box::new(ZstdSyncDecoder::new(f)?),
        Compression::Lzma => {
            let stream = LzmaStream::new_lzma_decoder(u64::MAX)?;
            Box::new(XzSyncDecoder::new_stream(f, stream))
        }
        Compression::Lz4 => Box::new(Lz4SyncDecoder::new(f)),
        Compression::Zlib => Box::new(ZlibSyncDecoder::new(f)),
    };
    if format.tar {
        Ok(Decoder::new(Discarder::new(TarFileReader::new(
            decompressed,
        )?)))
    } else {
        Ok(Decoder::new(Discarder::new(decompressed)))
    }
}

async fn wrap_async_decoder<S>(path: &Path, stream: S) -> Result<Box<dyn AsyncRead + Unpin + Send>>
where
    S: futures::io::AsyncBufRead + Unpin + Send + 'static,
{
    let format =
        classify_format(path).ok_or_else(|| anyhow!("Image file format not implemented"))?;
    let decompressed: Box<dyn AsyncRead + Unpin + Send> = match format.compression {
        Compression::None => Box::new(stream),
        Compression::Gzip => Box::new(GzipDecoder::new(stream)),
        Compression::Bzip2 => Box::new(BzDecoder::new(stream)),
        Compression::Xz => Box::new(XzDecoder::new(stream)),
        Compression::Zstd => {
            let mut zstd = ZstdDecoder::new(stream);
            // async_compression's ZstdDecoder defaults to decoding only the
            // first frame, unlike zstd::stream::read::Decoder (used on the
            // sync path) which decodes all concatenated frames. Without this,
            // a multi-frame image (e.g. produced by pzstd) would be silently
            // truncated after the first frame.
            zstd.multiple_members(true);
            Box::new(zstd)
        }
        Compression::Lzma => Box::new(LzmaDecoder::new(stream)),
        Compression::Lz4 => Box::new(Lz4Decoder::new(stream)),
        Compression::Zlib => Box::new(ZlibDecoder::new(stream)),
    };
    if format.tar {
        Ok(Box::new(AsyncTarFileReader::new(decompressed).await?))
    } else {
        Ok(decompressed)
    }
}

async fn setup_remote_input(url: Url) -> Result<Response> {
    let path = PathBuf::from(url.path());
    match classify_format(&path) {
        Some(_) => reqwest::get(url).await.map_err(anyhow::Error::new),
        None => bail!("Image file format not implemented"),
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

    let path = PathBuf::from(source.path());
    let res = setup_remote_input(source).await?;
    let stream = res
        .bytes_stream()
        .map_err(std::io::Error::other)
        .into_async_read();
    let reader = wrap_async_decoder(&path, stream).await?;
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

    let path = PathBuf::from(source.path());
    let res = setup_remote_input(source).await?;
    let stream = res
        .bytes_stream()
        .map_err(std::io::Error::other)
        .into_async_read();
    let reader = wrap_async_decoder(&path, stream).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tar::{Builder, EntryType, Header};

    /// Build an in-memory tar archive from `(name, data, entry_type)` entries.
    fn tar_bytes(entries: &[(&str, &[u8], EntryType)]) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());
        for (name, data, entry_type) in entries {
            let mut header = Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_entry_type(*entry_type);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn classify_plain_compression() {
        for (name, compression) in [
            ("disk.img.gz", Compression::Gzip),
            ("disk.img.bz2", Compression::Bzip2),
            ("disk.img.xz", Compression::Xz),
            ("disk.img.zst", Compression::Zstd),
            ("disk.img.lzma", Compression::Lzma),
            ("disk.img.lz4", Compression::Lz4),
            ("disk.img.zz", Compression::Zlib),
        ] {
            assert_eq!(
                classify_format(Path::new(name)),
                Some(Format {
                    compression,
                    tar: false
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn classify_tar_variants() {
        for (name, compression) in [
            ("disk.tar", Compression::None),
            ("disk.tar.gz", Compression::Gzip),
            ("disk.tgz", Compression::Gzip),
            ("disk.tar.bz2", Compression::Bzip2),
            ("disk.tbz2", Compression::Bzip2),
            ("disk.tar.xz", Compression::Xz),
            ("disk.tar.zst", Compression::Zstd),
            ("disk.tar.lz4", Compression::Lz4),
            ("disk.tar.lzma", Compression::Lzma),
            ("disk.tar.zz", Compression::Zlib),
        ] {
            assert_eq!(
                classify_format(Path::new(name)),
                Some(Format {
                    compression,
                    tar: true
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn classify_tar_suffix_wins_over_bare() {
        // ".tar.gz" also ends with ".gz"; the tar-aware suffix must win.
        assert_eq!(
            classify_format(Path::new("disk.tar.gz")),
            Some(Format {
                compression: Compression::Gzip,
                tar: true
            })
        );
    }

    #[test]
    fn classify_case_insensitive() {
        assert_eq!(
            classify_format(Path::new("DISK.TAR.GZ")),
            Some(Format {
                compression: Compression::Gzip,
                tar: true
            })
        );
    }

    #[test]
    fn classify_unknown_and_missing() {
        assert_eq!(classify_format(Path::new("disk.img")), None);
        assert_eq!(classify_format(Path::new("disk")), None);
        assert_eq!(classify_format(Path::new("/")), None);
    }

    #[test]
    fn tar_reader_extracts_single_file() {
        let payload = b"the actual disk image contents";
        let archive = tar_bytes(&[("disk.img", payload, EntryType::Regular)]);
        let mut out = Vec::new();
        TarFileReader::new(archive.as_slice())
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn tar_reader_skips_leading_dir_and_empty_file() {
        let payload = b"real image";
        let archive = tar_bytes(&[
            ("subdir/", b"", EntryType::Directory),
            ("subdir/empty", b"", EntryType::Regular),
            ("subdir/disk.img", payload, EntryType::Regular),
        ]);
        let mut out = Vec::new();
        TarFileReader::new(archive.as_slice())
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn tar_reader_errors_on_empty_archive() {
        let archive = tar_bytes(&[]);
        assert!(TarFileReader::new(archive.as_slice()).is_err());
    }

    #[test]
    fn tar_reader_errors_on_truncated_entry() {
        let payload = vec![0xabu8; 2000];
        let archive = tar_bytes(&[("disk.img", &payload, EntryType::Regular)]);
        // Keep the header block plus only part of the file data.
        let truncated = &archive[..TAR_BLOCK_SIZE as usize + 500];
        let mut reader = TarFileReader::new(truncated).unwrap();
        let mut out = Vec::new();
        assert!(reader.read_to_end(&mut out).is_err());
    }

    #[tokio::test]
    async fn async_tar_reader_extracts_single_file() {
        let payload = b"async disk image contents";
        let archive = tar_bytes(&[("disk.img", payload, EntryType::Regular)]);
        let mut reader = AsyncTarFileReader::new(futures::io::Cursor::new(archive))
            .await
            .unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, payload);
    }
}
