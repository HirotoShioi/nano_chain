extern crate sha3;
extern crate hex;
extern crate base64;
extern crate serde_json;
extern crate serde;

use std::time::Duration;
use std::thread;

use nano_chain::{BlockChain, ConnectionManager, SendMessage};

fn main() {
    // let mut block = BlockChain::new();
    // block.mint_block(b"hello_world");
    // block.mint_block(b"I'm from canada");
    // block.mint_block(b"New block chain");
    // block.mint_block(b"Mexico");
    // block.mint_block(b"generator");

    // assert!(block.is_valid_chain().is_ok());
    // println!("{:#?}", block);

    let pool = ConnectionManager::new(vec![
        // "192.168.0.1:7888",
        "127.0.0.1:7878",
        "127.0.0.1:8080"
    ], Some("127.0.0.1:8000"));

    let mut num = 0;

    loop {
        thread::sleep(Duration::from_secs(5));
        println!("Block minted: {}", num);
        pool.broadcast(SendMessage::NewBlock(num)).unwrap();
        num += 1;
    }
}