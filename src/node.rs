use crate::blockchain::BlockChain;
use crate::utxo::UTXO;
use crate::transaction::Transaction;

#[derive(Debug)]
pub struct Node {
    blockchain : BlockChain,
    mempool : Vec<Transaction>,
    utxo: UTXO,
    network_difficulty: usize,
}