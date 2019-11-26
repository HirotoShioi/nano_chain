use std::sync::mpsc::{self, Iter, Receiver, RecvError, SendError, Sender};

///This can let you send `Terminate` on any given channel that sends data of type <T>
pub enum ChanMessage<T> {
    Message(T),
    Terminate,
}

pub struct MessageSender<T>(Sender<ChanMessage<T>>);

impl<T> MessageSender<T> {
    ///Send message `t` to the channel
    pub fn send(&self, t: T) -> Result<(), SendError<ChanMessage<T>>> {
        self.0.send(ChanMessage::Message(t))
    }

    ///Send `Terminate` message to the channel, you can make it so that receive
    /// end will shutdown its thread upon receiving this message.
    pub fn send_terminate(&self) -> Result<(), SendError<ChanMessage<T>>> {
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
    pub fn recv(&self) -> Result<ChanMessage<T>, RecvError> {
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
