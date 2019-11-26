use serde::Deserialize;

use std::fs;
use std::net::SocketAddr;

/// Configuration used for initializing the `ConnectionManager`
#[derive(Debug, Deserialize)]
pub struct NodeConfig {
    pub peer_addresses: Vec<SocketAddr>,
    pub server_address: SocketAddr,
    pub capacity: usize,
    pub mining: bool,
    pub connection_check_interval: u64,
    pub minimum_mining_delay: u64,
    pub mining_delay_interval: u64,
    pub minimum_broadcast_delay: u64,
    pub broadcast_delay_interval: u64,
    pub minimum_add_num: u32,
    pub add_num_range: u32,
}

///Read given file path and returns `NodeConfig`
pub fn read_node_config(config_path: &str) -> std::io::Result<NodeConfig> {
    let content = fs::read(config_path).expect("Unable to read file");
    let node_config: NodeConfig = serde_yaml::from_slice(&content).expect("Failed to parse file");
    Ok(node_config)
}
