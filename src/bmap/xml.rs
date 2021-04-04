use crate::bmap::{BmapBuilder, BmapBuilderError};
use serde::Deserialize;
use quick_xml::de::{from_str, DeError};
use thiserror::Error;

#[derive(Debug, Deserialize)]
struct Range {
    chksum: String,
    #[serde(rename = "$value")]
    range: String,
}

#[derive(Debug, Deserialize)]
struct BlockMap {
    #[serde(rename = "Range")]
    ranges: Vec<Range>,
}

#[derive(Debug, Deserialize)]
struct Bmap {
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
}

pub(crate) fn  from_xml(xml: &str) -> Result<crate::bmap::Bmap, XmlError> {
    let b: Bmap = from_str(xml)?;
    let mut builder = BmapBuilder::default();
    builder
        .image_size(b.image_size)
        .block_size(b.block_size)
        .blocks(b.blocks_count)
        .mapped_blocks(b.mapped_blocks_count);
    
    for range in b.block_map.ranges {
        let mut split = range.range.trim().splitn(2, '-');
        let start = match split.next() {
            Some(s) => s.parse().unwrap(),
            None => unimplemented!("woops"),
        };
        let end = match split.next() {
            Some(s) => s.parse().unwrap(),
            None => start
        };
        builder.add_block_range(start, end);
    }

    builder.build().map_err(std::convert::Into::into)
}
