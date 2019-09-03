use serde::Deserialize;

use std::fs;
use std::net::SocketAddr;

#[derive(Debug, Deserialize)]
pub struct NodeConfig {
    pub peer_addresses: Vec<SocketAddr>,
    pub server_address: SocketAddr,
    pub capacity: usize,
    pub mining: bool,
    pub connection_check_interval: u64,
    pub mining_delay_lowerbound: u64,
    pub mining_delay_upperbound: u64,
    pub broadcast_delay_lowerbound: u64,
    pub broadcast_delay_upperbound: u64,
    pub add_num_lowerbound: u32,
    pub add_num_upperbound: u32,
}

#[derive(Debug)]
pub enum ConfigError {
    MiningDelayInvalid,
    BroadcastDelayInvalid,
    AddNumInvalid,
}

use super::configuration::ConfigError::*;

pub fn read_node_config(config_path: &str) -> std::io::Result<NodeConfig> {
    let content = fs::read(config_path).expect("Unable to read file");
    let node_config: NodeConfig = serde_yaml::from_slice(&content).expect("Failed to parse file");
    Ok(node_config)
}

pub fn is_valid_config(config: &NodeConfig) -> Result<(), ConfigError> {
    if config.mining_delay_lowerbound > config.mining_delay_upperbound {
        Err(MiningDelayInvalid)
    } else if config.broadcast_delay_lowerbound > config.broadcast_delay_upperbound {
        Err(BroadcastDelayInvalid)
    } else if config.add_num_lowerbound > config.add_num_upperbound {
        Err(AddNumInvalid)
    } else {
        Ok(())
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let error_message = match self {
            ConfigError::MiningDelayInvalid => {
                "Mining delay invalid: Upperbound must be bigger than lowerbound"
            }
            ConfigError::BroadcastDelayInvalid => {
                "Broadcast delay invalid: Upperbound must be bigger than lowerbound"
            }
            ConfigError::AddNumInvalid => {
                "Add num delay invalid: Upperbound must be bigger than lowerbound"
            }
        };
        write!(f, "{}", error_message)
    }
}

impl std::error::Error for ConfigError {}
