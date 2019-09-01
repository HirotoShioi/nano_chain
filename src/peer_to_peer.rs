use ctrlc;
use std::collections::HashMap;
use std::error;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

mod connection;
mod connection_pool;
mod protocol_message;
mod util;

pub use connection_pool::{start_pool_manager, ConnectionPool, PoolError, PoolMessage};
pub use protocol_message::ProtocolMessage::{self, *};
use protocol_message::{read_message, send_message};

pub use connection::*;
use util::PeerResult;

///Connection pool handling messages between peers

///Connection manager is responsible of managing listener.
///It also should provide interface that other modules can use
pub struct ConnectionManager {
    server_address: SocketAddr,
    pools: ConnectionPool,
    register_done: Arc<AtomicBool>,
    register_handle: Option<JoinHandle<()>>,
    messenger_done: Arc<AtomicBool>,
    messenger_handle: Option<JoinHandle<()>>,
    addr_sender: mpsc::Sender<PoolMessage>,
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

        // If start is not being called, main thread will die
        // Move these into start()
        let (register_done, register_handle, addr_sender) =
            start_pool_manager(server_address, Arc::clone(&pools)).unwrap();

        for address in addrs.into_iter() {
            addr_sender.send(PoolMessage::Add(address)).unwrap();
        }

        //Spawn thread for proactive messaging
        let (messenger_done, messenger_handle) =
            start_messenger(Arc::clone(&pools), capacity, server_address).unwrap();
        //

        ConnectionManager {
            server_address,
            pools,
            register_done,
            register_handle: Some(register_handle),
            messenger_done,
            messenger_handle: Some(messenger_handle),
            addr_sender,
        }
    }

    /// Add new connection to the pool
    pub fn add(&mut self, address: &str) -> PeerResult<()> {
        let socket_addr = address.parse().unwrap();
        self.addr_sender
            .send(PoolMessage::Add(socket_addr))
            .unwrap();
        Ok(())
    }

    pub fn start(&mut self) {
        // Todo: drop self afterwards
        let listener_pool = Arc::clone(&self.pools);
        let server_address = self.server_address;
        let addr_sender = self.addr_sender.to_owned();
        let _listener_handle = thread::spawn(move || {
            start_listener(listener_pool, addr_sender, server_address);
        });

        let pool = Arc::clone(&self.pools);
        let server_address = self.server_address;
        let msg_done = self.messenger_done.clone();
        ctrlc::set_handler(move || {
            broadcast(&pool, Exiting(server_address)).unwrap();
            // Clear the connection pool
            // This will signal all the connected nodes that they should remove
            // it from pool
            pool.lock().unwrap().clear();
            println!("Exiting");
            msg_done.store(true, Ordering::Relaxed);
            process::exit(0);
        })
        .expect("Error setting Ctrl-C handler");

        if let Some(thread) = self.messenger_handle.take() {
            thread.join().unwrap();
        }
    }
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        println!("Dropping");

        // self.broadcast(Exiting(self.server_address)).unwrap();

        //Drop all the pool
        for (_socket, conn) in self.pools.lock().unwrap().iter_mut() {
            conn.done.store(true, Ordering::Relaxed);
            if let Some(thread) = conn.send_thread.take() {
                thread.join().unwrap();
            }

            if let Some(thread) = conn.recv_thread.take() {
                thread.join().unwrap();
            }
        }

        self.register_done.store(true, Ordering::Relaxed);

        if let Some(thread) = self.register_handle.take() {
            thread.join().unwrap();
        }

        self.messenger_done.store(true, Ordering::Relaxed);
        if let Some(thread) = self.messenger_handle.take() {
            thread.join().unwrap();
        }
    }
}

///Start the network server by binding to given address
// If listener cannot be binded, let the program crash.
fn start_listener<T: 'static + ToSocketAddrs + Sync + Send>(
    pools: ConnectionPool,
    conn_adder: mpsc::Sender<PoolMessage>,
    address: T,
) {
    let listener = TcpListener::bind(address).unwrap();
    // accept connections and process them
    for stream in listener.incoming() {
        let conn_pools = Arc::clone(&pools);
        let conn_adder_c = conn_adder.clone();
        let conn_pools_c = Arc::clone(&pools);
        thread::spawn(move || {
            let stream = stream.unwrap();
            if let Ok(Request(socket_addr)) = read_message(&stream) {
                match is_connection_acceptable(&socket_addr, &conn_pools) {
                    None => {
                        send_message(&stream, Accepted).unwrap();
                        let conn = Connection::connect_stream(
                            socket_addr.to_owned(),
                            stream,
                            conn_pools_c,
                            conn_adder_c,
                        )
                        .unwrap();
                        conn_pools.lock().unwrap().insert(socket_addr, conn);
                    }
                    Some(err_message) => send_message(&stream, err_message).unwrap(),
                }
            } else {
                send_message(&stream, Denied).unwrap();
            }
        });
    }
}

fn start_messenger(
    conn_pool: ConnectionPool,
    capacity: usize,
    my_address: SocketAddr,
) -> PeerResult<(Arc<AtomicBool>, JoinHandle<()>)> {
    let messenger_done = Arc::new(AtomicBool::new(false));
    let messenger_done_c = Arc::clone(&messenger_done);
    let messender_handle = thread::spawn(move || {
        while !messenger_done_c.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(10));
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

    Ok((messenger_done, messender_handle))
}
