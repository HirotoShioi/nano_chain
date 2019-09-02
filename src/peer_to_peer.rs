use ctrlc;
use rand::Rng;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

mod connection;
mod connection_pool;
mod util;

pub use connection::ProtocolMessage::{self, *};
use connection::{read_message, send_message};
pub use connection_pool::{start_pool_manager, ConnectionPool, PoolMessage};

pub use connection::*;
pub use util::{ChanMessage, MessageSender, PeerError, PeerResult};

const INTERVAL: u64 = 20;
const MINIMUM_MINING_INTERVAL: u64 = 10;
const RANDOM_DELAY_UPPERBOUND: u64 = 10;
const RANDOM_DELAY_LOWERBOUND: u64 = 1;

const ADD_NUM_LOWERBOUND: u32 = 1;
const ADD_NUM_UPPERBOUD: u32 = 10;

///Connection pool handling messages between peers

///Connection manager is responsible of managing listener.
///It also should provide interface that other modules can use
pub struct ConnectionManager {
    server_address: SocketAddr,
    addrs: Vec<SocketAddr>,
    capacity: usize,
    messenger_done: Arc<AtomicBool>,
    mining: bool,
    pools: ConnectionPool,
    shared_num: Arc<AtomicU32>,
    // We need another AtomicU32 to store the temorary candidate number
    // candidate_num: Arc<AtomicU32>
}

impl ConnectionManager {
    /// Create an instance of `ConnectionManager`
    ///
    /// You can provide vector of addresses which can be used to connect to the other
    /// nodes initially as well as launching server by providin `server_address`
    pub fn new(
        addrs: Vec<SocketAddr>,
        server_address: SocketAddr,
        capacity: usize,
        mining: bool,
    ) -> ConnectionManager {
        let pools = Arc::new(Mutex::new(HashMap::with_capacity(capacity)));
        let messenger_done = Arc::new(AtomicBool::new(false));
        let shared_num = Arc::new(AtomicU32::new(0));

        ConnectionManager {
            server_address,
            addrs,
            capacity,
            pools,
            messenger_done,
            mining,
            shared_num,
        }
    }

    pub fn start(&self) {
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
        let mining_handle = start_mining(
            self.server_address,
            self.pools.to_owned(),
            mining_done.to_owned(),
            self.shared_num.to_owned(),
        );

        let pool = Arc::clone(&self.pools);
        let server_address = self.server_address;
        let message_done = self.messenger_done.clone();
        ctrlc::set_handler(move || {
            broadcast(&pool, Exiting(server_address)).unwrap();
            message_done.store(true, Ordering::Relaxed);
            // Clear the connection pool
            // This will signal all the connected nodes that they should remove
            // it from pool
            pool.lock().unwrap().clear();
            println!("Exiting");
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
                match is_connection_acceptable(&socket_addr, &conn_pools) {
                    None => {
                        send_message(Some(&socket_addr), &stream, ConnectionAccepted).unwrap();
                        let conn = Connection::connect_stream(
                            address,
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
                    Some(err_message) => send_message(Some(&socket_addr), &stream, err_message)
                        .expect("Unable to send message"),
                }
            } else {
                send_message(None, &stream, ConnectionDenied).expect("Unable to send message")
            }
        });
    }
}

fn start_messenger(
    conn_pool: ConnectionPool,
    messenger_done: Arc<AtomicBool>,
    capacity: usize,
    my_address: SocketAddr,
) -> PeerResult<JoinHandle<()>> {
    let messenger_done_c = Arc::clone(&messenger_done);
    let messender_handle = thread::spawn(move || {
        while !messenger_done_c.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(INTERVAL));
            let conn_pool = Arc::clone(&conn_pool);
            let conn_pool_c = Arc::clone(&conn_pool);
            // perform clean up (remove dead connection)
            let conn_pool = conn_pool.lock().unwrap();
            let conn_len = conn_pool.len();
            if conn_len < capacity {
                let conn_addr: Vec<SocketAddr> = conn_pool.keys().map(|k| k.to_owned()).collect();
                let ask_message = AskPeer(my_address, conn_addr, conn_len);
                drop(conn_pool);
                broadcast(&conn_pool_c, ask_message).unwrap();
            }
        }
    });

    Ok(messender_handle)
}

fn start_mining(
    server_address: SocketAddr,
    pool: ConnectionPool,
    mining_done: Arc<AtomicBool>,
    shared_num: Arc<AtomicU32>,
) -> JoinHandle<()> {
    let handle = thread::spawn(move || {
        while !mining_done.load(Ordering::Relaxed) {
            //sleep for random time
            let mut rng = rand::thread_rng();
            let interval = rng.gen_range(RANDOM_DELAY_LOWERBOUND, RANDOM_DELAY_UPPERBOUND)
                + MINIMUM_MINING_INTERVAL;
            thread::sleep(Duration::from_secs(interval));

            //generate random number
            let random_num = rng.gen_range(ADD_NUM_LOWERBOUND, ADD_NUM_UPPERBOUD);

            //store num
            let curr_num = shared_num.load(Ordering::Relaxed) + random_num;
            let delay = rng.gen_range(RANDOM_DELAY_LOWERBOUND, RANDOM_DELAY_UPPERBOUND);
            println!(
                "New value generated: {}, will broadcast in {} seconds",
                curr_num, delay
            );
            //Broadcast after some delay
            thread::sleep(Duration::from_secs(delay));
            println!("Broadcasting new value");
            broadcast(&pool, NewNumber(server_address, curr_num)).unwrap();
        }
    });

    handle
}
