use std::net::{TcpStream, SocketAddr, TcpListener};
use std::io::prelude::*;
use std::io::{Error, ErrorKind};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

///Connection pool handling messages between peers
///TODO: Should make interface such that it limits the number of connection
type ConnectionPool = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

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
    pub fn new(addrs: Vec<&str>, server_address: Option<&'static str>) -> ConnectionManager {
        let pools = Arc::new(Mutex::new(HashMap::new()));
        for address in addrs.iter() {
            let conn_pool = Arc::clone(&pools);
            if Connection::from_addr(address, conn_pool).is_err() {
                continue;
            };
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
    pub fn broadcast(&self, message: SendMessage) -> Result<(), PoolError>{
        broadcast(Arc::clone(&self.pools), message)
    }

}

///Set of messages that can be sent
#[derive(Clone, Debug)]
pub enum SendMessage {
    Pong,
    Ping,
    World,
    NewBlock(usize),
    Peer(String),
}

use super::SendMessage::*;

///Set of message that can be received within the network
#[derive(Clone, Debug)]
pub enum RecvMessage {
    Ping,
    Hello,
    NewBlock(usize),
    Peer(String),
}

///Connection handles message handling between peers
///It will start send and recv thread once instantiated.
pub struct Connection {
    // Perhaps add id field?
    // Thread handle for send operation
    send_thread: Option<JoinHandle<()>>,
    // Thread handle for receive operation
    recv_thread: Option<JoinHandle<()>>,
    // Used to send message to the peer
    conn_sender: mpsc::Sender<SendMessage>,
    // Used to shutdown connection gracefully
    done: Arc<AtomicBool>,
}

impl Connection {
    /// Create TCPStream with given SocketAddr
    ///If connection is successful, instantiate `Connection` and insert it into
    ///given `ConnectionPool`
    fn from_addr
        ( address: &str, 
          conn_pool: ConnectionPool,
        ) -> std::io::Result<()> {
        let socket_addr: SocketAddr = match address.parse(){
            Ok(socket) => socket,
            Err(_) => return Err(Error::new(ErrorKind::Other, "Invalid address")),
        };
        let stream = TcpStream::connect(socket_addr)?;
        Connection::new(stream, Arc::clone(&conn_pool))
    }

    ///Instantiate `Connection` and insert it into `ConnectionPool`
    fn new(stream: TcpStream, conn_pool: ConnectionPool) -> std::io::Result<()> {
        let (conn_sender, conn_receiver) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = Arc::clone(&done);
        let mut send_stream = stream.try_clone()?;
        send_stream.write(b"Connection established\n").unwrap();
        let send_thread = thread::spawn(move || {
            while !send_done.load(Ordering::Relaxed) {
                // Can you make it such that if any of the function fails,
                // it does the same restore action?
                let message = conn_receiver.recv()
                    .expect("Unable to receive message");
                //If write fails due to broken pipe, close the threads
                if send_stream.write(format!("{:?}\n", &message).as_bytes()).is_err() {
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
                // Implement:
                // Message handling
                // Deserialize the recv message
                let mut buffer = [0u8; 512];
                if let Ok(_) = recv_stream.read(&mut buffer) {
                    let ping = b"Ping";
                    if buffer.starts_with(ping) {
                        broadcast(Arc::clone(&conn_pool), Pong)
                            .expect("Unable to perform broadcast");
                    }
                };
                recv_stream.flush().expect("Unable to flush"); //?
            }
        });
        
        let conn = Connection {
            send_thread: Some(send_thread),
            recv_thread: Some(recv_thread),
            conn_sender,
            done,
        };

        //Store it in the connection pool
        //Not sure if this is good idea..
        read_pool.lock().unwrap().insert(stream.peer_addr().unwrap(), conn);

        Ok(())
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
fn broadcast(pools: ConnectionPool, message: SendMessage) -> Result<(), PoolError> {
    // If any of the `send` fails, store the `SocketAddr` here so it can later be
    // removed.
    let mut failed_addr = Vec::new();
    for (socket_addr, conn) in pools.lock().unwrap().iter_mut() {
        if conn.conn_sender.send(message.clone()).is_err() {
            failed_addr.push(socket_addr.clone());
        };
    }

    for socket_addr in failed_addr.iter() {
        pools.lock().unwrap().remove(socket_addr);
    }

    Ok(())
}

///Start the network server by binding to given address
fn start_listener(
    pools: ConnectionPool,
    address: &'static str) -> std::io::Result<JoinHandle<()>> {
    let handle = thread::spawn(move || {
        let listener = TcpListener::bind(address).unwrap();
        // accept connections and process them
        for stream in listener.incoming() {
            let conn_pools = Arc::clone(&pools);
            thread::spawn(move || {
                Connection::new(stream.unwrap(), conn_pools).unwrap();
            });
        } 
    });
    Ok(handle)
}