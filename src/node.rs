use crate::blockchain::BlockChain;
use crate::utxo::{Transaction, UTXO};

#[derive(Debug)]
pub struct Node {
    blockchain: BlockChain,
    mempool: Vec<Transaction>,
    utxo: UTXO,
    network_difficulty: usize,
}
