use log::warn;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::connection::{is_connection_acceptable, Connection};
use super::error::Result;
use super::util::{channel, ChanMessage::*, MessageSender};

pub type ConnectionPool = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

pub enum PoolMessage {
    Add(SocketAddr),
    AddConn(SocketAddr, Connection),
    Delete(SocketAddr),
}

use super::connection_pool::PoolMessage::*;

/// Pool manager is responsible for adding/deleting `Connection` from the connection pool.
///
/// Other threads can send messages to pool manager via `Sender<PoolMessage>`
pub fn start_pool_manager(
    my_addr: SocketAddr,
    conn_pool: ConnectionPool,
    shared_num: Arc<AtomicU32>,
) -> Result<(JoinHandle<()>, MessageSender<PoolMessage>)> {
    let (tx, rx) = channel::<PoolMessage>();
    let tx_c = tx.clone();
    let handle = thread::spawn(move || {
        while let Some(message) = rx.iter().next() {
            match message {
                Message(AddConn(socket_addr, conn)) => {
                    conn_pool.lock().unwrap().entry(socket_addr).or_insert(conn);
                }
                Message(Add(socket_addr)) => {
                    match is_connection_acceptable(&socket_addr, &conn_pool) {
                        None => {
                            if let Ok(conn) = Connection::connect(
                                my_addr,
                                socket_addr,
                                conn_pool.to_owned(),
                                tx.clone(),
                                shared_num.to_owned(),
                            ) {
                                conn_pool
                                    .lock()
                                    .unwrap()
                                    .entry(conn.address)
                                    .or_insert(conn);
                            };
                        }
                        Some(reason) => warn!(
                            "Connection denied on {:?}, reason: {:?}",
                            socket_addr, reason
                        ),
                    }
                }
                Message(Delete(socket_addr)) => {
                    warn!("Removing address: {:?}", &socket_addr);
                    conn_pool.lock().unwrap().remove(&socket_addr);
                }
                Terminate => break,
            }
        }
    });
    Ok((handle, tx_c))
}
