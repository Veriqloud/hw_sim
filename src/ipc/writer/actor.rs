use std::marker::PhantomData;

use snafu::ResultExt;
use tokio::sync::{mpsc, oneshot};

use super::{
    errors::{self, Error},
    Writer,
};

pub struct Actor<T: Writer> {
    writer: T,
    receiver: mpsc::Receiver<ActorMessage>,
}
impl<T: Writer> Actor<T> {
    pub fn new(writer: T, receiver: mpsc::Receiver<ActorMessage>) -> Self {
        Actor { receiver, writer }
    }

    async fn handle_message(&mut self, msg: ActorMessage) {
        match msg {
            ActorMessage::InsertData { reply_to, data } => {
                match self.writer.insert_data(data).await {
                    Ok(_) => reply_to.send(Ok(())).unwrap(),
                    Err(e) => reply_to.send(Err(e)).unwrap(),
                }
            }
        }
    }
}

pub enum ActorMessage {
    InsertData {
        data: Vec<u8>,
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
}

pub async fn run_writer_actor<T: Writer>(mut actor: Actor<T>) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await;
    }
}

#[derive(Clone)]
pub struct ActorHandle<T: Writer> {
    sender: mpsc::Sender<ActorMessage>,
    _phantom: PhantomData<T>,
}

impl<T: Writer> ActorHandle<T> {
    pub fn new(writer: T) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let actor = Actor::new(writer, receiver);
        tokio::spawn(run_writer_actor(actor));

        Self {
            sender,
            _phantom: Default::default(),
        }
    }

    pub async fn insert_data(&self, data: Vec<u8>) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();

        let message = ActorMessage::InsertData {
            reply_to: send,
            data,
        };

        // Ignore send errors. If this send fails, so does the
        // recv.await below. There's no reason to check for the
        // same failure twice.
        let _ = self.sender.send(message).await;

        recv.await.context(errors::ActorDiedSnafu)?
    }
}
