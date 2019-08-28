extern crate sha3;
extern crate hex;
extern crate base64;
extern crate serde_json;
extern crate serde;

use std::time::Duration;
use std::thread;

use nano_chain::{BlockChain, ConnectionManager, SendMessage};

fn main() {

    let pool = ConnectionManager::new(vec!["127.0.0.1:8080"], Some("127.0.0.1:8000"));

    let mut num = 0;

    loop {
        thread::sleep(Duration::from_secs(5));
        println!("Block minted: {}", num);
        pool.broadcast(SendMessage::NewBlock(num)).unwrap();
        num += 1;
    }
}