use snafu::ResultExt;
use tokio::sync::{mpsc, oneshot};

use super::{
    errors::{self, Error, HardwareSnafu},
    BytesGenerator,
};

pub struct Actor<T: BytesGenerator> {
    receiver: mpsc::Receiver<ActorMessage>,
    simulator: T,
}

impl<T: BytesGenerator> Actor<T> {
    pub fn new(simulator: T, receiver: mpsc::Receiver<ActorMessage>) -> Self {
        Actor {
            receiver,
            simulator,
        }
    }

    async fn handle_message(&mut self, msg: ActorMessage) -> Result<(), Error> {
        match msg {
            ActorMessage::ReadAngles { reply_to } => {
                tracing::debug!("Processing a ReadAngle");
                let keys_results = self.simulator.read_angles().await.context(HardwareSnafu);

                tracing::debug!("Processing a ReadAngle : {}", &keys_results.is_ok());
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
                self.simulator
                    .set_role(nb_parties, position)
                    .context(HardwareSnafu)?;
                let _ = reply_to.send(Ok(()));
                Ok(())
            }
            ActorMessage::StartAtGc {
                global_counter,
                reply_to,
            } => {
                let _ = reply_to.send(
                    self.simulator
                        .start_at_gc(global_counter)
                        .context(HardwareSnafu),
                );
                Ok(())
            }
            ActorMessage::SetAngles { angles, reply_to } => {
                let _ = reply_to.send(self.simulator.set_angles(angles).context(HardwareSnafu));
                Ok(())
            }
            ActorMessage::Start { reply_to } => {
                let _ = reply_to.send(self.simulator.start().context(HardwareSnafu));
                Ok(())
            }
            ActorMessage::Stop { reply_to } => {
                tracing::debug!("Processing a STOP");
                let result = self.simulator.stop().context(HardwareSnafu);

                tracing::debug!("Processing a stop : {}", &result.is_ok());
                let _ = reply_to
                    .send(self.simulator.stop().context(HardwareSnafu))
                    .unwrap();
                Ok(())
            }
        }
    }
}

pub enum ActorMessage {
    Start {
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    Stop {
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    StartAtGc {
        global_counter: u64,
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    SetAngles {
        angles: [u8; 4],
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

pub async fn run_simulator_actor<T: BytesGenerator>(mut actor: Actor<T>) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await.unwrap();
    }
}

#[derive(Clone)]
pub struct ActorHandle {
    sender: mpsc::Sender<ActorMessage>,
}

impl ActorHandle {
    pub fn new<T: BytesGenerator>(simulator: T) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let actor = Actor::new(simulator, receiver);
        tokio::spawn(run_simulator_actor(actor));

        Self { sender }
    }

    pub async fn start(&self) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::Start { reply_to: send };
        let _ = self.sender.send(message).await;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn stop(&self) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::Stop { reply_to: send };
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

    pub async fn set_angles(&self, angles: [u8; 4]) -> Result<(), Error> {
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
