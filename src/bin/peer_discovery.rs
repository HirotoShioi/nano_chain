extern crate clap;
extern crate serde_yaml;

use clap::{App, Arg};
use env_logger::Builder;
use log::info;

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

    // Since value `config` is required, we know this will never fail
    let config_path = matches.value_of("config").unwrap();
    let node_config = read_node_config(config_path).expect("Failed to read file");

    Builder::from_default_env()
        .default_format_module_path(false)
        .default_format_timestamp(false)
        .init();
    info!("{:#?}", node_config);

    let connection_manager = ConnectionManager::new(node_config);

    //You can insert something like a http server here which will enable you to
    //interact with the manager via browser/curl.
    // let server_handle = thread::spawn(move || {
    //    MyServer::new(connection.pool, connection.shared_num).start();
    // });

    connection_manager.start().expect("Error starting node");
}
