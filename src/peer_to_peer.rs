use std::net::{TcpStream, SocketAddr};
use std::io::prelude::*;
use std::io::{Error, ErrorKind};
use std::thread::{self, JoinHandle};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};

type ConnectionPool = Arc<Mutex<Vec<Connection>>>;

pub struct ConnectionManager {
    //Connection pool handling messages between peers
    pools: ConnectionPool,
    started: bool,
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        for conn in self.pools.lock().unwrap().iter() {
            conn.done.store(true, Ordering::Relaxed);
        }

        for conn in self.pools.lock().unwrap().iter_mut() {
            if let Some(thread) = conn.send_thread.take() {
                thread.join().expect("Unable to close thread");
            };

            if let Some(thread) = conn.recv_thread.take() {
                thread.join().expect("Unable to close thread");
            };            
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
    pub fn new(addrs: Vec<&str>) -> ConnectionManager {
        let pools = Arc::new(Mutex::new(Vec::new()));
        for address in addrs.iter() {
            let conn_pool = Arc::clone(&pools);
            if let Ok(conn) = Connection::new(address, conn_pool) {
                pools.lock().unwrap().push(conn);
            }
        }
        
        //start server here

        ConnectionManager {
            pools,
            started: true,
        }
    }

    pub fn add(&mut self, address: &str) -> Result<(), PoolError> {
        let conn_pool = Arc::clone(&self.pools);
        if let Ok(conn) = Connection::new(address, conn_pool){
            self.pools.lock().unwrap().push(conn);
        };

        Ok(())
    }

    pub fn broadcast(&self, message: SendMessage) -> Result<(), PoolError>{
        broadcast(Arc::clone(&self.pools), message)
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

use super::RecvMessage::*;

pub struct Connection {
    // Perhaps add id field?
    // Address
    socket_addr: SocketAddr,
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
    // Pool must provide recv end of channel for sending
    pub fn new
        ( address: &str, 
          conn_pool: ConnectionPool,
        ) -> std::io::Result<Connection> {
        let socket_addr = match address.parse(){
            Ok(socket) => socket,
            Err(_) => return Err(Error::new(ErrorKind::Other, "Invalid address")),
        };
        let stream = TcpStream::connect(socket_addr)?;
    
        let (conn_sender, conn_receiver) = mpsc::channel();

        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = Arc::clone(&done);
        let mut send_stream = stream.try_clone()?;
        let send_thread = thread::spawn(move || {
            while !send_done.load(std::sync::atomic::Ordering::Relaxed) {
                let message = conn_receiver.recv()
                    .expect("Unable to receive message");
                send_stream.write(format!("{:?}\n", &message).as_bytes())
                    .expect("Unable to write message");
                send_stream.flush()
                    .expect("Unable to flush stream");
            }      
        });

        //Handles incoming message
        let read_done = Arc::clone(&done);
        let mut recv_stream = stream.try_clone()?;
        recv_stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        let recv_thread = thread::spawn(move || {
            while !read_done.load(std::sync::atomic::Ordering::Relaxed) {
                // Implement:
                // Message handling
                // Deserialize the recv message
                let mut buffer = [0; 512];
                if let Ok(_) = recv_stream.read(&mut buffer) {
                    let ping = b"Ping";
                    if buffer.starts_with(ping) {
                        println!("Got ping");
                        broadcast(Arc::clone(&conn_pool), Pong)
                            .expect("Unable to perform broadcast");
                    }
                };
            }
        });

        Ok(Connection {
            socket_addr,
            send_thread: Some(send_thread),
            recv_thread: Some(recv_thread),
            conn_sender,
            done,
        })
    }
}

// Implementd drop for `Connection`
impl Drop for Connection {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(thread) = self.send_thread.take() {
            thread.join()
                .expect("Unable to close thread");
        }

        if let Some(thread) = self.recv_thread.take() {
            thread.join()
                .expect("Unable to close thread");
        }
    }
}

fn broadcast(pools: Arc<Mutex<Vec<Connection>>>, message: SendMessage) -> Result<(), PoolError> {
    for conn in pools.lock().unwrap().iter() {
        if let Err(_) = conn.conn_sender.send(message.clone()) {
            //pools.lock().unwrap().remove_item(&conn);
            drop(conn);
        };
    }
    Ok(())
}