use ctrlc;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

mod connection;
mod connection_pool;
mod protocol_message;
mod util;

pub use connection_pool::{start_pool_manager, ConnectionPool, PoolMessage};
pub use protocol_message::ProtocolMessage::{self, *};
use protocol_message::{read_message, send_message};

pub use connection::*;
pub use util::{ChanMessage, MessageSender, PeerError, PeerResult};

const INTERVAL: u64 = 10;

///Connection pool handling messages between peers

///Connection manager is responsible of managing listener.
///It also should provide interface that other modules can use
pub struct ConnectionManager {
    server_address: SocketAddr,
    addrs: Vec<SocketAddr>,
    capacity: usize,
    pools: ConnectionPool,
    messenger_done: Arc<AtomicBool>,
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
    ) -> ConnectionManager {
        let pools = Arc::new(Mutex::new(HashMap::with_capacity(capacity)));
        let messenger_done = Arc::new(AtomicBool::new(false));

        ConnectionManager {
            server_address,
            addrs,
            capacity,
            pools,
            messenger_done,
        }
    }

    pub fn start(&mut self) {
        // If start is not being called, main thread will die
        // Move these into start()
        let (register_handle, addr_sender) =
            start_pool_manager(self.server_address, Arc::clone(&self.pools)).unwrap();

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
        //

        // Todo: drop self afterwards
        let listener_pool = Arc::clone(&self.pools);
        let server_address = self.server_address;
        let addr_sender = addr_sender.to_owned();
        let _listener_handle = thread::spawn(move || {
            //Error handling
            start_listener(listener_pool, addr_sender, server_address);
        });

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
    }
}

///Start the network server by binding to given address
// If listener cannot be binded, let the program crash.
fn start_listener(
    pools: ConnectionPool,
    conn_sender: MessageSender<PoolMessage>,
    address: SocketAddr,
) {
    let listener = TcpListener::bind(address).unwrap();
    // accept connections and process them
    for stream in listener.incoming() {
        let conn_pools = Arc::clone(&pools);
        let conn_sender_c = conn_sender.clone();
        let conn_pools_c = Arc::clone(&pools);
        thread::spawn(move || {
            let stream = stream.unwrap();
            if let Ok(Request(socket_addr)) = read_message(&stream) {
                match is_connection_acceptable(&socket_addr, &conn_pools) {
                    None => {
                        send_message(Some(&socket_addr), &stream, Accepted).unwrap();
                        let conn = Connection::connect_stream(
                            socket_addr.to_owned(),
                            stream,
                            conn_pools_c,
                            conn_sender_c,
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
                send_message(None, &stream, Denied).expect("Unable to send message")
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
