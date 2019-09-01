use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::connection::{is_connection_acceptable, Connection};
use super::util::*;
use super::util::ChanMessage::*;

pub type ConnectionPool = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

pub enum PoolMessage {
    Add(SocketAddr),
    Delete(SocketAddr),
}

use super::connection_pool::PoolMessage::*;

pub fn start_pool_manager(
    my_addr: SocketAddr,
    conn_pool: ConnectionPool,
) -> PeerResult<(JoinHandle<()>, MessageSender<PoolMessage>)> {
    let (tx, rx) = channel::<PoolMessage>();
    let tx_c = tx.clone();
    let handle = thread::spawn(move || {
        loop {
            match rx.recv().unwrap() {
                Message(Add(socket_addr)) => {
                    match is_connection_acceptable(&socket_addr, &conn_pool) {
                        None => {
                            if let Ok(conn) = Connection::connect(
                                my_addr,
                                socket_addr,
                                Arc::clone(&conn_pool),
                                tx.clone(),
                            ) {
                                conn_pool.lock().unwrap().insert(conn.address, conn);
                            };
                        },
                        Some(reason) => println!("Connection denied: {:?}", reason),
                    }
                }
                Message(Delete(socket_addr)) => {
                    println!("Removing address: {:?}", &socket_addr);
                    conn_pool.lock().unwrap().remove(&socket_addr);
                }
                Terminate => break,
            }
        }
    });
    Ok((handle, tx_c))
}
