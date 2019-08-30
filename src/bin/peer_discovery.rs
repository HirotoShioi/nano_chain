extern crate clap;
extern crate serde_yaml;

use clap::{App, Arg};
use serde::Deserialize;

use std::fs;
use std::net::SocketAddr;
// use std::thread;
// use std::time::Duration;

use nano_chain::{ConnectionManager};

// ./target/release/peer_discovery -c ./config/node4.yaml
fn main() {
    let matches = App::new("Peer discovery mechanism")
        .version("1.0")
        .about("Launches peer discovery executable")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("FILE")
                .help("Sets a custom config file")
                .takes_value(true)
                .required(true),
        )
        .get_matches();

    let config_path = matches.value_of("config").unwrap();
    let node_config = read_node_config(config_path).expect("Failed to read file");

    println!("{:?}", node_config);
    println!("Starting node");
    ConnectionManager::new(
        node_config.peer_addresses,
        node_config.server_address,
        node_config.capacity,
    )
    .start();

    // let mut num = 0;
    // loop {
    //     thread::sleep(Duration::from_secs(20));
    //     println!("Block minted: {}", num);
    //     pool.broadcast(ProtocolMessage::NewBlock(num)).unwrap();
    //     num += 1;
    // }
}

#[derive(Debug, Deserialize)]
pub struct NodeConfig {
    peer_addresses: Vec<SocketAddr>,
    server_address: SocketAddr,
    capacity: usize,
}

fn read_node_config(config_path: &str) -> std::io::Result<NodeConfig> {
    let content = fs::read(config_path).expect("Unable to read file");
    let node_config: NodeConfig = serde_yaml::from_slice(&content).expect("Failed to parse file");
    Ok(node_config)
}
