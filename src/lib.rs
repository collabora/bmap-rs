mod bmap;
pub use crate::bmap::*;

use std::io::{Read, Write, Seek, SeekFrom};

pub fn copy<I, O>(input: &mut I, output: &mut O, map: &Bmap) -> Result<(), std::io::Error> 
    where I: Read + Seek,
          O: Write + Seek,
{
    // TODO check if output is big enough 
    for range in map.block_map() {
        input.seek(SeekFrom::Start(range.offset()))?;
        output.seek(SeekFrom::Start(range.offset()))?;

        assert_eq!(4096, map.block_size());

        // TODO bigger buffer sizee
        let mut buf = [0; 4096];
        let mut total = 0;
        while total < range.length() {
            // TODO be more robust
            let r = input.read(&mut buf)?;
            if r == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Unexpected EOF"));
            }
            output.write_all(&buf[0..r])?;
            total += r as u64;
        }
    }

    Ok(())
}
