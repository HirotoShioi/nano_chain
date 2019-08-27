use std::net::{TcpStream, SocketAddr, TcpListener};
use std::io::prelude::*;
use std::io::{Error, ErrorKind};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

//Connection pool handling messages between peers
type ConnectionPool = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

pub struct ConnectionManager {
    pools: ConnectionPool,
    handle: Option<JoinHandle<()>>,
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        if let Some(thread) = self.handle.take() {
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
    pub fn new(addrs: Vec<&str>, server_address: Option<&'static str>) -> ConnectionManager {
        let pools = Arc::new(Mutex::new(HashMap::new()));
        for address in addrs.iter() {
            let conn_pool = Arc::clone(&pools);
            if Connection::from_addr(address, conn_pool).is_err() {
                continue;
            };
        }

        let handle = match server_address {
            Some(address) => Some(start_listener(Arc::clone(&pools), address).unwrap()),
            None => None,
        };

        ConnectionManager {
            pools,
            handle,
        }
    }

    pub fn add(&mut self, address: &str) -> Result<(), PoolError> {
        let conn_pool = Arc::clone(&self.pools);
        if Connection::from_addr(address, conn_pool).is_err() {
            return Err(FailedToCreateConnection)
        }

        Ok(())
    }

    pub fn broadcast(&self, message: SendMessage) -> Result<(), PoolError>{
        broadcast(Arc::clone(&self.pools), message)
    }

    pub fn start_server(&mut self, address: &'static str) -> Result<(), PoolError> {
        let conn_pools = Arc::clone(&self.pools);
        let handle = thread::spawn(move || {
            let listener = TcpListener::bind(address).unwrap();
            // accept connections and process them
            for stream in listener.incoming() {
                let conn_pools = Arc::clone(&conn_pools); // Used to instansiate Connection
                thread::spawn(move || {
                    let stream = stream.unwrap();
                    Connection::new(stream, conn_pools).unwrap();
                });
            } 
        });

        self.handle.replace(handle);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum SendMessage {
    Pong,
    Ping,
    World,
    NewBlock(usize),
    Peer(String),
}

use super::SendMessage::*;

#[derive(Clone, Debug)]
pub enum RecvMessage {
    Ping,
    Hello,
    NewBlock(usize),
    Peer(String),
}

pub struct Connection {
    // Perhaps add id field?
    // Thread handle for send operation
    send_thread: Option<JoinHandle<()>>,
    // Thread handle for receive operation
    recv_thread: Option<JoinHandle<()>>,
    // Used to send message to the connection
    conn_sender: mpsc::Sender<SendMessage>,
    // Used to shutdown connection gracefully
    done: Arc<AtomicBool>,
}

impl Connection {
    pub fn from_addr
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

    pub fn new(stream: TcpStream, conn_pool: ConnectionPool) -> std::io::Result<()> {
        let (conn_sender, conn_receiver) = mpsc::channel();

        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = Arc::clone(&done);
        let mut send_stream = stream.try_clone()?;

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

        read_pool.lock().unwrap().insert(stream.peer_addr().unwrap(), conn);

        Ok(())
    }
}

// Implementd drop for `Connection`
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

fn broadcast(pools: ConnectionPool, message: SendMessage) -> Result<(), PoolError> {
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

fn start_listener(pools: ConnectionPool, address: &'static str) -> std::io::Result<JoinHandle<()>> {
    let handle = thread::spawn(move || {
        let listener = TcpListener::bind(address).unwrap();
        // accept connections and process them
        for stream in listener.incoming() {
            let conn_pools = Arc::clone(&pools); // Used to instansiate Connection
            thread::spawn(move || {
                Connection::new(stream.unwrap(), conn_pools).unwrap();
            });
        } 
    });
    Ok(handle)
}