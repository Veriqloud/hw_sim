use std::{fs::OpenOptions, io::Write, marker::PhantomData};

use snafu::ResultExt;
use tokio::sync::{mpsc, oneshot};

use crate::ANGLE_PATH;

use super::{
    errors::{self, Error, HardwareSnafu, IoSnafu, SerdeJsonSnafu},
    Angles, BytesGenerator,
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

    async fn handle_message(&mut self, msg: ActorMessage) -> Result<(), Error> {
        match msg {
            ActorMessage::ReadAngles { reply_to } => {
                let keys_results = self.simulator.read_angles().await.context(HardwareSnafu);

                let _ = reply_to.send({
                    match keys_results {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                });
                Ok(())
            }
            ActorMessage::GetGlobalCounter { reply_to } => {
                let gc = self.simulator.get_global_counter();
                let _ = reply_to.send(gc);
                Ok(())
            }
            ActorMessage::SetRole {
                nb_parties,
                position,
                reply_to,
            } => {
                self.simulator.set_role(nb_parties, position).unwrap();
                let _ = reply_to.send(Ok(()));
                Ok(())
            }
            ActorMessage::StartAtGc {
                global_counter,
                reply_to,
            } => {
                let _ = reply_to.send({
                    match self
                        .simulator
                        .start_at_gc(global_counter)
                        .context(HardwareSnafu)
                    {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                });
                Ok(())
            }
            ActorMessage::FifoIdle { reply_to } => {
                let _ = reply_to.send({
                    match self.simulator.fifo_idle().context(HardwareSnafu) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                });
                Ok(())
            }
            ActorMessage::SetAngles { angles, reply_to } => {
                let _ = reply_to.send({
                    match self.simulator.set_angles(angles).context(HardwareSnafu) {
                        Ok(v) => {
                            let mut f = OpenOptions::new()
                                .write(true)
                                .open(ANGLE_PATH)
                                .context(IoSnafu)?;
                            let angles = Angles {
                                angles: angles.to_vec(),
                            };
                            let angles_str =
                                serde_json::to_string(&angles).context(SerdeJsonSnafu)?;
                            // f.write_fmt(&angles_str.into()).context(IoSnafu)?;
                            f.write_all(&angles_str.into_bytes()).context(IoSnafu)?;
                            Ok(v)
                        }
                        Err(e) => Err(e),
                    }
                });
                Ok(())
            }
        }
    }
}

pub enum ActorMessage {
    StartAtGc {
        global_counter: u64,
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    FifoIdle {
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    SetAngles {
        angles: [u8; 8],
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    ReadAngles {
        reply_to: oneshot::Sender<Result<[u8; 1024], Error>>,
    },
    GetGlobalCounter {
        reply_to: oneshot::Sender<Option<u64>>,
    },
    SetRole {
        nb_parties: u32,
        position: u32,
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
}

pub async fn run_simulator_actor<T: BytesGenerator + Clone>(mut actor: Actor<T>) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await.unwrap();
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

    pub async fn fifo_idle(&self) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::FifoIdle { reply_to: send };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn start_at_gc(&self, gc: u64) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::StartAtGc {
            global_counter: gc,
            reply_to: send,
        };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn read_angles(&self) -> Result<[u8; 1024], Error> {
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

    pub async fn set_angles(&self, angles: [u8; 8]) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::SetAngles {
            angles,
            reply_to: send,
        };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn set_role(&self, nb_parties: u32, position: u32) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::SetRole {
            nb_parties,
            position,
            reply_to: send,
        };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)?
    }
}
