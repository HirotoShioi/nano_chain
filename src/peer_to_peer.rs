use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::error;
use std::io::prelude::*;
use std::io::BufReader;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
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

        self.broadcast(Exit).unwrap();

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

#[derive(Debug)]
pub enum PoolError {
    NoPool,
    FailedToCreateConnection,
    UnableToConnect,
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
        let (register_done, register_handle, addr_sender) =
            start_connection_registerer(Arc::clone(&pools)).unwrap();

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
        broadcast(Arc::clone(&self.pools), message)
    }

    pub fn start(&self) {
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
    Request(SocketAddr),
    Accept,
    Denied,
    NewBlock(usize),
    /// Broadcast new block when minted
    ReplyBlock(usize),
    /// If you have bigger number, reply with this message
    AskPeer(SocketAddr, Vec<SocketAddr>, usize), // Address are connnected
    ReplyPeer(Vec<SocketAddr>, usize),
    Exit,
}

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
                let message = conn_receiver.recv().expect("Unable to receive message");
                //If write fails due to broken pipe, close the threads
                if send_message(&send_stream, message).is_err() {
                    send_done.store(true, Ordering::Relaxed);
                };
                send_stream.flush().expect("Unable to flush stream");
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

        let peer_addr = stream.peer_addr()?;

        let conn = Connection {
            address: peer_addr,
            stream: stream,
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
        address: SocketAddr,
        conn_pool: ConnectionPool,
        conn_adder: mpsc::Sender<SocketAddr>,
    ) -> PeerResult<Connection> {
        let stream = TcpStream::connect(address)?;
        let conn = Connection::connect_stream(stream, conn_pool, conn_adder).unwrap();
        Ok(conn)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        println!("Lost connection: {:?}", self.address);

        self.done.store(true, Ordering::Relaxed);
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
fn broadcast(pools: ConnectionPool, message: ProtocolMessage) -> PeerResult<()> {
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
            let conn =
                Connection::connect_stream(stream, conn_pools, conn_adder_c).unwrap();
            conn_pools_c.lock().unwrap().insert(conn.address, conn);
        });
    }
}

///Read messages from the `TcpStream` and handle them accordingly
fn handle_recv_message(
    stream: &TcpStream,
    pool: ConnectionPool,
    conn_sender: mpsc::Sender<SocketAddr>,
) -> PeerResult<()> {
    if let Ok(recv_message) = read_message(&stream) {
        println!("Got message: {:?}", recv_message);
        match recv_message {
            Ping => send_message(&stream, Pong)?,
            Request(socket_addr) => {
                conn_sender.send(socket_addr)?; // Handling!
            }
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
            _ => {}
        }
    };
    Ok(())
}

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
    Ok(recv_message)
}

fn start_connection_registerer(
    conn_pool: ConnectionPool,
) -> PeerResult<(Arc<AtomicBool>, JoinHandle<()>, mpsc::Sender<SocketAddr>)> {
    let (tx, rx) = mpsc::channel::<SocketAddr>();
    let is_done = Arc::new(AtomicBool::new(false));
    let is_done_c = Arc::clone(&is_done);
    let tx_c = tx.clone();
    let handle = thread::spawn(move || {
        while !is_done_c.load(Ordering::Relaxed) {
            let socket_addr = rx.recv().unwrap();
            let mut conn_pool_locked = conn_pool.lock().unwrap();
            if !conn_pool_locked.contains_key(&socket_addr) {
                if let Ok(conn) =
                    Connection::connect(socket_addr, Arc::clone(&conn_pool), tx.clone())
                {
                    println!("New connection: {:?}", &conn.address);
                    conn_pool_locked.insert(conn.address, conn);
                };
            }
        }
    });
    Ok((is_done, handle, tx_c))
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
                broadcast(conn_pool_c, ask_message).unwrap();
            }
        }
    });

    Ok((messenger_done, messender_handle))
}
