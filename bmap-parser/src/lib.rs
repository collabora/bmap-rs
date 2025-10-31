mod bmap;
pub use crate::bmap::*;
mod discarder;
pub use crate::discarder::*;
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use futures::TryFutureExt;
use sha2::{Digest, Sha256};
use thiserror::Error;

use std::io::Result as IOResult;
use std::io::{Read, Seek, SeekFrom, Write};

#[cfg(feature = "progress_bar")]
use std::io::Stdout;

/// Trait that can only seek further forwards
pub trait SeekForward {
    fn seek_forward(&mut self, offset: u64) -> IOResult<()>;
}

impl<T: Seek> SeekForward for T {
    fn seek_forward(&mut self, forward: u64) -> IOResult<()> {
        self.seek(SeekFrom::Current(forward as i64))?;
        Ok(())
    }
}

#[async_trait]
pub trait AsyncSeekForward {
    async fn async_seek_forward(&mut self, offset: u64) -> IOResult<()>;
}

#[async_trait]
impl<T: AsyncSeek + Unpin + Send> AsyncSeekForward for T {
    async fn async_seek_forward(&mut self, forward: u64) -> IOResult<()> {
        self.seek(SeekFrom::Current(forward as i64)).await?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum CopyError {
    #[error("Failed to Read: {0}")]
    ReadError(std::io::Error),
    #[error("Failed to Write: {0}")]
    WriteError(std::io::Error),
    #[error("Checksum error")]
    ChecksumError,
    #[error("Unexpected EOF on input")]
    UnexpectedEof,
}

pub fn copy<I, O>(input: &mut I, output: &mut O, map: &Bmap) -> Result<(), CopyError>
where
    I: Read + SeekForward,
    O: Write + SeekForward,
{
    let mut hasher = match map.checksum_type() {
        HashType::Sha256 => Sha256::new(),
    };

    // TODO benchmark a reasonable size for this
    let mut v = vec![0; 8 * 1024 * 1024];

    let buf = v.as_mut_slice();
    let mut position = 0;

    #[cfg(feature = "progress_bar")]
    let num_range = map.block_map().len();
    #[cfg(feature = "progress_bar")]
    let mut idx_range = 1;

    for range in map.block_map() {
        let forward = range.offset() - position;
        input.seek_forward(forward).map_err(CopyError::ReadError)?;
        output
            .seek_forward(forward)
            .map_err(CopyError::WriteError)?;

        let bytes_to_copy = range.length() as usize;
        let mut left = bytes_to_copy;

        #[cfg(feature = "progress_bar")]
        let mut stdout = std::io::stdout();

        while left > 0 {
            let toread = left.min(buf.len());
            let r = input
                .read(&mut buf[0..toread])
                .map_err(CopyError::ReadError)?;
            if r == 0 {
                return Err(CopyError::UnexpectedEof);
            }
            hasher.update(&buf[0..r]);
            output
                .write_all(&buf[0..r])
                .map_err(CopyError::WriteError)?;
            left -= r;

            #[cfg(feature = "progress_bar")]
            {
                let bytes_copied = bytes_to_copy - left;
                let progess = (bytes_copied as f32 / bytes_to_copy as f32 * 100.0) as u8;
                print!("\rCopying Block [{:3}/{:3}] {:3}% [{}/{}]", idx_range, num_range, progess, bytes_copied, bytes_to_copy);
                let _ = stdout.flush();
            }
        }
        let digest = hasher.finalize_reset();
        if range.checksum().as_slice() != digest.as_slice() {
            return Err(CopyError::ChecksumError);
        }

        position = range.offset() + range.length();

        #[cfg(feature = "progress_bar")]
        {
            println!("");
            idx_range += 1;
        }
    }
    Ok(())
}

pub async fn copy_async<I, O>(input: &mut I, output: &mut O, map: &Bmap) -> Result<(), CopyError>
where
    I: AsyncRead + AsyncSeekForward + Unpin,
    O: AsyncWrite + AsyncSeekForward + Unpin,
{
    let mut hasher = match map.checksum_type() {
        HashType::Sha256 => Sha256::new(),
    };

    // TODO benchmark a reasonable size for this
    let mut v = vec![0; 8 * 1024 * 1024];

    let buf = v.as_mut_slice();
    let mut position = 0;

    #[cfg(feature = "progress_bar")]
    let num_range = map.block_map().len();
    #[cfg(feature = "progress_bar")]
    let mut idx_range = 1;

    for range in map.block_map() {
        let forward = range.offset() - position;
        input
            .async_seek_forward(forward)
            .map_err(CopyError::ReadError)
            .await?;
        output.flush().map_err(CopyError::WriteError).await?;
        output
            .async_seek_forward(forward)
            .map_err(CopyError::WriteError)
            .await?;

        // Progress Bar so you don't have to stare at void
        let bytes_to_copy = range.length() as usize;
        let mut left = bytes_to_copy;

        #[cfg(feature = "progress_bar")]
        let mut stdout = std::io::stdout();

        while left > 0 {
            let toread = left.min(buf.len());
            let r = input
                .read(&mut buf[0..toread])
                .map_err(CopyError::ReadError)
                .await?;
            if r == 0 {
                return Err(CopyError::UnexpectedEof);
            }
            hasher.update(&buf[0..r]);
            output
                .write_all(&buf[0..r])
                .await
                .map_err(CopyError::WriteError)?;
            left -= r;

            #[cfg(feature = "progress_bar")]
            {
                let bytes_copied = bytes_to_copy - left;
                let progess = (bytes_copied as f32 / bytes_to_copy as f32 * 100.0) as u8;
                print!("\rCopying Range [{:3}/{:3}] {:3}% [{}/{}]", idx_range, num_range, progess, bytes_copied, bytes_to_copy);
                let _ = stdout.flush();
            }
        }
        let digest = hasher.finalize_reset();
        if range.checksum().as_slice() != digest.as_slice() {
            return Err(CopyError::ChecksumError);
        }

        position = range.offset() + range.length();
        
        #[cfg(feature = "progress_bar")]
        {
            println!("");
            idx_range += 1;
        }
    }
    Ok(())
}

pub fn copy_nobmap<I, O>(input: &mut I, output: &mut O) -> Result<(), CopyError>
where
    I: Read,
    O: Write,
{
    std::io::copy(input, output).map_err(CopyError::WriteError)?;
    Ok(())
}

pub async fn copy_async_nobmap<I, O>(input: &mut I, output: &mut O) -> Result<(), CopyError>
where
    I: AsyncRead + AsyncSeekForward + Unpin,
    O: AsyncWrite + AsyncSeekForward + Unpin,
{
    futures::io::copy(input, output)
        .map_err(CopyError::WriteError)
        .await?;
    Ok(())
}
