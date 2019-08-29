use std::io::prelude::*;
use std::io::BufReader;
use std::error;
use std::net::{TcpStream, SocketAddr, TcpListener, ToSocketAddrs};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json;

///Connection pool handling messages between peers
///TODO: Should make interface such that it limits the number of connection
//Bug: Inserting existing address to the pool will cause main thread to block indefinately
type ConnectionPool = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

type PeerResult<T> = Result<T, Box<dyn error::Error>>;

///Connection manager is responsible of managing listener.
///It also should provide interface that other modules can use
pub struct ConnectionManager {
    pools: ConnectionPool,
    server_handle: Option<JoinHandle<()>>,
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        if let Some(thread) = self.server_handle.take() {
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

use super::PoolError::*;

impl ConnectionManager {
    /// Create an instance of `ConnectionManager`
    /// 
    /// You can provide vector of addresses which can be used to connect to the other
    /// nodes initially as well as launching server by providin `server_address`
    pub fn new <T:'static + ToSocketAddrs, U: 'static + ToSocketAddrs + Send + Sync>
    (addrs: T, server_address: Option<U>) -> ConnectionManager
    {
        let pools = Arc::new(Mutex::new(HashMap::new()));
        for address in addrs.to_socket_addrs().unwrap() {
            let conn_pool = Arc::clone(&pools);
            let conn = Connection::new(address);
        }

        let server_handle = match server_address {
            Some(address) => Some(start_listener(Arc::clone(&pools), address).unwrap()),
            None => None,
        };

        ConnectionManager {
            pools,
            server_handle,
        }
    }

    /// Add new connection to the pool
    pub fn add(&mut self, address: &str) -> Result<(), PoolError> {
        let conn_pool = Arc::clone(&self.pools);
        if Connection::from_addr(address, conn_pool).is_err() {
            return Err(FailedToCreateConnection)
        }
        Ok(())
    }

    /// Broadcast `SendMessage` to the known peer
    pub fn broadcast(&self, message: SendMessage) -> PeerResult<()>{
        broadcast(Arc::clone(&self.pools), message)
    }

}

///Set of messages that can be sent
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SendMessage {
    Pong,
    Ping,
    NewBlock(usize),
    Peer(String),
    AskPeer,
}

use super::SendMessage::*;

///Set of message that can be received within the network
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum RecvMessage {
    Ping,
    Pong,
    NewBlock(usize),
    Peer(String),
}

///Connection handles message handling between peers
///It will start send and recv thread once instantiated.
pub struct Connection {
    // Perhaps add id field?
    address: SocketAddr,
    // Thread handle for send operation
    send_thread: Option<JoinHandle<()>>,
    // Thread handle for receive operation
    recv_thread: Option<JoinHandle<()>>,
    // Used to send message to the peer
    conn_sender: Option<mpsc::Sender<SendMessage>>,
    // Used to shutdown connection gracefully
    done: Arc<AtomicBool>,
}

impl Connection {
    //`new` will return `Connection` with nothing being initialized
    fn new(address: SocketAddr) -> Connection {
        Connection {
            address,
            send_thread: None,
            recv_thread: None,
            conn_sender: None,
            done: Arc::new(AtomicBool::new(false)),
        }
    }
    //pass it to the `ConnectionManager` and let the manager handle the initialization

    /// Create TCPStream with given SocketAddr
    ///If connection is successful, instantiate `Connection` and insert it into
    ///given `ConnectionPool`
    fn from_addr<T: ToSocketAddrs>
        ( address: T, 
          conn_pool: ConnectionPool,
        ) -> PeerResult<()> {
        let stream = TcpStream::connect(address)?;
        Connection::connect_stream(stream, Arc::clone(&conn_pool))
    }

    ///Instantiate `Connection` and insert it into `ConnectionPool`
    fn connect_stream(stream: TcpStream, conn_pool: ConnectionPool) -> PeerResult<()> {
        //Check if address aleady exists in the pool
        let (conn_sender, conn_receiver) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = Arc::clone(&done);
        let mut send_stream = stream.try_clone()?;
        send_message(&send_stream, Ping)?;
        let send_thread = thread::spawn(move || {
            while !send_done.load(Ordering::Relaxed) {
                // Can you make it such that if any of the function fails,
                // it does the same restore action?
                let message = conn_receiver.recv()
                    .expect("Unable to receive message");
                //If write fails due to broken pipe, close the threads
                if send_message(&send_stream, message).is_err() {
                    send_done.store(true, Ordering::Relaxed);
                };
                send_stream.flush()
                    .expect("Unable to flush stream");
            }      
        });

        //Handles incoming message
        let read_done = Arc::clone(&done);
        let read_pool = Arc::clone(&conn_pool);
        let mut recv_stream = stream.try_clone()?;
        recv_stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        let recv_thread = thread::spawn(move || {
            while !read_done.load(Ordering::Relaxed) {
                if handle_recv_message(&recv_stream, Arc::clone(&conn_pool)).is_ok() {
                    recv_stream.flush().unwrap();
                }
            }
        });
        
        let peer_addr = stream.peer_addr()?;

        let conn = Connection {
            address: peer_addr,
            send_thread: Some(send_thread),
            recv_thread: Some(recv_thread),
            conn_sender: Some(conn_sender),
            done,
        };

        //Store it in the connection pool
        //Delete this, let manager take care of it.
        read_pool.lock().unwrap().insert(peer_addr, conn);

        Ok(())
    }

    fn connect2(&mut self, conn_pool: ConnectionPool, conn_adder: mpsc::Sender<(SocketAddr, &mut Connection)>) -> PeerResult<Connection> {
        let stream = TcpStream::connect(&self.address)?;
        //Check if address aleady exists in the pool
        let (conn_sender, conn_receiver) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = Arc::clone(&done);
        let mut send_stream = stream.try_clone()?;
        send_message(&send_stream, Ping)?;
        let send_thread = thread::spawn(move || {
            while !send_done.load(Ordering::Relaxed) {
                // Can you make it such that if any of the function fails,
                // it does the same restore action?
                let message = conn_receiver.recv()
                    .expect("Unable to receive message");
                //If write fails due to broken pipe, close the threads
                if send_message(&send_stream, message).is_err() {
                    send_done.store(true, Ordering::Relaxed);
                };
                send_stream.flush()
                    .expect("Unable to flush stream");
            }      
        });

        //Handles incoming message
        let read_done = Arc::clone(&done);
        let mut recv_stream = stream.try_clone()?;
        recv_stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        let recv_thread = thread::spawn(move || {
            while !read_done.load(Ordering::Relaxed) {
                if handle_recv_message(&recv_stream, Arc::clone(&conn_pool)).is_ok() {
                    recv_stream.flush().unwrap();
                }
            }
        });
        
        let peer_addr = stream.peer_addr()?;

        let conn = Connection {
            address: peer_addr,
            send_thread: Some(send_thread),
            recv_thread: Some(recv_thread),
            conn_sender: Some(conn_sender),
            done,
        };

        Ok(conn)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(thread) = self.send_thread.take() {
            thread.join()
                .expect("Unable to close send_thread");
        }

        if let Some(thread) = self.recv_thread.take() {
            thread.join()
                .expect("Unable to close recv_thread");
        }
    }
}

/// Broadcast given `message` to the known peers
fn broadcast(pools: ConnectionPool, message: SendMessage) -> PeerResult<()> {
    // If any of the `send` fails, store the `SocketAddr` here so it can later be
    // removed.
    let mut failed_addr = Vec::new();
    for (socket_addr, conn) in pools.lock().unwrap().iter_mut() {
        if let Some(sender) = &conn.conn_sender {
            if sender.send(message.clone()).is_err() {
                failed_addr.push(socket_addr.clone());
            };
        };
    }

    for socket_addr in failed_addr.iter() {
        pools.lock().unwrap().remove(socket_addr);
    }

    Ok(())
}

///Start the network server by binding to given address
fn start_listener<T:'static + ToSocketAddrs + Sync + Send>(
    pools: ConnectionPool,
    address: T) -> std::io::Result<JoinHandle<()>> {
    let handle = thread::spawn(move || {
        let listener = TcpListener::bind(address).unwrap();
        // accept connections and process them
        for stream in listener.incoming() {
            let conn_pools = Arc::clone(&pools);
            thread::spawn(move || {
                Connection::connect_stream(stream.unwrap(), conn_pools).unwrap();
            });
        } 
    });
    Ok(handle)
}

///Read messages from the `TcpStream` and handle them accordingly
fn handle_recv_message(stream: &TcpStream, pool: ConnectionPool) -> PeerResult<()>{
    let mut reader = BufReader::new(stream);
    let mut buffer = String::new();
    if reader.read_line(&mut buffer).is_ok() {
        let recv_message: RecvMessage = serde_json::from_str(&buffer)?;
        println!("Got message: {:?}", recv_message);
        match recv_message {
            RecvMessage::Ping => send_message(&stream, Pong)?,
            RecvMessage::Pong => {},
            RecvMessage::NewBlock(num) => broadcast(Arc::clone(&pool), NewBlock(num))?,
            RecvMessage::Peer(addr) => { 
                Connection::from_addr(&addr, Arc::clone(&pool))?;
            }
        }
    }
    Ok(())
}

///Send `SendMessage` the given stream
fn send_message(stream: &TcpStream, send_message: SendMessage) -> PeerResult<()>{
    let mut stream_clone = stream.try_clone()?;
    serde_json::to_writer(stream, &send_message)?;
    stream_clone.write_all(b"\n")?;
    stream_clone.flush()?;
    Ok(())
}

fn start_connection_registerer(conn_pool: ConnectionPool) 
    -> PeerResult<(Arc<AtomicBool>, JoinHandle<()>, mpsc::Sender<(SocketAddr, &'static mut Connection)>)> {
    let (tx, rx) = mpsc::channel::<(SocketAddr, &mut Connection)>();
    let is_done = Arc::new(AtomicBool::new(false));
    let is_done_c = Arc::clone(&is_done);
    let tx_c = tx.clone();
    let handle = thread::spawn(move || {
        while !is_done_c.load(Ordering::Relaxed) {
            let (socket_addr, conn) = rx.recv().unwrap();
            if !conn_pool.lock().unwrap().contains_key(&socket_addr) {
                let conn = conn.connect2(Arc::clone(&conn_pool), tx.clone()).unwrap();
                conn_pool.lock().unwrap().insert(socket_addr, conn);
            }
        }
    });
    Ok((is_done, handle, tx_c))
}