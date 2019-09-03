use serde::{Deserialize, Serialize};
use serde_json;
use std::io::prelude::*;
use std::io::BufReader;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::connection_pool::{ConnectionPool, PoolMessage};
use super::util::ChanMessage::*;
use super::util::*;

use log::{info, trace, warn};

const READ_TIME_OUT: u64 = 200;

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
        their_addr: SocketAddr,
        stream: TcpStream,
        conn_pool: ConnectionPool,
        conn_sender: MessageSender<PoolMessage>,
        shared_num: Arc<AtomicU32>,
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
                            if send_message(&their_addr, &send_stream, msg).is_err() {
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
        recv_stream.set_read_timeout(Some(Duration::from_millis(READ_TIME_OUT)))?;
        let recv_thread = thread::spawn(move || {
            while !read_done.load(Ordering::Relaxed) {
                if handle_recv_message(
                    their_addr,
                    &recv_stream,
                    &conn_pool,
                    conn_sender.clone(),
                    &shared_num,
                )
                .is_ok()
                {
                    recv_stream.flush().unwrap();
                }
            }
        });

        let conn = Connection {
            address: their_addr,
            stream,
            send_thread: Some(send_thread),
            recv_thread: Some(recv_thread),
            conn_sender: tx,
            done,
        };

        println!("New connection: {:?}", &their_addr);
        Ok(conn)
    }

    /// Create TCPStream with given SocketAddr
    ///If connection is successful, instantiate `Connection`
    pub fn connect(
        my_address: SocketAddr,
        their_address: SocketAddr,
        conn_pool: ConnectionPool,
        conn_sender: MessageSender<PoolMessage>,
        shared_num: Arc<AtomicU32>,
    ) -> PeerResult<Connection> {
        let stream = TcpStream::connect(their_address)?;
        send_message(&their_address, &stream, Request(my_address))?;
        if let Ok(ConnectionAccepted) = read_message(&stream) {
            let conn = Connection::connect_stream(
                their_address,
                stream,
                conn_pool,
                conn_sender,
                shared_num,
            )
            .unwrap();
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
    their_addr: SocketAddr,
    stream: &TcpStream,
    pool: &ConnectionPool,
    conn_sender: MessageSender<PoolMessage>,
    shared_num: &Arc<AtomicU32>,
) -> PeerResult<()> {
    if let Ok(recv_message) = read_message(&stream) {
        match recv_message {
            Ping => send_message(&their_addr, &stream, Pong)?,
            NewNumber(their_address, their_num) => {
                let my_num = shared_num.load(Ordering::Relaxed);
                //If their number and your number is same, ignore it
                if my_num < their_num {
                    println!("Their number is bigger than ours, accepting: {}", their_num);
                    shared_num.store(their_num, Ordering::Relaxed);

                    send_message(&their_addr, &stream, NumAccepted(their_num))
                        .expect("Unable to send accept message");
                    for conn in pool.lock().unwrap().values() {
                        if their_address != conn.address {
                            conn.conn_sender
                                .send(NewNumber(their_address, their_num))
                                .expect("Failed to send message");
                        }
                    }
                } else if my_num > their_num {
                    send_message(&their_addr, &stream, NumDenied(my_num))
                        .expect("Unable to reply message");
                }
            }
            NumAccepted(my_num) => {
                shared_num.store(my_num, Ordering::Relaxed);
            }
            NumDenied(their_num) => {
                let my_num = shared_num.load(Ordering::Relaxed);
                if my_num < their_num {
                    println!("Their number is bigger than ours, accepting: {}", their_num);
                    shared_num.store(their_num, Ordering::Relaxed);
                }
                //Should we broadcast it..?
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
                send_message(&their_address, stream, msg)?;
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
    } else if conn_pool.len() > conn_pool.capacity() - 1 {
        Some(CapacityReached)
    } else {
        None
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ProtocolMessage {
    Ping,
    Pong,
    // Handshake
    Request(SocketAddr),
    ConnectionAccepted,
    ConnectionDenied,
    CapacityReached,
    AlreadyConnected,
    // Sharing states
    //Broadcast new number when minted
    NewNumber(SocketAddr, u32),
    //Reply when you get when network has bigger number
    NumDenied(u32),
    NumAccepted(u32),
    /// Exchange information about peers
    AskPeer(SocketAddr, Vec<SocketAddr>, usize), //usize should be how connection we want
    ReplyPeer(Vec<SocketAddr>, usize),
    Exiting(SocketAddr),
}

use super::connection::ProtocolMessage::*;

// Want to make these functions generic
///Send `ProtocolMessage` the given stream
pub fn send_message(
    //Address you're sending to
    their_addr: &SocketAddr,
    stream: &TcpStream,
    message: ProtocolMessage,
) -> PeerResult<()> {
    trace!("Sending message to: {:?},  {:?}", their_addr, message);
    let mut stream_clone = stream.try_clone()?;
    serde_json::to_writer(stream, &message)?;
    stream_clone.write_all(b"\n")?;
    stream_clone.flush()?;
    Ok(())
}

///Send `ReadMessage` from given stream
pub fn read_message(stream: &TcpStream) -> PeerResult<ProtocolMessage> {
    let mut reader = BufReader::new(stream);
    let mut buffer = String::new();
    reader.read_line(&mut buffer)?;
    let recv_message: ProtocolMessage = serde_json::from_str(&buffer)?;
    trace!("Got message: {:?}", recv_message);
    Ok(recv_message)
}
