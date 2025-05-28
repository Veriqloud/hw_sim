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
            ActorMessage::StartSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StartSession");
                let result = self.simulator.start_session().context(HardwareSnafu);
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::StopSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StopSession");
                let result = self.simulator.stop_session().context(HardwareSnafu);
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::GenerateGcrAndAnglesBatch { reply_to } => {
                tracing::debug!("SimulatorActor: Processing GenerateGcrAndAnglesBatch");
                let result = self
                    .simulator
                    .generate_gcr_and_angles_batch()
                    .await // This is async
                    .context(HardwareSnafu); // Assuming HardwareSnafu is appropriate
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::RetrievePendingAnglesBatch {
                received_gcs,
                reply_to,
            } => {
                tracing::debug!("SimulatorActor: Processing RetrievePendingAnglesBatch");
                let result = self
                    .simulator
                    .retrieve_pending_angles_batch(received_gcs)
                    .context(HardwareSnafu); // Assuming HardwareSnafu is appropriate
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::SetAngles { angles, reply_to } => {
                tracing::debug!("SimulatorActor: Processing SetAngles");
                let result = self.simulator.set_angles(angles).context(HardwareSnafu);
                let _ = reply_to.send(result);
                Ok(())
            }
            // ActorMessage::SetRole was removed
        }
    }
}

// #[derive(Debug)] // oneshot::Sender is not Debug, consider removing if not strictly needed or use a wrapper
pub enum ActorMessage {
    StartSession {
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    StopSession {
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    GenerateGcrAndAnglesBatch {
        reply_to: oneshot::Sender<Result<Vec<[u8; 8]>, Error>>, // Returns GCR data
    },
    RetrievePendingAnglesBatch {
        received_gcs: Vec<u64>,
        reply_to: oneshot::Sender<Result<Vec<u8>, Error>>, // Returns Angles data
    },
    SetAngles { // For configuring bases
        angles: [u8; 4],
        reply_to: oneshot::Sender<Result<(), Error>>,
    },
    // SetRole was removed
    // Old messages like ReadAngles, GetGlobalCounter, SeedAndStartGeneration, Start, Stop might be obsolete
    // depending on whether the VqSim trait still needs them directly or if all interaction is through new messages.
    // For now, keeping them if they are still part of VqSim trait used by other parts,
    // but the primary flow uses the new messages.
    // Based on the VqSim changes, the old messages are indeed obsolete for the new flow.
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

    pub async fn start_session(&self) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::StartSession { reply_to: send };
        self.sender
            .send(message)
            .await
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn stop_session(&self) -> Result<(), Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::StopSession { reply_to: send };
        self.sender
            .send(message)
            .await
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn generate_gcr_and_angles_batch(&self) -> Result<Vec<[u8; 8]>, Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::GenerateGcrAndAnglesBatch { reply_to: send };
        self.sender
            .send(message)
            .await
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        recv.await.context(errors::ActorDiedSnafu)?
    }

    pub async fn retrieve_pending_angles_batch(
        &self,
        received_gcs: Vec<u64>,
    ) -> Result<Vec<u8>, Error> {
        let (send, recv) = oneshot::channel();
        let message = ActorMessage::RetrievePendingAnglesBatch {
            received_gcs,
            reply_to: send,
        };
        self.sender
            .send(message)
            .await
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        recv.await.context(errors::ActorDiedSnafu)?
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

    // pub async fn set_role(&self, nb_parties: u32, position: u32) -> Result<(), Error> { // Removed
    //     let (send, recv) = oneshot::channel(); // Removed
    //     let message = ActorMessage::SetRole { // Removed
    //         nb_parties, // Removed
    //         position, // Removed
    //         reply_to: send, // Removed
    //     }; // Removed
    //     let _ = self.sender.send(message).await; // Removed
    //     recv.await.context(errors::ActorDiedSnafu)? // Removed
    // } // Removed
}
