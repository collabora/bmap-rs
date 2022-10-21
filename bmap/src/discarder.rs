use crate::{AsyncSeekForward, SeekForward};
use async_trait::async_trait;
use futures::executor::block_on;
use std::fmt::Debug;
use std::io::Read;
use std::io::Result as IOResult;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncReadExt};

/// Adaptor that implements SeekForward on types only implementing Read by discarding data
pub struct Discarder<R: Read> {
    reader: R,
}

impl<R: Read> Discarder<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R: Read> Read for Discarder<R> {
    fn read(&mut self, buf: &mut [u8]) -> IOResult<usize> {
        self.reader.read(buf)
    }
}

impl<R: Read> SeekForward for Discarder<R> {
    fn seek_forward(&mut self, forward: u64) -> IOResult<()> {
        let mut buf = [0; 4096];
        let mut left = forward as usize;
        while left > 0 {
            let toread = left.min(buf.len());
            let r = self.reader.read(&mut buf[0..toread])?;
            left -= r;
        }
        Ok(())
    }
}
//Async implementation

pub struct AsyncDiscarder<R: AsyncRead> {
    reader: R,
}

impl<R: AsyncRead + Debug> AsyncDiscarder<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for AsyncDiscarder<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}
#[async_trait(?Send)]
impl<R: AsyncRead + Unpin> AsyncSeekForward for AsyncDiscarder<R> {
    async fn async_seek_forward(&mut self, forward: u64) -> IOResult<()> {
        let mut buf = [0; 4096];
        let mut left = forward as usize;
        while left > 0 {
            let toread = left.min(buf.len());
            let r = block_on(self.read(&mut buf[0..toread]))?;
            left -= r;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::slice;

    #[test]
    fn discard() {
        let mut data = Vec::with_capacity(256);
        for byte in 0u8..=255 {
            data.push(byte);
        }

        let mut discarder = Discarder::new(data.as_slice());
        let _ = &[0u64, 5, 16, 31, 63, 200, 255]
            .iter()
            .fold(0, |pos, offset| {
                let mut byte: u8 = 1;
                discarder.seek_forward((offset - pos) as u64).unwrap();
                assert_eq!(1, discarder.read(slice::from_mut(&mut byte)).unwrap());
                assert_eq!(*offset, byte as u64);
                *offset + 1
            });
    }
}
