use ctrlc;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::error;
use std::fmt;
use std::io::prelude::*;
use std::io::BufReader;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

///Connection pool handling messages between peers
type ConnectionPool = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

type PeerResult<T> = Result<T, Box<dyn error::Error>>;

///Connection manager is responsible of managing listener.
///It also should provide interface that other modules can use
pub struct ConnectionManager {
    server_address: SocketAddr,
    pools: ConnectionPool,
    register_done: Arc<AtomicBool>,
    register_handle: Option<JoinHandle<()>>,
    messenger_done: Arc<AtomicBool>,
    messenger_handle: Option<JoinHandle<()>>,
    addr_sender: mpsc::Sender<SocketAddr>,
    started: bool,
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        println!("Dropping");

        self.broadcast(Exiting(self.server_address)).unwrap();

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
    conn_adder: mpsc::Sender<SocketAddr>,
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

fn start_connection_registerer(
    my_addr: SocketAddr,
    conn_pool: ConnectionPool,
) -> PeerResult<(Arc<AtomicBool>, JoinHandle<()>, mpsc::Sender<SocketAddr>)> {
    let (tx, rx) = mpsc::channel::<SocketAddr>();
    let is_done = Arc::new(AtomicBool::new(false));
    let is_done_c = Arc::clone(&is_done);
    let tx_c = tx.clone();
    let handle = thread::spawn(move || {
        while !is_done_c.load(Ordering::Relaxed) {
            let socket_addr = rx.recv().unwrap();
            if is_connection_acceptable(&socket_addr, &conn_pool).is_none() {
                if let Ok(conn) =
                    Connection::connect(my_addr, socket_addr, Arc::clone(&conn_pool), tx.clone())
                {
                    println!("New connection: {:?}", &conn.address);
                    conn_pool.lock().unwrap().insert(conn.address, conn);
                };
            }
        }
    });
    Ok((is_done, handle, tx_c))
}

#[derive(Debug)]
pub enum PoolError {
    NoPool,
    FailedToCreateConnection,
    UnableToConnect,
    ConnectionDenied,
}

use super::PoolError::*;

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            NoPool => "No connection available",
            FailedToCreateConnection => "Failed to create connection",
            UnableToConnect => "Unable to connect to the peer",
            ConnectionDenied => "Connection request was rejected",
        };
        write!(f, "{}", msg)
    }
}

impl error::Error for PoolError {}

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
        let (register_done, register_handle, addr_sender) =
            start_connection_registerer(server_address, Arc::clone(&pools)).unwrap();

        for address in addrs.into_iter() {
            addr_sender.send(address).unwrap();
        }

        //Spawn thread for proactive messaging
        let (messenger_done, messenger_handle) =
            start_messenger(Arc::clone(&pools), capacity, server_address).unwrap();

        ConnectionManager {
            server_address,
            pools,
            register_done,
            register_handle: Some(register_handle),
            messenger_done,
            messenger_handle: Some(messenger_handle),
            addr_sender,
            started: false,
        }
    }

    /// Add new connection to the pool
    pub fn add(&mut self, address: &str) -> Result<(), PoolError> {
        let socket_addr = address.parse().unwrap();
        self.addr_sender.send(socket_addr).unwrap();
        Ok(())
    }

    /// Broadcast `ProtocolMessage` to the known peer
    pub fn broadcast(&self, message: ProtocolMessage) -> PeerResult<()> {
        assert!(self.started, "Please start the server");
        broadcast(&self.pools, message)
    }

    pub fn start(&mut self) {
        let pool = Arc::clone(&self.pools);
        let server_address = self.server_address;
        ctrlc::set_handler(move || {
            broadcast(&pool, Exiting(server_address)).unwrap();
            // Clear the connection pool
            // This will signal all the connected nodes that they should remove
            // it from pool
            pool.lock().unwrap().clear();
            println!("Exiting");
            process::exit(0);
        })
        .expect("Error setting Ctrl-C handler");

        // Todo: drop self afterwards
        start_listener(
            Arc::clone(&self.pools),
            self.addr_sender.to_owned(),
            self.server_address,
        );
    }
}

//------------------------------------------------------------------------------
// Protocol
//------------------------------------------------------------------------------

///Set of messages that can be sent
use super::peer_to_peer::ProtocolMessage::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ProtocolMessage {
    Ping,
    Pong,
    // Handshake
    Request(SocketAddr),
    Accepted,
    Denied,
    CapacityReached,
    AlreadyConnected, // Should be more specific (CapacityReached, AlreadyConnected, Denied)
    // Sharing states
    NewBlock(usize),
    /// Broadcast new block when minted
    ReplyBlock(usize),
    /// Exchange information about peers
    AskPeer(SocketAddr, Vec<SocketAddr>, usize), // Address are connnected
    ReplyPeer(Vec<SocketAddr>, usize),
    Exiting(SocketAddr),
}


// Want to make these functions generic

///Send `ProtocolMessage` the given stream
fn send_message(stream: &TcpStream, message: ProtocolMessage) -> PeerResult<()> {
    let peer_addr = stream.peer_addr().unwrap();
    println!("Sending message to: {:?},  {:?}", peer_addr, message);
    let mut stream_clone = stream.try_clone()?;
    serde_json::to_writer(stream, &message)?;
    stream_clone.write_all(b"\n")?;
    stream_clone.flush()?;
    Ok(())
}

fn read_message(stream: &TcpStream) -> PeerResult<ProtocolMessage> {
    let mut reader = BufReader::new(stream);
    let mut buffer = String::new();
    reader.read_line(&mut buffer)?;
    let recv_message: ProtocolMessage = serde_json::from_str(&buffer)?;
    println!("Got message: {:?}", recv_message);
    Ok(recv_message)
}

// -----------------------------------------------------------------------------
// Connection
// -----------------------------------------------------------------------------

///Connection handles message handling between peers
///It will start send and recv thread once instantiated.
pub struct Connection {
    // Perhaps add id field?
    address: SocketAddr,
    stream: TcpStream,
    // Thread handle for send operation
    send_thread: Option<JoinHandle<()>>,
    // Thread handle for receive operation
    recv_thread: Option<JoinHandle<()>>,
    // Used to send message to the peer
    conn_sender: Option<mpsc::Sender<ProtocolMessage>>,
    // Used to shutdown connection gracefully
    done: Arc<AtomicBool>,
}

impl Connection {
    ///Instantiate `Connection` and insert it into `ConnectionPool`
    fn connect_stream(
        socket_addr: SocketAddr,
        stream: TcpStream,
        conn_pool: ConnectionPool,
        conn_adder: mpsc::Sender<SocketAddr>,
    ) -> PeerResult<Connection> {
        //Check if address aleady exists in the pool
        let (conn_sender, conn_receiver) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = Arc::clone(&done);
        let mut send_stream = stream.try_clone()?;
        let send_thread = thread::spawn(move || {
            while !send_done.load(Ordering::Relaxed) {
                // Can you make it such that if any of the function fails,
                // it does the same restore action?
                if let Ok(message) = conn_receiver.recv() {
                    //If write fails due to broken pipe, close the threads
                    // Make it cleaner
                    if send_message(&send_stream, message).is_err() {
                        drop(&send_stream);
                        send_done.store(true, Ordering::Relaxed);
                    };
                    send_stream.flush().expect("Unable to flush stream");
                };
            }
        });

        //Handles incoming message
        let read_done = Arc::clone(&done);
        let mut recv_stream = stream.try_clone()?;
        recv_stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        let recv_thread = thread::spawn(move || {
            while !read_done.load(Ordering::Relaxed) {
                if handle_recv_message(&recv_stream, Arc::clone(&conn_pool), conn_adder.clone())
                    .is_ok()
                {
                    recv_stream.flush().unwrap();
                }
            }
        });

        let conn = Connection {
            address: socket_addr,
            stream,
            send_thread: Some(send_thread),
            recv_thread: Some(recv_thread),
            conn_sender: Some(conn_sender),
            done,
        };

        Ok(conn)
    }

    /// Create TCPStream with given SocketAddr
    ///If connection is successful, instantiate `Connection`
    fn connect(
        my_address: SocketAddr,
        address: SocketAddr,
        conn_pool: ConnectionPool,
        conn_adder: mpsc::Sender<SocketAddr>,
    ) -> PeerResult<Connection> {
        let stream = TcpStream::connect(address)?;
        send_message(&stream, Request(my_address))?;
        if let Ok(Accepted) = read_message(&stream) {
            let conn = Connection::connect_stream(address, stream, conn_pool, conn_adder).unwrap();
            Ok(conn)
        } else {
            Err(Box::new(ConnectionDenied))
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(sender) = self.conn_sender.take() {
            drop(sender);
        }

        if let Some(thread) = self.send_thread.take() {
            thread.join().expect("Unable to close send_thread");
        }

        if let Some(thread) = self.recv_thread.take() {
            thread.join().expect("Unable to close recv_thread");
        }

        if self.stream.shutdown(Shutdown::Both).is_err() {};
    }
}

/// Broadcast given `message` to the known peers
fn broadcast(pools: &ConnectionPool, message: ProtocolMessage) -> PeerResult<()> {
    let mut pools = pools.lock().unwrap();
    // If any of the `send` fails, store the `SocketAddr` here so it can later be
    // removed.
    let mut failed_addr = Vec::new();

    for (socket_addr, conn) in pools.iter_mut() {
        if let Some(sender) = &conn.conn_sender {
            if sender.send(message.to_owned()).is_err() {
                failed_addr.push(socket_addr.to_owned());
            };
        };
    }

    for socket_addr in failed_addr.iter() {
        pools.remove(socket_addr);
    }

    Ok(())
}

///Read messages from the `TcpStream` and handle them accordingly
fn handle_recv_message(
    stream: &TcpStream,
    pool: ConnectionPool,
    conn_sender: mpsc::Sender<SocketAddr>,
) -> PeerResult<()> {
    if let Ok(recv_message) = read_message(&stream) {
        match recv_message {
            Ping => send_message(&stream, Pong)?,
            NewBlock(_num) => {
                //check if new number is bigger
                //if yes, update it and broadcast to the others
            }
            ReplyBlock(_num) => {
                //check if new number is bigger
                //if yes, update it and broadcast to the others
            }
            ReplyPeer(socket_addresses, size) => {
                socket_addresses.to_owned().truncate(size);
                for socket_addr in socket_addresses {
                    // Registerer will handle filtering
                    conn_sender.send(socket_addr)?;
                }
            }
            AskPeer(their_address, their_known_address, _size) => {
                let mut their_known_address = their_known_address.to_owned();
                their_known_address.push(their_address);
                // Bugged
                let new_addresses: Vec<SocketAddr> = pool
                    .lock()
                    .unwrap()
                    .keys()
                    .filter(|key| !their_known_address.contains(key))
                    .map(|k| k.to_owned())
                    .collect();

                let addr_len = new_addresses.len();
                let msg = ReplyPeer(new_addresses, addr_len);
                send_message(stream, msg).unwrap();
            }
            Exiting(socket_addr) => {
            },
            _ => {},
        }
    };
    Ok(())
}

//This can be tested!
fn is_connection_acceptable(
    socket_addr: &SocketAddr,
    conn_pool: &ConnectionPool,
) -> Option<ProtocolMessage> {
    let conn_pool = conn_pool.lock().unwrap();
    if conn_pool.contains_key(socket_addr) {
        Some(AlreadyConnected)
    } else if conn_pool.len() >= conn_pool.capacity() {
        Some(CapacityReached)
    } else {
        None
    }
}
