use std::{marker::PhantomData, thread, time};

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
                let mut simulator_cpy = self.simulator.clone();

                tokio::spawn(async move {
                    let gc = simulator_cpy.get_global_counter().unwrap() + 1000;
                    simulator_cpy
                        .set_modulator_state(ModulatorState::Qkd, gc)
                        .unwrap();

                    // sleep and read
                    thread::sleep(time::Duration::from_millis(50));
                    let keys_results = simulator_cpy.read_angles().context(HardwareSnafu);

                    let _ = reply_to.send({
                        match keys_results {
                            Ok(v) => Ok(v),
                            Err(e) => Err(e),
                        }
                    });
                });
            }
            ActorMessage::SetModulatorState {
                at_global_counter: _,
                modulator_state: _,
                reply_to: _,
            } => {
                todo!()
            }
            ActorMessage::GetGlobalCounter { reply_to: _ } => todo!(),
            ActorMessage::GetGcsafe { reply_to: _ } => todo!(),
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

    pub async fn read_angles(&self) -> Result<Vec<u8>, Error> {
        let (send, recv) = oneshot::channel();

        let message = ActorMessage::ReadAngles { reply_to: send };

        // Ignore send errors. If this send fails, so does the
        // recv.await below. There's no reason to check for the
        // same failure twice.
        let _ = self.sender.send(message).await;

        recv.await.context(errors::ActorDiedSnafu)?
    }
}
