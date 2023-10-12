use std::marker::PhantomData;

use snafu::ResultExt;
use tokio::sync::{mpsc, oneshot};

use super::{
    errors::{self, Error},
    Insertor, Keys,
};

pub struct Actor<T: Insertor> {
    insertor: T,
    receiver: mpsc::Receiver<ActorMessage>,
}
impl<T: Insertor> Actor<T> {
    pub fn new(insertor: T, receiver: mpsc::Receiver<ActorMessage>) -> Self {
        Actor { receiver, insertor }
    }

    async fn handle_message(&mut self, msg: ActorMessage) {
        match msg {
            ActorMessage::InsertKeys { reply_to, keys } => {
                match self.insertor.insert_keys(keys).await {
                    Ok(_) => reply_to.send(Ok(())).unwrap(),
                    Err(e) => reply_to.send(Err(e)).unwrap(),
                }
            }
        }
    }
}

pub enum ActorMessage {
    InsertKeys {
        keys: Keys,
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
}

pub async fn run_insertor_actor<T: Insertor>(mut actor: Actor<T>) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await;
    }
}

#[derive(Clone)]
pub struct ActorHandle<T: Insertor> {
    sender: mpsc::Sender<ActorMessage>,
    _phantom: PhantomData<T>,
}

impl<T: Insertor> ActorHandle<T> {
    pub fn new(insertor: T) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let actor = Actor::new(insertor, receiver);
        tokio::spawn(run_insertor_actor(actor));

        Self {
            sender,
            _phantom: Default::default(),
        }
    }

    pub async fn insert_keys(&self, keys: Keys) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();

        let message = ActorMessage::InsertKeys {
            keys,
            reply_to: send,
        };

        // Ignore send errors. If this send fails, so does the
        // recv.await below. There's no reason to check for the
        // same failure twice.
        let _ = self.sender.send(message).await;

        recv.await.context(errors::ActorDiedSnafu)?
    }
}
