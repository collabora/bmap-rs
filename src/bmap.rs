use thiserror::Error;
mod xml;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRange {
    offset: u64,
    length: u64,
    // TODO checksum
}

impl BlockRange {
    pub fn checksum(&self) -> &str {
        todo!()
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
    blockmap: Vec<BlockRange>,
}

impl Bmap {
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

    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    pub fn mapped_blocks(&self) -> u64 {
        self.mapped_blocks
    }

    pub fn block_map(&self) -> impl Iterator<Item = &BlockRange> {
        self.blockmap.iter()
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
    #[error("Mappd blocks missing")]
    MissingMappedBlocks,
    #[error("No block ranges")]
    NoBlockRanges,
}

#[derive(Clone, Debug, Default)]
pub struct BmapBuilder {
    image_size: Option<u64>,
    block_size: Option<u64>,
    blocks: Option<u64>,
    mapped_blocks: Option<u64>,
    blockmap: Vec<BlockRange>,
}

impl BmapBuilder {
    pub fn image_size(&mut self, size: u64)  -> &mut Self {
        self.image_size = Some(size);
        self
    }

    pub fn block_size(&mut self, block_size: u64)  -> &mut Self {
        self.block_size = Some(block_size);
        self
    }

    pub fn blocks(&mut self, blocks: u64)  -> &mut Self {
        self.blocks = Some(blocks);
        self
    }

    pub fn mapped_blocks(&mut self, blocks: u64)  -> &mut Self {
        self.mapped_blocks = Some(blocks);
        self
    }

    pub fn add_block_range(&mut self, start: u64, end: u64) -> &mut Self {
        let bs = self.block_size.expect("Blocksize needs to be set first");
        let total = self.image_size.expect("Image size needs to be set first");
        let offset = start * bs;
        let length = (total - offset).min((end - start + 1) * bs);
        self.add_byte_range(offset, length)
    }

    pub fn add_byte_range(&mut self, offset: u64, length: u64) -> &mut Self {
        let range = BlockRange { offset , length };
        self.blockmap.push(range);
        self
    }

    pub fn build(self) -> Result<Bmap, BmapBuilderError> {
        let image_size = self.image_size.ok_or(BmapBuilderError::MissingImageSize)?;
        let block_size = self.block_size.ok_or(BmapBuilderError::MissingBlockSize)?;
        let blocks = self.blocks.ok_or(BmapBuilderError::MissingBlocks)?;
        let mapped_blocks = self.mapped_blocks.ok_or(BmapBuilderError::MissingMappedBlocks)?;
        let blockmap = self.blockmap;
        Ok(Bmap {
            image_size,
            block_size,
            blocks,
            mapped_blocks,
            blockmap
        })
    }
}
