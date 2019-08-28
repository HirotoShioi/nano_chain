use std::time::Duration;
use std::thread;

use nano_chain::{ConnectionManager, SendMessage};

fn main() {
    //Need CLI
    let pool = ConnectionManager::new(vec!["127.0.0.1:8080"], Some("127.0.0.1:8000"));

    let mut num = 0;

    loop {
        thread::sleep(Duration::from_secs(5));
        println!("Block minted: {}", num);
        pool.broadcast(SendMessage::NewBlock(num)).unwrap();
        num += 1;
    }
}