use strum::{Display, EnumDiscriminants, EnumString};
use thiserror::Error;
mod xml;

#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumString, Display)]
#[strum(serialize_all = "lowercase")]
#[non_exhaustive]
pub enum HashType {
    Sha256,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumDiscriminants)]
#[non_exhaustive]
pub enum HashValue {
    Sha256([u8; 32]),
}

impl HashValue {
    pub fn to_type(&self) -> HashType {
        match self {
            HashValue::Sha256(_) => HashType::Sha256,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            HashValue::Sha256(v) => v,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRange {
    offset: u64,
    length: u64,
    checksum: HashValue,
}

impl BlockRange {
    pub fn checksum(&self) -> HashValue {
        self.checksum
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn length(&self) -> u64 {
        self.length
    }
}

#[derive(Clone, Debug)]
pub struct Bmap {
    image_size: u64,
    block_size: u64,
    blocks: u64,
    mapped_blocks: u64,
    checksum_type: HashType,
    blockmap: Vec<BlockRange>,
}

impl Bmap {
    pub fn builder() -> BmapBuilder {
        BmapBuilder::default()
    }

    /// Build from a .bmap xml file
    pub fn from_xml(xml: &str) -> Result<Self, xml::XmlError> {
        xml::from_xml(xml)
    }

    /// Image size in bytes
    pub fn image_size(&self) -> u64 {
        self.image_size
    }

    /// block size in bytes
    pub const fn block_size(&self) -> u64 {
        self.block_size
    }

    /// number of blocks in the image
    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    /// number of mapped blocks in the image
    pub fn mapped_blocks(&self) -> u64 {
        self.mapped_blocks
    }

    /// checksum type used
    pub fn checksum_type(&self) -> HashType {
        self.checksum_type
    }

    /// Iterator over the block map
    pub fn block_map(&self) -> impl ExactSizeIterator + Iterator<Item = &BlockRange> {
        self.blockmap.iter()
    }

    /// Total mapped size in bytes
    pub fn total_mapped_size(&self) -> u64 {
        self.block_size * self.mapped_blocks
    }
}

#[derive(Clone, Debug, Error)]
pub enum BmapBuilderError {
    #[error("Image size missing")]
    MissingImageSize,
    #[error("Block size missing")]
    MissingBlockSize,
    #[error("Blocks missing")]
    MissingBlocks,
    #[error("Mapped blocks missing")]
    MissingMappedBlocks,
    #[error("Checksum type missing")]
    MissingChecksumType,
    #[error("No block ranges")]
    NoBlockRanges,
}

#[derive(Clone, Debug, Default)]
pub struct BmapBuilder {
    image_size: Option<u64>,
    block_size: Option<u64>,
    blocks: Option<u64>,
    checksum_type: Option<HashType>,
    mapped_blocks: Option<u64>,
    blockmap: Vec<BlockRange>,
}

impl BmapBuilder {
    pub fn image_size(&mut self, size: u64) -> &mut Self {
        self.image_size = Some(size);
        self
    }

    pub fn block_size(&mut self, block_size: u64) -> &mut Self {
        self.block_size = Some(block_size);
        self
    }

    pub fn blocks(&mut self, blocks: u64) -> &mut Self {
        self.blocks = Some(blocks);
        self
    }

    pub fn mapped_blocks(&mut self, blocks: u64) -> &mut Self {
        self.mapped_blocks = Some(blocks);
        self
    }

    pub fn checksum_type(&mut self, checksum_type: HashType) -> &mut Self {
        self.checksum_type = Some(checksum_type);
        self
    }

    pub fn add_block_range(&mut self, start: u64, end: u64, checksum: HashValue) -> &mut Self {
        let bs = self.block_size.expect("Blocksize needs to be set first");
        let total = self.image_size.expect("Image size needs to be set first");
        let offset = start * bs;
        let length = (total - offset).min((end - start + 1) * bs);
        self.add_byte_range(offset, length, checksum)
    }

    pub fn add_byte_range(&mut self, offset: u64, length: u64, checksum: HashValue) -> &mut Self {
        let range = BlockRange {
            offset,
            length,
            checksum,
        };
        self.blockmap.push(range);
        self
    }

    pub fn build(self) -> Result<Bmap, BmapBuilderError> {
        let image_size = self.image_size.ok_or(BmapBuilderError::MissingImageSize)?;
        let block_size = self.block_size.ok_or(BmapBuilderError::MissingBlockSize)?;
        let blocks = self.blocks.ok_or(BmapBuilderError::MissingBlocks)?;
        let mapped_blocks = self
            .mapped_blocks
            .ok_or(BmapBuilderError::MissingMappedBlocks)?;
        let checksum_type = self
            .checksum_type
            .ok_or(BmapBuilderError::MissingChecksumType)?;
        let blockmap = self.blockmap;

        Ok(Bmap {
            image_size,
            block_size,
            blocks,
            mapped_blocks,
            checksum_type,
            blockmap,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn hashes() {
        assert_eq!("sha256", &HashType::Sha256.to_string());
        assert_eq!(HashType::Sha256, HashType::from_str("sha256").unwrap());
        let h = HashValue::Sha256([0; 32]);
        assert_eq!(HashType::Sha256, h.to_type());
    }
}
