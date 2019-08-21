use sha3::{Digest, Sha3_256};
use std::time::{SystemTime, UNIX_EPOCH};
use std::iter;
use base64;

type ByteString = String; // Might use..?
type Index = u32;
type Hash = ByteString;
type Timestamp = u64;
type BlockData = ByteString;

#[derive(Debug)]
pub struct BlockChain {
    blocks: Vec<Block>,
}

#[derive(Debug)]
pub enum BlockChainError {
    EmptyChain,
    InvalidIndex,
    InvalidPreviousHash(Hash),
    InvalidHash(Hash),
    WorkNotProven,
}

use crate::blockchain::BlockChainError::*;

impl BlockChain {
    pub fn new() -> BlockChain {
        let blocks = vec![genesis_block()];

        BlockChain {
            blocks
        }
    }

    pub fn mint_block(&mut self, block_data: &str) {
        let previous_block = self.blocks.last().unwrap();
        let difficulty = calculate_difficulty(&previous_block);
        let new_block = generate_next_block(previous_block, &difficulty, &now(), &block_data.to_string());

        self.blocks.push(new_block);
    }

    pub fn add_block(&mut self, new_block: Block) -> Result<(), BlockChainError> {
        let previous_block = self.blocks.last().unwrap();

        match is_valid_block(previous_block , &new_block) {
            Err(err) => Err(err),
            Ok(_) => {
                self.blocks.push(new_block);
                Ok(())
            }
        }
    }

    pub fn is_valid_chain(&self) -> Result<(), BlockChainError> {
        is_valid_chain(self)
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    index : Index,
    previous_hash : Hash,
    timestamp : Timestamp,
    block_data : BlockData,
    nonce : u64,
    difficulty : usize,
    hash : Hash,
}

impl Block {
    pub fn new
        ( index: Index
        , previous_hash: Hash
        , timestamp: Timestamp
        , block_data: BlockData
        , difficulty: usize
        , nonce: u64
        ) -> Block {
        let hash = calculate_hash_base64(&index, &previous_hash, &timestamp, &block_data, &nonce);
        Block {
            index,
            previous_hash,
            timestamp,
            block_data,
            nonce,
            difficulty,
            hash
        }
    }

    pub fn get_hash(&self) -> Hash {
        calculate_hash_base64(&self.index
                            , &self.previous_hash
                            , &self.timestamp
                            , &self.block_data
                            , &self.nonce)
    }
}

pub fn genesis_block() -> Block {
    let index = 0;
    let previous_hash = String::from("0");
    let timestamp = 0;
    let nonce = 0;
    let difficulty = 0;
    let block_data = String::from("<<Genesis block data>>");
    let hash = calculate_hash_base64(&index, &previous_hash, &timestamp, &block_data, &nonce);
    Block {
        index,
        previous_hash,
        timestamp,
        block_data,
        nonce,
        difficulty,
        hash,
    }
}

fn calculate_hash_base64
    ( index: &Index
    , previous_hash: &Hash
    , timestamp: &Timestamp
    , blockdata: &BlockData
    , nonce: &u64
    ) -> Hash
{
    let concat_bytes = 
        [ index.to_string().into_bytes(),
          previous_hash.to_string().into_bytes(),
          timestamp.to_string().into_bytes(),
          blockdata.to_string().into_bytes(),
          nonce.to_string().into_bytes(),
        ].concat();

    hash_base64(&concat_bytes)
}

fn hash_base64(bytes: &Vec<u8>) -> Hash {
    let mut hasher = Sha3_256::new();
    hasher.input(bytes);
    base64::encode(&hasher.result())
}

fn now() -> u64 {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    since_the_epoch.as_secs()
}

fn generate_next_block
    ( previous_block : &Block
    , difficulty: &usize
    , timestamp: &Timestamp
    , block_data: &BlockData) -> Block
{
    let index = previous_block.index + 1;
    let previous_hash = &previous_block.hash;
    let nonce = proof_of_work(&index, &difficulty, &previous_hash, timestamp, block_data);

    Block::new
        (index.clone(),
        previous_hash.clone(),
        timestamp.clone(),
        block_data.clone(),
        difficulty.clone(),
        nonce,
        )
}

fn proof_of_work
    ( index: &Index
    , difficulty: &usize
    , previous_hash: &Hash
    , timestamp: &Timestamp
    , blockdata: &BlockData) -> u64
{   let mut nonce = 0;

    while !is_work_proven(difficulty, index, previous_hash, timestamp, blockdata, &nonce) {
        nonce = nonce + 1;
    }

    nonce
}

fn is_work_proven
    ( difficulty: &usize,
      index: &Index,
      previous_hash: &Hash,
      timestamp: &Timestamp,
      blockdata: &BlockData,
      nonce: &u64
    ) -> bool 
{   
    let prefix: String = iter::repeat('0').take(*difficulty).collect();
    let hash = calculate_hash_base64(&index, previous_hash, timestamp, blockdata, nonce);

    let hash_prefix = hash.get(0..*difficulty)
        .expect("Error when encoding hash");

    prefix == hash_prefix
}

fn is_valid_block(previous_block: &Block, new_block: &Block) -> Result<(), BlockChainError> {
    
    let work_invalid = !is_work_proven
        (&new_block.difficulty,
        &new_block.index,
        &new_block.previous_hash,
        &new_block.timestamp,
        &new_block.block_data,
        &new_block.nonce);

    if previous_block.index + 1 != new_block.index {
        return Err(InvalidIndex)
    } else if previous_block.hash != new_block.previous_hash {
        return Err(InvalidPreviousHash(new_block.previous_hash.clone()))
    } else if new_block.get_hash() != new_block.hash {
        return Err(InvalidHash(new_block.hash.clone()))
    } else if work_invalid{
        return Err(WorkNotProven)
    } else {
        return Ok(())
    }
}

fn is_valid_chain(blockchain: &BlockChain) -> Result<(), BlockChainError> {
    let chain_len = blockchain.blocks.len();

    if chain_len == 0 {
        return Err(EmptyChain)
    } else if chain_len == 1 {
        return Ok(())
    } else {
        let b0 = &blockchain.blocks[0];
        let b1 = &blockchain.blocks[1];
        let b0_b1_valid = is_valid_block(&b0, &b1);
        let rest_block = BlockChain { blocks : blockchain.blocks.split_at(1).1.to_vec()};
        let rest_valid = is_valid_chain(&rest_block);
        
        match b0_b1_valid {
            Err(e) => Err(e),
            Ok(_) => rest_valid,
        }

    }
}

fn calculate_difficulty(previous_block: &Block) -> usize {
    let previous_difficulty = if previous_block.index < 2 {
        2
    } else {
        previous_block.index
    };

    let dbits = (previous_difficulty as f32).log2().round() as usize;
    dbits
}