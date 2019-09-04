use ctrlc;
use log::{info, warn};
use rand::Rng;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

mod configuration;
mod connection;
mod connection_pool;
mod util;

use configuration::{is_valid_config, ConfigError};
pub use configuration::{read_node_config, NodeConfig};
pub use connection::ProtocolMessage::{self, *};
use connection::{broadcast, is_connection_acceptable, Connection};
use connection::{read_message, send_message};
use connection_pool::{start_pool_manager, ConnectionPool, PoolMessage};
pub use util::PeerError;
use util::{MessageSender, PeerResult};

///Connection pool handling messages between peers

///Connection manager is responsible of managing listener.
///It also should provide interface that other modules can use
#[derive(Clone)]
pub struct ConnectionManager {
    server_address: SocketAddr,
    addrs: Vec<SocketAddr>,
    capacity: usize,
    messenger_done: Arc<AtomicBool>,
    mining: bool,
    pools: ConnectionPool,
    shared_num: Arc<AtomicU32>,
    connection_check_interval: u64,
    mining_delay_lowerbound: u64,
    mining_delay_upperbound: u64,
    broadcast_delay_lowerbound: u64,
    broadcast_delay_upperbound: u64,
    add_num_lowerbound: u32,
    add_num_upperbound: u32,
}

impl ConnectionManager {
    /// Create an instance of `ConnectionManager`
    ///
    /// You can provide vector of addresses which can be used to connect to the other
    /// nodes initially as well as launching server by providin `server_address`
    pub fn new(config: NodeConfig) -> Result<ConnectionManager, ConfigError> {
        if let Err(err) = is_valid_config(&config) {
            return Err(err);
        }
        let pools = Arc::new(Mutex::new(HashMap::with_capacity(config.capacity)));
        let messenger_done = Arc::new(AtomicBool::new(false));
        let shared_num = Arc::new(AtomicU32::new(0));

        let conn_manager = ConnectionManager {
            server_address: config.server_address,
            addrs: config.peer_addresses,
            capacity: config.capacity,
            pools,
            messenger_done,
            mining: config.mining,
            shared_num,
            connection_check_interval: config.connection_check_interval,
            mining_delay_lowerbound: config.mining_delay_lowerbound,
            mining_delay_upperbound: config.mining_delay_upperbound,
            broadcast_delay_lowerbound: config.broadcast_delay_lowerbound,
            broadcast_delay_upperbound: config.broadcast_delay_upperbound,
            add_num_lowerbound: config.add_num_lowerbound,
            add_num_upperbound: config.add_num_upperbound,
        };

        Ok(conn_manager)
    }

    pub fn start(&self) {
        info!("Starting the node");
        // If start is not being called, main thread will die
        // Move these into start()
        let (register_handle, addr_sender) = start_pool_manager(
            self.server_address,
            Arc::clone(&self.pools),
            self.shared_num.to_owned(),
        )
        .unwrap();

        for address in self.addrs.to_owned().into_iter() {
            addr_sender.send(PoolMessage::Add(address)).unwrap();
        }

        //Spawn thread for proactive messaging
        let messenger_handle = start_messenger(
            self.connection_check_interval,
            Arc::clone(&self.pools),
            Arc::clone(&self.messenger_done),
            self.capacity,
            self.server_address,
        )
        .unwrap();

        let listener_pool = Arc::clone(&self.pools);
        let server_address = self.server_address;
        let addr_sender = addr_sender.to_owned();
        let shared_num_l = Arc::clone(&self.shared_num);
        let _listener_handle = thread::spawn(move || {
            //Error handling
            start_listener(listener_pool, addr_sender, server_address, shared_num_l);
        });

        //Start mining thread here

        let mining_done = Arc::new(AtomicBool::new(!self.mining));
        let mining_handle = start_mining(self.to_owned(), mining_done.to_owned());

        let pool = Arc::clone(&self.pools);
        let message_done = self.messenger_done.clone();
        ctrlc::set_handler(move || {
            broadcast(&pool, Exiting).unwrap();
            message_done.store(true, Ordering::Relaxed);
            // Clear the connection pool
            // This will trigger `Drop`, messaging each of connection nodes
            // that we have terminated
            pool.lock().unwrap().clear();
            warn!("Shutting down the node");
            process::exit(0);
        })
        .expect("Error setting Ctrl-C handler");

        messenger_handle.join().unwrap();
        register_handle.join().unwrap();
        mining_handle.join().unwrap();
    }
}

///Start the network server by binding to given address
// If listener cannot be binded, let the program crash.
fn start_listener(
    pools: ConnectionPool,
    conn_sender: MessageSender<PoolMessage>,
    address: SocketAddr,
    shared_num: Arc<AtomicU32>,
) {
    let listener = TcpListener::bind(address).unwrap();
    // accept connections and process them
    for stream in listener.incoming() {
        let conn_pools = Arc::clone(&pools);
        let conn_sender_c = conn_sender.clone();
        let conn_pools_c = Arc::clone(&pools);
        let shared_num_c = Arc::clone(&shared_num);
        thread::spawn(move || {
            let stream = stream.unwrap();
            if let Ok(Request(socket_addr)) = read_message(&stream) {
                info!("Received connection request from: {:?}", socket_addr);
                match is_connection_acceptable(&socket_addr, &conn_pools) {
                    None => {
                        send_message(&socket_addr, &stream, ConnectionAccepted).unwrap();
                        let conn = Connection::connect_stream(
                            socket_addr.to_owned(),
                            stream,
                            conn_pools_c,
                            conn_sender_c,
                            shared_num_c,
                        )
                        .expect("Unable to send message");
                        conn_pools
                            .lock()
                            .expect("Unable to lock pool")
                            .insert(socket_addr, conn);
                    }
                    Some(err_message) => send_message(&socket_addr, &stream, err_message)
                        .expect("Unable to send message"),
                }
            } else {
                let peer_addr = stream.peer_addr().unwrap();
                send_message(&peer_addr, &stream, ConnectionDenied).expect("Unable to send message")
            }
        });
    }
}

fn start_messenger(
    interval: u64,
    conn_pool: ConnectionPool,
    messenger_done: Arc<AtomicBool>,
    capacity: usize,
    my_address: SocketAddr,
) -> PeerResult<JoinHandle<()>> {
    let messenger_done_c = Arc::clone(&messenger_done);
    let messender_handle = thread::spawn(move || {
        while !messenger_done_c.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(interval));
            let conn_pool = Arc::clone(&conn_pool);
            let conn_pool_c = Arc::clone(&conn_pool);
            // perform clean up (remove dead connection)
            let conn_pool = conn_pool.lock().unwrap();
            let conn_len = conn_pool.len();
            if conn_len < capacity {
                info!("Wants more connection, asking others");
                let conn_addr: Vec<SocketAddr> = conn_pool.keys().map(|k| k.to_owned()).collect();
                let ask_message = AskPeer(my_address, conn_addr, conn_len);
                // You have to drop here explicity or else broadcast will not be
                // able to lock the connection pool
                drop(conn_pool);
                broadcast(&conn_pool_c, ask_message).unwrap();
            }
        }
    });

    Ok(messender_handle)
}

fn start_mining(manager: ConnectionManager, mining_done: Arc<AtomicBool>) -> JoinHandle<()> {
    let handle = thread::spawn(move || {
        while !mining_done.load(Ordering::Relaxed) {
            //sleep for random time
            let mut rng = rand::thread_rng();
            let interval = rng.gen_range(
                manager.mining_delay_lowerbound,
                manager.mining_delay_upperbound,
            );
            thread::sleep(Duration::from_secs(interval));

            //generate random number
            let random_num = rng.gen_range(manager.add_num_lowerbound, manager.add_num_upperbound);

            let new_num = manager.shared_num.load(Ordering::Relaxed) + random_num;
            let delay = rng.gen_range(
                manager.broadcast_delay_lowerbound,
                manager.broadcast_delay_upperbound,
            );
            info!(
                "New number generated: {}, will broadcast in {} seconds",
                new_num, delay
            );
            //Broadcast after some delay
            thread::sleep(Duration::from_secs(delay));

            let curr_num = manager.shared_num.load(Ordering::Relaxed);

            if new_num > curr_num {
                info!("Broadcasting: {:?}", new_num);
                broadcast(&manager.pools, NewNumber(manager.server_address, new_num)).unwrap();
            } else {
                warn!("Got bigger number: {:?}, aborting broadcast", curr_num);
            }
        }
    });

    handle
}
