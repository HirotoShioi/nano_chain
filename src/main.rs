
extern crate sha3;
extern crate hex;
extern crate base64;
extern crate serde_json;
extern crate serde;

use nano_chain::BlockChain;

fn main() {
    let mut block = BlockChain::new();
    block.mint_block("hello_world");
    block.mint_block("I'm from canada");
    block.mint_block("New block chain");
    block.mint_block("Mexico");
    block.mint_block("generator");

    assert!(block.is_valid_chain().is_ok());
    println!("{:#?}", block);
}