mod bmap;
pub use crate::bmap::*;
use sha2::{Digest, Sha256};
use thiserror::Error;

use std::io::{Read, Seek, SeekFrom, Write};

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
    I: Read + Seek,
    O: Write + Seek,
{
    let mut hasher = match map.checksum_type() {
        HashType::Sha256 => Sha256::new(),
    };

    let mut v = Vec::new();
    v.resize(8 * 1024 * 1024, 0);

    let buf = v.as_mut_slice();
    for range in map.block_map() {
        input
            .seek(SeekFrom::Start(range.offset()))
            .map_err(CopyError::ReadError)?;
        output
            .seek(SeekFrom::Start(range.offset()))
            .map_err(CopyError::WriteError)?;

        let mut left = range.length() as usize;
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
        }
        let digest = hasher.finalize_reset();
        if range.checksum().as_slice() != digest.as_slice() {
            return Err(CopyError::ChecksumError);
        }
    }

    Ok(())
}
