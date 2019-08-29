use std::time::Duration;
use std::thread;
use std::net::SocketAddr;

use nano_chain::{ConnectionManager, SendMessage};

fn main() {
    //Need CLI
    let addr1 = SocketAddr::from(([127, 0, 0, 1], 8080));
    // let addr2 = SocketAddr::from(([127, 0, 0, 1], 7878));
    let addrs = vec![addr1];
    let pool = ConnectionManager::new(&addrs[..], Some("127.0.0.1:8000"));

    let mut num = 0;

    loop {
        thread::sleep(Duration::from_secs(5));
        println!("Block minted: {}", num);
        pool.broadcast(SendMessage::NewBlock(num)).unwrap();
        num += 1;
    }
}