extern crate clap;
extern crate serde_yaml;

use clap::{App, Arg};

use nano_chain::{read_node_config, ConnectionManager};

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

    // This is required, we know this value is given
    let config_path = matches.value_of("config").unwrap();
    let node_config = read_node_config(config_path).expect("Failed to read file");

    println!("{:#?}", node_config);
    println!("Starting node");

    let connection_manager = ConnectionManager::new(node_config).unwrap();

    //You can insert something like a http server here which will enable you to
    //interact with the manager via browser/curl.
    connection_manager.start();
}
