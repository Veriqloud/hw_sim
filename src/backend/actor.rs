use std::marker::PhantomData;

use libhardware::ModulatorState;
use snafu::ResultExt;
use tokio::sync::{mpsc, oneshot};

use super::{
    errors::{self, Error, HardwareSnafu},
    BytesGenerator,
};

pub struct Actor<T: BytesGenerator + Clone> {
    receiver: mpsc::Receiver<ActorMessage>,
    simulator: T,
}

impl<T: BytesGenerator + Clone> Actor<T> {
    pub fn new(simulator: T, receiver: mpsc::Receiver<ActorMessage>) -> Self {
        Actor {
            receiver,
            simulator,
        }
    }

    async fn handle_message(&mut self, msg: ActorMessage) {
        match msg {
            ActorMessage::ReadAngles { reply_to } => {
                let keys_results = self.simulator.read_angles().context(HardwareSnafu);

                let _ = reply_to.send({
                    match keys_results {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                });
            }
            ActorMessage::SetModulatorState {
                at_global_counter,
                modulator_state,
                reply_to,
            } => {
                let res = self
                    .simulator
                    .set_modulator_state(modulator_state, at_global_counter)
                    .context(HardwareSnafu);
                let _ = reply_to.send({
                    match res {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                });
            }
            ActorMessage::GetGlobalCounter { reply_to } => {
                let gc = self.simulator.get_global_counter();
                let _ = reply_to.send(gc);
            }
            ActorMessage::GetGcsafe { reply_to } => {
                let gc = self.simulator.get_global_counter();
                let _ = reply_to.send(gc.unwrap_or(0_u64));
            }
        }
    }
}

pub enum ActorMessage {
    SetModulatorState {
        at_global_counter: u64,
        modulator_state: ModulatorState,
        reply_to: oneshot::Sender<Result<u32, Error>>,
    },
    ReadAngles {
        reply_to: oneshot::Sender<Result<Vec<u8>, Error>>,
    },
    GetGlobalCounter {
        reply_to: oneshot::Sender<Option<u64>>,
    },
    GetGcsafe {
        reply_to: oneshot::Sender<u64>,
    },
}

pub async fn run_simulator_actor<T: BytesGenerator + Clone>(mut actor: Actor<T>) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await;
    }
}

#[derive(Clone)]
pub struct ActorHandle<T: BytesGenerator> {
    sender: mpsc::Sender<ActorMessage>,
    _phantom: PhantomData<T>,
}

impl<T: BytesGenerator + Clone> ActorHandle<T> {
    pub fn new(simulator: T) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let actor = Actor::new(simulator, receiver);
        tokio::spawn(run_simulator_actor(actor));

        Self {
            sender,
            _phantom: Default::default(),
        }
    }

    pub async fn set_modulator_state(
        &self,
        at_global_counter: u64,
        modulator_state: ModulatorState,
    ) -> Result<u32, Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::SetModulatorState {
            at_global_counter,
            modulator_state,
            reply_to: send,
        };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn read_angles(&self) -> Result<Vec<u8>, Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::ReadAngles { reply_to: send };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn get_global_counter(&self) -> Result<Option<u64>, Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::GetGlobalCounter { reply_to: send };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)
    }

    pub async fn get_gc_safe(&self) -> Result<u64, Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::GetGcsafe { reply_to: send };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)
    }
}
