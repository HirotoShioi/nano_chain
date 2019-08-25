use std::net::{TcpStream, SocketAddr};
use std::io::prelude::*;
use std::thread::{self, JoinHandle};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

pub struct ConnectionPool {
    //Connection pool handling messages between peers
    pools: Arc<Mutex<HashMap<String, Connection>>>,
    //Copy of broadcast send end, when spawning new connections, pass this
    sender: mpsc::Sender<RecvMessage>,
    receiver: mpsc::Receiver<RecvMessage>,
    started: bool,
}

impl Drop for ConnectionPool {
    fn drop(&mut self) {
        for conn in self.pools.lock().unwrap().values_mut() {
            conn.done.store(true, Ordering::Relaxed);
        }

        for conn in self.pools.lock().unwrap().values_mut() {
            if let Some(thread) = conn.send_thread.take() {
                thread.join().unwrap();
            };

            if let Some(thread) = conn.recv_thread.take() {
                thread.join().unwrap();
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

impl ConnectionPool {
    pub fn new(addrs: Vec<&str>) -> ConnectionPool {
        let mut pools = HashMap::new();
        let (sender, receiver) = mpsc::channel();

        for address in addrs.iter() {
            let conn_sender = sender.clone();
            if let Ok(conn) = Connection::new(address, conn_sender) {
                pools.insert(address.to_string(), conn);
            }
        }

        ConnectionPool {
            pools: Arc::new(Mutex::new(pools)),
            sender,
            receiver,
            started: false,
        }
    }

    pub fn add(&mut self, address: &str) -> Result<(), PoolError> {

        let conn = Connection::new(address, self.sender.clone())?;

        self.pools.lock().unwrap().insert(address.to_string(), conn);

        Ok(())
    }

    pub fn broadcast(&self, message: SendMessage) -> Result<(), PoolError>{
        
        if self.pools.lock().unwrap().len() <= 0 {
            return Err(PoolError::NoPool)
        }

        for conn in self.pools.lock().unwrap().values() {
            conn.conn_sender.send(message.clone()).unwrap();
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum SendMessage {
    Pong,
    World,
    NewBlock(usize),
}

#[derive(Clone, Debug)]
pub enum RecvMessage {
    Ping,
    Hello,
}

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
          sender: mpsc::Sender<RecvMessage>,
        ) -> Result<Connection, PoolError> {
        let socket_addr = address.parse().unwrap();
        let stream = match TcpStream::connect(socket_addr) {
            Err(_) => return Err(UnableToConnect),
            Ok(stream) => stream,
        };
    
        let (conn_sender, conn_receiver) = mpsc::channel();

        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = Arc::clone(&done);
        let mut send_stream = stream.try_clone().unwrap();
        let send_thread = thread::spawn(move || {
            while !send_done.load(std::sync::atomic::Ordering::Relaxed) {
                let message = conn_receiver.recv().unwrap();
                send_stream.write(format!("{:?}\n", &message).as_bytes()).unwrap();
                send_stream.flush().unwrap();
            }      
        });

        //Handles incoming message
        let read_done = Arc::clone(&done);
        let mut recv_stream = stream.try_clone().unwrap();
        recv_stream.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let recv_thread = thread::spawn(move || {
            while !read_done.load(std::sync::atomic::Ordering::Relaxed) {
                // Message handling
                let mut buffer = [0; 512];
                if let Ok(_) = recv_stream.read(&mut buffer) {
                    let ping = b"Ping";
                    if buffer.starts_with(ping) {
                        println!("Got ping");
                        sender.send(RecvMessage::Ping).unwrap();
                        recv_stream.write(b"Pong\n").unwrap();
                        recv_stream.flush().unwrap();
                    }
                };
            }
        });

        Ok(Connection {
            // Need these to handle drop
            socket_addr,
            send_thread: Some(send_thread),
            recv_thread: Some(recv_thread),
            conn_sender,
            done,
        })
    }
}
