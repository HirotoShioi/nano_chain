use serde::{Deserialize, Serialize};
use serde_json;
use std::io::prelude::*;
use std::io::BufReader;
use std::net::{SocketAddr, TcpStream};

use super::util::PeerResult;

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
pub fn send_message(server_address: Option<&SocketAddr>, stream: &TcpStream, message: ProtocolMessage) -> PeerResult<()> {
    match server_address {
        Some(address) =>  println!("Sending message to: {:?},  {:?}", address, message),
        None => println!("Sending message: {:?}", message)
    };
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
    println!("Got message: {:?}", recv_message);
    Ok(recv_message)
}
