use std::sync::mpsc::{self, Sender, Receiver, SendError, RecvError};

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

///This can let you send `Terminate` on any given channel
pub enum ChanMessage<T> {
    Message(T),
    Terminate,
}

#[derive(Clone)]
pub struct MessageSender<T>(Sender<ChanMessage<T>>);

impl<T> MessageSender<T> 
{
    pub fn send(&self, t: T) -> Result<(), SendError<ChanMessage<T>>> {
        self.0.send(ChanMessage::Message(t))
    }

    pub fn send_terminate(&self) -> Result<(), SendError<ChanMessage<T>>> {
        self.0.send(ChanMessage::Terminate)
    }

    pub fn clone(&self) -> Self {
        MessageSender(self.0.clone())
    }

    pub fn to_owned(&self) -> Self {
        self.clone()
    }
}

pub struct MessageReceiver<T>(Receiver<ChanMessage<T>>);

impl<T> MessageReceiver<T> {
    pub fn recv(&self) -> Result<ChanMessage<T>, RecvError> {
        self.0.recv()
    }
}

///`channel` returns a tuple of sendend and receive end
pub fn channel<T>() -> (MessageSender<T>, MessageReceiver<T>) {
    let (tx, rx) = mpsc::channel();
    (MessageSender(tx), MessageReceiver(rx))
}