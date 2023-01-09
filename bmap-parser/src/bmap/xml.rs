use crate::bmap::{BmapBuilder, BmapBuilderError, HashType, HashValue};
use quick_xml::de::{from_str, DeError};
use serde::Deserialize;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Deserialize)]
struct Range {
    #[serde(rename = "@chksum")]
    chksum: String,
    #[serde(rename = "$value")]
    range: String,
}

#[derive(Debug, Deserialize)]
struct BlockMap {
    #[serde(rename = "Range")]
    ranges: Vec<Range>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Bmap {
    #[serde(rename = "@version")]
    version: String,
    #[serde(rename = "ImageSize")]
    image_size: u64,
    #[serde(rename = "BlockSize")]
    block_size: u64,
    #[serde(rename = "BlocksCount")]
    blocks_count: u64,
    #[serde(rename = "MappedBlocksCount")]
    mapped_blocks_count: u64,
    #[serde(rename = "ChecksumType")]
    checksum_type: String,
    #[serde(rename = "BmapFileChecksum")]
    bmap_file_checksum: String,
    #[serde(rename = "BlockMap")]
    block_map: BlockMap,
}

#[derive(Debug, Error)]
pub enum XmlError {
    #[error("Failed to parse bmap XML: {0}")]
    XmlParsError(#[from] DeError),
    #[error("Invalid bmap file: {0}")]
    InvalidFIleError(#[from] BmapBuilderError),
    #[error("Unknown checksum type: {0}")]
    UnknownChecksumType(String),
    #[error("Invalid checksum: {0}")]
    InvalidChecksum(String),
}

const fn hexdigit_to_u8(c: u8) -> Option<u8> {
    match c {
        b'a'..=b'f' => Some(c - b'a' + 0xa),
        b'A'..=b'F' => Some(c - b'A' + 0xa),
        b'0'..=b'9' => Some(c - b'0'),
        _ => None,
    }
}

fn str_to_digest(s: String, digest: &mut [u8]) -> Result<(), XmlError> {
    let l = digest.len();
    if s.len() != l * 2 {
        return Err(XmlError::InvalidChecksum(format!(
            "No enough chars: {} {}",
            s,
            s.len()
        )));
    }

    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = match hexdigit_to_u8(chunk[0]) {
            Some(v) => v,
            None => return Err(XmlError::InvalidChecksum(s)),
        };
        let lo = match hexdigit_to_u8(chunk[1]) {
            Some(v) => v,
            None => return Err(XmlError::InvalidChecksum(s)),
        };
        digest[i] = hi << 4 | lo;
    }

    Ok(())
}

pub(crate) fn from_xml(xml: &str) -> Result<crate::bmap::Bmap, XmlError> {
    let b: Bmap = from_str(xml)?;
    let mut builder = BmapBuilder::default();
    let hash_type = b.checksum_type;
    let hash_type =
        HashType::from_str(&hash_type).map_err(|_| XmlError::UnknownChecksumType(hash_type))?;
    builder
        .image_size(b.image_size)
        .block_size(b.block_size)
        .blocks(b.blocks_count)
        .checksum_type(hash_type)
        .mapped_blocks(b.mapped_blocks_count);

    for range in b.block_map.ranges {
        let mut split = range.range.trim().splitn(2, '-');
        let start = match split.next() {
            Some(s) => s.parse().unwrap(),
            None => unimplemented!("woops"),
        };
        let end = match split.next() {
            Some(s) => s.parse().unwrap(),
            None => start,
        };

        let checksum = match hash_type {
            HashType::Sha256 => {
                let mut v = [0; 32];
                str_to_digest(range.chksum, &mut v)?;
                HashValue::Sha256(v)
            }
        };
        builder.add_block_range(start, end, checksum);
    }

    builder.build().map_err(std::convert::Into::into)
}
