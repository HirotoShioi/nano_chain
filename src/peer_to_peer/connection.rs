use std::io::prelude::*;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::connection_pool::{ConnectionPool, PoolMessage};
use super::protocol_message::ProtocolMessage::{self, *};
use super::protocol_message::{read_message, send_message};
use super::util::*;

use super::util::ChanMessage::*;

///Connection handles message handling between peers
///It will start send and recv thread once instantiated.
pub struct Connection {
    // Perhaps add id field?
    pub address: SocketAddr,
    pub stream: TcpStream,
    // Thread handle for send operation
    pub send_thread: Option<JoinHandle<()>>,
    // Thread handle for receive operation
    pub recv_thread: Option<JoinHandle<()>>,
    // Used to send message to the peer
    pub conn_sender: MessageSender<ProtocolMessage>,
    // Used to shutdown connection gracefully
    pub done: Arc<AtomicBool>,
}

impl Connection {
    ///Instantiate `Connection` and insert it into `ConnectionPool`
    pub fn connect_stream(
        socket_addr: SocketAddr,
        stream: TcpStream,
        conn_pool: ConnectionPool,
        conn_sender: MessageSender<PoolMessage>,
    ) -> PeerResult<Connection> {
        //Check if address aleady exists in the pool
        let (tx, rx) = channel();
        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = done.to_owned();
        let mut send_stream = stream.try_clone()?;
        let send_thread = thread::spawn(move || {
            loop {
                // Can you make it such that if any of the function fails,
                // it does the same restore action?
                if let Ok(message) = rx.recv() {
                    match message {
                        Message(msg) => {
                            //If write fails due to broken pipe, close the threads
                            // Make it cleaner
                            if send_message(Some(&socket_addr), &send_stream, msg).is_err() {
                                drop(&send_stream);
                                send_done.store(true, Ordering::Relaxed);
                            };
                            send_stream.flush().expect("Unable to flush stream");
                        }
                        Terminate => break,
                    }
                };
            }
        });

        //Handles incoming message
        let read_done = done.to_owned();
        let mut recv_stream = stream.try_clone()?;
        recv_stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        let recv_thread = thread::spawn(move || {
            while !read_done.load(Ordering::Relaxed) {
                if handle_recv_message(&recv_stream, &conn_pool, conn_sender.clone())
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
            conn_sender: tx,
            done,
        };

        println!("New connection: {:?}", &socket_addr);
        Ok(conn)
    }

    /// Create TCPStream with given SocketAddr
    ///If connection is successful, instantiate `Connection`
    pub fn connect(
        my_address: SocketAddr,
        address: SocketAddr,
        conn_pool: ConnectionPool,
        conn_sender: MessageSender<PoolMessage>,
    ) -> PeerResult<Connection> {
        let stream = TcpStream::connect(address)?;
        send_message(None, &stream, Request(my_address))?;
        if let Ok(Accepted) = read_message(&stream) {
            let conn = Connection::connect_stream(address, stream, conn_pool, conn_sender).unwrap();
            Ok(conn)
        } else {
            Err(Box::new(PeerError::ConnectionDenied))
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.conn_sender
            .send_terminate()
            .expect("Unable to send terminate message");
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
pub fn broadcast(pools: &ConnectionPool, message: ProtocolMessage) -> PeerResult<()> {
    let mut pools = pools.lock().unwrap();
    // If any of the `send` fails, store the `SocketAddr` here so it can later be
    // removed.
    let mut failed_addr = Vec::new();

    for (socket_addr, conn) in pools.iter_mut() {
        if conn.conn_sender.send(message.to_owned()).is_err() {
            failed_addr.push(socket_addr.to_owned());
        };
    }

    //Remove this
    //Use PoolMessage::Delete instead
    for socket_addr in failed_addr.iter() {
        pools.remove(socket_addr);
    }

    Ok(())
}

///Read messages from the `TcpStream` and handle them accordingly
pub fn handle_recv_message(
    stream: &TcpStream,
    pool: &ConnectionPool,
    conn_sender: MessageSender<PoolMessage>,
) -> PeerResult<()> {
    if let Ok(recv_message) = read_message(&stream) {
        match recv_message {
            Ping => send_message(None, &stream, Pong)?,
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
                    conn_sender.send(PoolMessage::Add(socket_addr))?;
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
                send_message(Some(&their_address), stream, msg)?;
            }
            Exiting(socket_addr) => {
                conn_sender.send(PoolMessage::Delete(socket_addr))?;
            }
            _ => {}
        }
    };
    Ok(())
}

//This can be tested!
pub fn is_connection_acceptable(
    socket_addr: &SocketAddr,
    conn_pool: &ConnectionPool,
) -> Option<ProtocolMessage> {
    let conn_pool = conn_pool.lock().unwrap();
    if conn_pool.contains_key(socket_addr) {
        Some(AlreadyConnected)
    //Need to subtract 1 because capacity() return lowerbound (capacity + 1)
    } else if conn_pool.len() >= conn_pool.capacity() - 1 {
        Some(CapacityReached)
    } else {
        None
    }
}
