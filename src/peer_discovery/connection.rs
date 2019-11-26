use log::{info, trace, warn};
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
use super::error::{PeerError, Result};
use super::util::{channel, ChanMessage::*, MessageSender};

///Time out duration for reading message from TCP stream
const READ_TIME_OUT: u64 = 200;

///Connection handles message handling between peers
///It will start send and recv thread once instantiated.
pub struct Connection {
    /// Address we're connecting to
    pub address: SocketAddr,
    /// Stream
    pub stream: TcpStream,
    // Thread handle for send operation
    pub send_thread: Option<JoinHandle<()>>,
    // Thread handle for receive operation
    pub recv_thread: Option<JoinHandle<()>>,
    // Used to send message to the peer
    pub conn_sender: MessageSender<ProtocolMessage>,
    // Used to shutdown `Connection` gracefully
    pub done: Arc<AtomicBool>,
}

/// `Connection` is a handler for each connection we make with other peer
/// It will be responsible for sending & receiving messages between us and the others.
impl Connection {
    ///Instantiate `Connection`
    ///
    /// This will create threads for sending and receiving messages from TcpStream
    pub fn from_stream(
        their_addr: SocketAddr,
        stream: TcpStream,
        conn_pool: ConnectionPool,
        conn_sender: MessageSender<PoolMessage>,
        shared_num: Arc<AtomicU32>,
    ) -> Result<Connection> {
        let (tx, rx) = channel();
        let done = Arc::new(AtomicBool::new(false));

        //Handles sending messages
        let send_done = done.to_owned();
        let mut send_stream = stream.try_clone()?;
        let send_thread = thread::spawn(move || {
            // Can you make it such that if any of the function fails,
            // it does the same restore action?
            while let Some(message) = rx.iter().next() {
                match message {
                    Message(msg) => {
                        //If write fails due to broken pipe, close the threads
                        //(TODO): Make it cleaner
                        if send_message(&their_addr, &send_stream, msg).is_err() {
                            send_stream
                                .shutdown(Shutdown::Both)
                                .expect("Failed to shutdown stream");
                            send_done.store(true, Ordering::Relaxed);
                        };
                        send_stream.flush().expect("Unable to flush stream");
                    }
                    Terminate => break,
                }
            }
        });

        //Handles incoming message
        let read_done = done.to_owned();
        let mut recv_stream = stream.try_clone()?;
        recv_stream.set_read_timeout(Some(Duration::from_millis(READ_TIME_OUT)))?;
        let recv_thread = thread::spawn(move || {
            while !read_done.load(Ordering::Relaxed) {
                match handle_recv_message(
                    their_addr,
                    &recv_stream,
                    &conn_pool,
                    conn_sender.clone(),
                    &shared_num,
                ) {
                    Ok(_) => recv_stream.flush().unwrap(),
                    Err(_) => recv_stream.shutdown(Shutdown::Both).unwrap(),
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

        info!("New connection accepted: {:?}", &their_addr);
        Ok(conn)
    }

    /// Create TCPStream with given SocketAddr
    ///
    ///This will perform a handshake between us and the opponent to reach an
    /// agreement on whether they can talk to one another.
    ///
    /// Agreement will fail if:
    ///
    /// - Opponent sends invalid message. Any message other than `ConnectionAccepted`
    /// will mean we did not reached to an agreement.
    ///
    /// - Opponent cannot be reached (Network error)
    ///
    /// - Opponent does not have the capacity to talk with us
    pub fn connect(
        my_address: SocketAddr,
        their_address: SocketAddr,
        conn_pool: ConnectionPool,
        conn_sender: MessageSender<PoolMessage>,
        shared_num: Arc<AtomicU32>,
    ) -> Result<Connection> {
        info!("Attempting to connect to the node: {:?}", their_address);
        let stream = TcpStream::connect(their_address)?;
        send_message(&their_address, &stream, Request(my_address))?;
        if let Ok(ConnectionAccepted) = read_message(&stream) {
            let conn =
                Connection::from_stream(their_address, stream, conn_pool, conn_sender, shared_num)
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

///Read messages from the `TcpStream` and handle them accordingly
pub fn handle_recv_message(
    their_addr: SocketAddr,
    stream: &TcpStream,
    pool: &ConnectionPool,
    conn_sender: MessageSender<PoolMessage>,
    shared_num: &Arc<AtomicU32>,
) -> Result<()> {
    if let Ok(recv_message) = read_message(&stream) {
        match recv_message {
            Ping => send_message(&their_addr, &stream, Pong)?,
            NewNumber(miner_address, their_num) => {
                info!("New value from {:?}: {:?}", miner_address, their_num);
                let my_num = shared_num.load(Ordering::Relaxed);
                //If their number and your number is same, ignore it
                use std::cmp;

                match my_num.cmp(&their_num) {
                    cmp::Ordering::Less => {
                        info!(
                            "Their number({:?}) is bigger than ours({:?}), accepting",
                            their_num, my_num
                        );
                        shared_num.store(their_num, Ordering::Relaxed);

                        let pool = pool.lock().unwrap();
                        if let Some(conn) = pool.get(&miner_address) {
                            info!(
                                "Notifying miner {:?} that the number has been accepted",
                                miner_address
                            );
                            conn.conn_sender.send(NumAccepted(their_num)).unwrap();
                        }
                        // This will prevent double send
                        if miner_address != their_addr {
                            info!("Replying to {:?} that number was accepted", their_addr);
                            send_message(&their_addr, &stream, NumAccepted(their_num))?;
                        }
                        for conn in pool.values() {
                            if miner_address != conn.address || their_addr != conn.address {
                                conn.conn_sender.send(NewNumber(miner_address, their_num))?;
                            }
                        }
                    }
                    cmp::Ordering::Greater => {
                        info!("Our value is bigger({:?}), responsing", my_num);
                        send_message(&their_addr, &stream, NumDenied(my_num))?;
                    }
                    cmp::Ordering::Equal => {}
                }
            }
            NumAccepted(my_num) => {
                info!("Peer ({}) accepted number: {:?}", their_addr, my_num);
                shared_num.store(my_num, Ordering::Relaxed);
            }
            NumDenied(their_num) => {
                let my_num = shared_num.load(Ordering::Relaxed);
                if my_num < their_num {
                    info!(
                        "Their number({:?}) is bigger than ours({:?})",
                        their_num, my_num
                    );
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
                let mut their_known_address = their_known_address;
                their_known_address.push(their_address);
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
            Exiting => {
                warn!("Node {:?} is shutting down", their_addr);
                conn_sender.send(PoolMessage::Delete(their_addr))?;
            }
            _ => {}
        }
    };
    Ok(())
}

///This will determine whether we can connect to the given `SocketAddr`
///
/// If not, this will return `Some` with the reason.
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

/// List of message that are being used within the procotol
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
    NewNumber(SocketAddr, u32),
    NumDenied(u32),
    NumAccepted(u32),
    /// Exchange information about peers
    AskPeer(SocketAddr, Vec<SocketAddr>, usize), //usize should be how connection we want
    ReplyPeer(Vec<SocketAddr>, usize),
    Exiting,
}

use super::connection::ProtocolMessage::*;

// Using these functions will allow us to have strongly typed communication with
// one another.
// (TODO)Make these functions generic

///Send `ProtocolMessage` the given TcpStream
pub fn send_message(
    //Address you're sending to
    their_addr: &SocketAddr,
    stream: &TcpStream,
    message: ProtocolMessage,
) -> Result<()> {
    trace!("Sending message to: {:?},  {:?}", their_addr, message);
    let mut stream_clone = stream.try_clone()?;
    serde_json::to_writer(stream, &message)?;
    stream_clone.write_all(b"\n")?;
    stream_clone.flush()?;
    Ok(())
}

///Read `ProtocolMessage` from given TcpStream
pub fn read_message(stream: &TcpStream) -> Result<ProtocolMessage> {
    let mut reader = BufReader::new(stream);
    let mut buffer = String::new();
    reader.read_line(&mut buffer)?;
    let recv_message: ProtocolMessage = serde_json::from_str(&buffer)?; // Should throw error if unable to parse
    trace!("Got message: {:?}", recv_message);
    Ok(recv_message)
}

/// Broadcast given `message` to the known peers
pub fn broadcast(pools: &ConnectionPool, message: ProtocolMessage) -> Result<()> {
    for (_socket_addr, conn) in pools.lock().unwrap().iter() {
        conn.conn_sender.send(message.to_owned()).unwrap();
    }

    Ok(())
}
