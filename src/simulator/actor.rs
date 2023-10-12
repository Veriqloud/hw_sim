use std::marker::PhantomData;

use crate::simulator::Simulator;
use snafu::ResultExt;
use tokio::sync::{mpsc, oneshot};

use super::{
    errors::{self, Error},
    Keys,
};

pub struct Actor<T: Simulator + Clone> {
    receiver: mpsc::Receiver<ActorMessage>,
    simulator: T,
}

impl<T: Simulator + Clone> Actor<T> {
    pub fn new(simulator: T, receiver: mpsc::Receiver<ActorMessage>) -> Self {
        Actor {
            receiver,
            simulator,
        }
    }

    async fn handle_message(&mut self, msg: ActorMessage) {
        match msg {
            ActorMessage::GenerateRawKeys {
                size,
                owner,
                reply_to,
            } => {
                let simulator_cpy = self.simulator.clone();

                tokio::spawn(async move {
                    let keys_results = simulator_cpy.generate_raw_keys(size, owner);

                    let _ = reply_to.send({
                        match keys_results {
                            Ok(v) => Ok(v),
                            Err(e) => Err(e),
                        }
                    });
                });
            }
        }
    }
}

pub enum ActorMessage {
    GenerateRawKeys {
        size: usize,
        owner: String,
        reply_to: oneshot::Sender<Result<Keys, Error>>,
    },
}

pub async fn run_simulator_actor<T: Simulator + Clone>(mut actor: Actor<T>) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await;
    }
}

#[derive(Clone)]
pub struct ActorHandle<T: Simulator> {
    sender: mpsc::Sender<ActorMessage>,
    _phantom: PhantomData<T>,
}

impl<T: Simulator + Clone> ActorHandle<T> {
    pub fn new(simulator: T) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let actor = Actor::new(simulator, receiver);
        tokio::spawn(run_simulator_actor(actor));

        Self {
            sender,
            _phantom: Default::default(),
        }
    }

    pub async fn generate_raw_keys(&self, size: usize, owner: String) -> Result<Keys, Error> {
        let (send, recv) = oneshot::channel();

        let message = ActorMessage::GenerateRawKeys {
            size,
            owner,
            reply_to: send,
        };

        // Ignore send errors. If this send fails, so does the
        // recv.await below. There's no reason to check for the
        // same failure twice.
        let _ = self.sender.send(message).await;

        recv.await.context(errors::ActorDiedSnafu)?
    }
}
