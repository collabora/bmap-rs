use strum::{Display, EnumDiscriminants, EnumString};
use thiserror::Error;
mod xml;

/// Hash type to be used to build the checksum at blocks.
#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumString, Display)]
#[strum(serialize_all = "lowercase")]
#[non_exhaustive]
pub enum HashType {
    Sha256,
}

/// Value holder for the block's checksum.
#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumDiscriminants)]
#[non_exhaustive]
pub enum HashValue {
    Sha256([u8; 32]),
}

impl HashValue {
    /// Returns the hash type used  in the HashValue.
    pub fn to_type(&self) -> HashType {
        match self {
            HashValue::Sha256(_) => HashType::Sha256,
        }
    }

    /// Returns the value of the checksum in the HashValue.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            HashValue::Sha256(v) => v,
        }
    }
}

/// Reference to a mapped block of data that contain its checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRange {
    offset: u64,
    length: u64,
    checksum: HashValue,
}

impl BlockRange {
    /// Returns the checksum of the data mapped in the range of the block.
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

/// Contains the bmap file information, including a vector of blocks.
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
    /// Returns a BmapBuilder.
    pub fn builder() -> BmapBuilder {
        BmapBuilder::default()
    }

    pub fn from_xml(xml: &str) -> Result<Self, xml::XmlError> {
        xml::from_xml(xml)
    }

    pub fn image_size(&self) -> u64 {
        self.image_size
    }

    pub const fn block_size(&self) -> u64 {
        self.block_size
    }

    /// Returns the number of blocks.
    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    pub fn mapped_blocks(&self) -> u64 {
        self.mapped_blocks
    }

    /// Returns the type of Hash used for the checksum.
    pub fn checksum_type(&self) -> HashType {
        self.checksum_type
    }

    /// Returns an iterator of BlockRange.
    pub fn block_map(&self) -> impl ExactSizeIterator + Iterator<Item = &BlockRange> {
        self.blockmap.iter()
    }

    /// Returns the total size of mapped memory, it can be bigger than the image size.
    pub fn total_mapped_size(&self) -> u64 {
        self.block_size * self.mapped_blocks
    }
}

/// The error type returned by BmapBuilder.
///
/// This error indicates that the Bmap could not be built correctly. It also points at the field that originated the error.
#[derive(Clone, Debug, Error)]
pub enum BmapBuilderError {
    #[error("Image size missing")]
    MissingImageSize,
    #[error("Block size missing")]
    MissingBlockSize,
    /// Error that indicates the number of blocks is missing.
    #[error("Blocks missing")]
    MissingBlocks,
    /// Error that indicates the number of mapped blocks is missing.
    #[error("Mapped blocks missing")]
    MissingMappedBlocks,
    #[error("Checksum type missing")]
    MissingChecksumType,
    /// Error that indicates there are not BlockRanges in the vector.
    #[error("No block ranges")]
    NoBlockRanges,
}

/// Intermediary tool to generate Bmap.
///
/// Contains the same data fields as a Bmap, but most of them as Option. Allowing a progressive parsing and then a casting into Bmap.
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

    /// Add a BlockRange to the vector indicating start, end and checksum. Needs Blocksize and Image to be set first. Returns the BmapBuilder.
    pub fn add_block_range(&mut self, start: u64, end: u64, checksum: HashValue) -> &mut Self {
        let bs = self.block_size.expect("Blocksize needs to be set first");
        let total = self.image_size.expect("Image size needs to be set first");
        let offset = start * bs;
        let length = (total - offset).min((end - start + 1) * bs);
        self.add_byte_range(offset, length, checksum)
    }

    /// Add a BlockRange to the vector indicating offset, length and checksum.  Returns the BmapBuilder.
    pub fn add_byte_range(&mut self, offset: u64, length: u64, checksum: HashValue) -> &mut Self {
        let range = BlockRange {
            offset,
            length,
            checksum,
        };
        self.blockmap.push(range);
        self
    }

    /// Returns a Bmap or an Error as a Result.
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
