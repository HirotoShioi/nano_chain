use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::connection::{is_connection_acceptable, Connection};
use super::util::*;

pub type ConnectionPool = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

pub enum PoolMessage {
    Add(SocketAddr),
    Delete(SocketAddr),
    Terminate,
}

#[derive(Debug)]
pub enum PoolError {
    NoPool,
    FailedToCreateConnection,
    UnableToConnect,
    ConnectionDenied,
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            PoolError::NoPool => "No connection available",
            PoolError::FailedToCreateConnection => "Failed to create connection",
            PoolError::UnableToConnect => "Unable to connect to the peer",
            PoolError::ConnectionDenied => "Connection request was rejected",
        };
        write!(f, "{}", msg)
    }
}

impl std::error::Error for PoolError {}

pub fn start_pool_manager(
    my_addr: SocketAddr,
    conn_pool: ConnectionPool,
) -> PeerResult<(Arc<AtomicBool>, JoinHandle<()>, mpsc::Sender<PoolMessage>)> {
    let (tx, rx) = mpsc::channel::<PoolMessage>();
    let is_done = Arc::new(AtomicBool::new(false));
    let is_done_c = Arc::clone(&is_done);
    let tx_c = tx.clone();
    let handle = thread::spawn(move || {
        while !is_done_c.load(Ordering::Relaxed) {
            match rx.recv().unwrap() {
                PoolMessage::Add(socket_addr) => {
                    if is_connection_acceptable(&socket_addr, &conn_pool).is_none() {
                        if let Ok(conn) = Connection::connect(
                            my_addr,
                            socket_addr,
                            Arc::clone(&conn_pool),
                            tx.clone(),
                        ) {
                            conn_pool.lock().unwrap().insert(conn.address, conn);
                        };
                    }
                }
                PoolMessage::Delete(socket_addr) => {
                    println!("Removing address: {:?}", &socket_addr);
                    conn_pool.lock().unwrap().remove(&socket_addr);
                }
                PoolMessage::Terminate => {
                    println!("Terminating");
                }
            }
        }
    });
    Ok((is_done, handle, tx_c))
}
