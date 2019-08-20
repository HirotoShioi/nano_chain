use sha3::{Digest, Sha3_256};
use hex;

type ByteString = String; // Might use..?
type Index = u32;
type Hash = ByteString;
type Timestamp = u32;
type BlockData = ByteString;
type BlockChain = Vec<Block>;

#[derive(Debug)]
pub struct Block {
    index : Index,
    previous_hash : Hash,
    timestamp : Timestamp,
    block_data : BlockData,
    nonce : u64,
    hash : Hash,
}

impl Block {
    pub fn new
        ( index: Index
        , previous_hash: Hash
        , timestamp: Timestamp
        , block_data: BlockData
        , nonce: u64
        , hash: Hash
        ) -> Block {
        Block {
            index,
            previous_hash,
            timestamp,
            block_data,
            nonce,
            hash
        }
    }

    pub fn calculate_hash(&self) -> Hash {
        calculate_hash(&self.index, &self.previous_hash, &self.timestamp, &self.block_data, &self.nonce)
    }

    pub fn genesis_block() -> Block {
        let index = 0;
        let previous_hash = String::from("0");
        let timestamp = 0;
        let nonce = 0;
        let block_data = String::from("<<Genesis block data>>");
        let hash = calculate_hash(&index, &previous_hash, &timestamp, &block_data, &nonce);
        Block {
            index,
            previous_hash,
            timestamp,
            block_data,
            nonce,
            hash,
        }
    }
}

fn calculate_hash
    ( index: &Index
    , previous_hash: &Hash
    , timestamp: &Timestamp
    , blockdata: &BlockData
    , nonce: &u64
    ) -> Hash {
        let concat_bytes = 
            [
            index.to_string().into_bytes(),
            previous_hash.to_string().into_bytes(),
            timestamp.to_string().into_bytes(),
            blockdata.to_string().into_bytes(),
            nonce.to_string().into_bytes(),
            ].concat();
        hash(&concat_bytes)
    }

fn hash(bytes: &Vec<u8>) -> Hash {
    let mut hasher = Sha3_256::new();
    hasher.input(bytes);
    hex::encode(hasher.result())
}