pub type PeerResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
pub enum PeerError {
    NoPool,
    FailedToCreateConnection,
    UnableToConnect,
    ConnectionDenied,
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            PeerError::NoPool => "No connection available",
            PeerError::FailedToCreateConnection => "Failed to create connection",
            PeerError::UnableToConnect => "Unable to connect to the peer",
            PeerError::ConnectionDenied => "Connection request was rejected",
        };
        write!(f, "{}", msg)
    }
}

impl std::error::Error for PeerError {}
