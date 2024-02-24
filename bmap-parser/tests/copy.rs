use bmap_parser::{Bmap, Discarder, SeekForward};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::File;
use std::io::Result as IOResult;
use std::io::{Error, ErrorKind, Read, Write};
use std::path::PathBuf;

#[derive(Clone, Debug)]
struct OutputMockRange {
    offset: u64,
    data: Vec<u8>,
}

impl OutputMockRange {
    fn new(offset: u64) -> Self {
        Self {
            offset,
            data: Vec::new(),
        }
    }

    fn write(&mut self, data: &[u8]) {
        self.data.extend_from_slice(data);
    }

    fn sha256(&self) -> [u8; 32] {
        Sha256::digest(&self.data).into()
    }
}

#[derive(Clone, Debug)]
struct OutputMock {
    size: u64,
    offset: u64,
    ranges: Vec<OutputMockRange>,
}

impl OutputMock {
    fn new(size: u64) -> Self {
        Self {
            size,
            offset: 0,
            ranges: Vec::new(),
        }
    }

    fn add_range(&mut self, offset: u64) -> &mut OutputMockRange {
        self.ranges.push(OutputMockRange::new(offset));
        self.ranges.last_mut().unwrap()
    }

    fn sha256(&mut self) -> [u8; 32] {
        fn pad(hasher: &mut Sha256, mut topad: u64) {
            const ZEROES: [u8; 4096] = [0; 4096];
            while topad > 0 {
                let len = ZEROES.len() as u64;
                let len = len.min(topad);
                hasher.update(&ZEROES[0..len as usize]);
                topad -= len;
            }
        }

        let mut hasher = Sha256::new();
        let mut offset = 0;
        for range in self.ranges.iter() {
            if offset < range.offset {
                pad(&mut hasher, range.offset - offset);
                offset = range.offset;
            }

            hasher.update(&range.data);
            offset += range.data.len() as u64;
        }

        pad(&mut hasher, self.size - offset);

        hasher.finalize().into()
    }
}

impl Write for OutputMock {
    fn write(&mut self, data: &[u8]) -> IOResult<usize> {
        let maxsize = self.size as usize;
        let range = match self.ranges.last_mut() {
            Some(last) if last.offset == self.offset => last,
            _ => self.add_range(self.offset),
        };
        if range.offset as usize + range.data.len() + data.len() > maxsize {
            return Err(Error::new(ErrorKind::Other, "Writing outside of space"));
        }
        range.write(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> IOResult<()> {
        Ok(())
    }
}

impl SeekForward for OutputMock {
    fn seek_forward(&mut self, forward: u64) -> IOResult<()> {
        self.offset += if let Some(last) = self.ranges.last() {
            last.data.len() as u64 + forward
        } else {
            forward
        };
        Ok(())
    }
}

fn setup_data(basename: &str) -> (Bmap, impl Read + SeekForward) {
    let mut datadir = PathBuf::new();
    datadir.push(env::var("CARGO_MANIFEST_DIR").unwrap());
    datadir.push("tests/data");

    let mut bmapfile = datadir.clone();
    bmapfile.push(format!("{}.bmap", basename));

    let mut b =
        File::open(&bmapfile).unwrap_or_else(|_| panic!("Failed to open bmap file:{:?}", bmapfile));
    let mut xml = String::new();
    b.read_to_string(&mut xml).unwrap();
    let bmap = Bmap::from_xml(&xml).unwrap();

    let mut datafile = datadir.clone();
    datafile.push(format!("{}.gz", basename));
    let g =
        File::open(&datafile).unwrap_or_else(|_| panic!("Failed to open data file:{:?}", datafile));
    let gz = GzDecoder::new(g);
    let gz = Discarder::new(gz);

    (bmap, gz)
}

fn sha256_reader<R: Read>(mut reader: R) -> [u8; 32] {
    let mut buffer = [0; 4096];
    let mut hasher = Sha256::new();
    loop {
        let r = reader.read(&mut buffer).unwrap();
        if r == 0 {
            break;
        }
        hasher.update(&buffer[0..r]);
    }

    hasher.finalize().into()
}

#[test]
fn copy() {
    let (bmap, mut input) = setup_data("test.img");
    let mut output = OutputMock::new(bmap.image_size());

    bmap_parser::copy(&mut input, &mut output, &bmap).unwrap();
    assert_eq!(bmap_parser::HashType::Sha256, bmap.checksum_type());
    assert_eq!(bmap.block_map().len(), output.ranges.len());

    // Assert that written ranges match the ranges in the map file
    for (map, range) in bmap.block_map().zip(output.ranges.iter()) {
        assert_eq!(map.offset(), range.offset);
        assert_eq!(map.length(), range.data.len() as u64);
        assert_eq!(map.checksum().as_slice(), range.sha256());
    }

    let (_, mut input) = setup_data("test.img");
    // Assert that the full gzipped content match the written output
    assert_eq!(sha256_reader(&mut input), output.sha256())
}
