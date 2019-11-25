use std::sync::mpsc::{self, Iter, Receiver, RecvError, SendError, Sender};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
pub enum PeerError {
    NoPool,
    FailedToCreateConnection,
    UnableToConnect,
    ConnectionDenied,
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let error_message = match self {
            PeerError::NoPool => "No connection available",
            PeerError::FailedToCreateConnection => "Failed to create connection",
            PeerError::UnableToConnect => "Unable to connect to the peer",
            PeerError::ConnectionDenied => "Connection request was rejected",
        };
        write!(f, "{}", error_message)
    }
}

impl std::error::Error for PeerError {}

///This can let you send `Terminate` on any given channel that sends data of type <T>
pub enum ChanMessage<T> {
    Message(T),
    Terminate,
}

pub struct MessageSender<T>(Sender<ChanMessage<T>>);

impl<T> MessageSender<T> {
    ///Send message `t` to the channel
    pub fn send(&self, t: T) -> std::result::Result<(), SendError<ChanMessage<T>>> {
        self.0.send(ChanMessage::Message(t))
    }

    ///Send `Terminate` message to the channel, you can make it so that receive
    /// end will shutdown its thread upon receiving this message.
    pub fn send_terminate(&self) -> std::result::Result<(), SendError<ChanMessage<T>>> {
        self.0.send(ChanMessage::Terminate)
    }

    ///Clone
    pub fn clone(&self) -> Self {
        MessageSender(self.0.clone())
    }

    ///Clone as well!!
    pub fn to_owned(&self) -> Self {
        self.clone()
    }
}

pub struct MessageReceiver<T>(Receiver<ChanMessage<T>>);

impl<T> MessageReceiver<T> {
    pub fn recv(&self) -> std::result::Result<ChanMessage<T>, RecvError> {
        self.0.recv()
    }

    pub fn iter(&self) -> Iter<ChanMessage<T>> {
        self.0.iter()
    }
}

///This will return a tuple of send end and receive end of the channel
pub fn channel<T>() -> (MessageSender<T>, MessageReceiver<T>) {
    let (tx, rx) = mpsc::channel();
    (MessageSender(tx), MessageReceiver(rx))
}
