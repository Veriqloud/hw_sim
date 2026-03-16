use std::sync::mpsc::{self, Sender};

use sim_lib::{errors::SimulationError, simulation::Simulator};
use snafu::{AsErrorSource, ResultExt};

use super::errors::{self, Error};

pub struct Actor {
    receiver: mpsc::Receiver<ActorMessage>,
    simulator: Simulator,
}

impl Actor {
    pub fn new(simulator: Simulator, receiver: mpsc::Receiver<ActorMessage>) -> Self {
        Actor {
            receiver,
            simulator,
        }
    }

    fn handle_message(&mut self, msg: ActorMessage) -> Result<(), SimulationError> {
        match msg {
            ActorMessage::StartSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StartSession");
                let result = self.simulator.start_session();
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::StopSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StopSession");
                let result = self.simulator.stop_session();
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::GenerateGcrAndAnglesBatch { reply_to } => {
                tracing::debug!("SimulatorActor: Processing GenerateGcrAndAnglesBatch");
                let result = self.simulator.generate_gcr_and_angles_batch();
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::RetrievePendingAnglesBatch {
                received_gcs,
                reply_to,
            } => {
                tracing::debug!("SimulatorActor: Processing RetrievePendingAnglesBatch");
                let result = self.simulator.retrieve_pending_angles_batch(received_gcs);
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::SetAngles { angles, reply_to } => {
                tracing::debug!("SimulatorActor: Processing SetAngles");
                let result = self.simulator.set_angles(angles);
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::GenerateAnglesForGcs {
                received_gcs,
                reply_to,
            } => {
                tracing::debug!("SimulatorActor: Processing GenerateAnglesForGcs");
                let result = self.simulator.generate_angles_for_gcs(received_gcs);
                let _ = reply_to.send(result);
                Ok(())
            }
        }
    }
}

// #[derive(Debug)]
pub enum ActorMessage {
    StartSession {
        reply_to: Sender<Result<(), SimulationError>>,
    },
    StopSession {
        reply_to: Sender<Result<(), SimulationError>>,
    },
    GenerateGcrAndAnglesBatch {
        reply_to: Sender<Result<Vec<[u8; 8]>, SimulationError>>, // Returns GCR data
    },
    RetrievePendingAnglesBatch {
        received_gcs: Vec<u64>,
        reply_to: Sender<Result<Vec<u8>, SimulationError>>, // Returns Angles data
    },
    SetAngles {
        // For configuring bases
        angles: [u8; 4],
        reply_to: Sender<Result<(), SimulationError>>,
    },
    GenerateAnglesForGcs {
        received_gcs: Vec<u64>,
        reply_to: Sender<Result<Vec<u8>, SimulationError>>, // Returns Angles data
    },
    // SetRole was removed
    // Old messages like ReadAngles, GetGlobalCounter, SeedAndStartGeneration, Start, Stop might be obsolete
    // depending on whether the VqSim trait still needs them directly or if all interaction is through new messages.
    // For now, keeping them if they are still part of VqSim trait used by other parts,
    // but the primary flow uses the new messages.
    // Based on the VqSim changes, the old messages are indeed obsolete for the new flow.
}

pub fn run_simulator_actor(mut actor: Actor) {
    while let Ok(msg) = actor.receiver.recv() {
        actor.handle_message(msg).unwrap();
    }
}

#[derive(Clone)]
pub struct ActorHandle {
    sender: mpsc::Sender<ActorMessage>,
}

impl ActorHandle {
    pub fn new(simulator: Simulator) -> Self {
        let (sender, receiver) = mpsc::channel();
        let actor = Actor::new(simulator, receiver);
        std::thread::spawn(move || run_simulator_actor(actor));

        Self { sender }
    }

    pub fn start_session(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::StartSession { reply_to: send };
        self.sender
            .send(message)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;

        match recv.recv() {
            Ok(v) => match v {
                Ok(_) => {
                    // Nothing to do
                    Ok(())
                }
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => {
                return Err(Error::ActorDied { source: e });
            }
        }
    }

    pub fn stop_session(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::StopSession { reply_to: send };
        self.sender
            .send(message)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        match recv.recv() {
            Ok(v) => match v {
                Ok(_) => {
                    // Nothing to do
                    Ok(())
                }
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => {
                return Err(Error::ActorDied { source: e });
            }
        }
    }

    pub fn generate_gcr_and_angles_batch(&self) -> Result<Vec<[u8; 8]>, Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::GenerateGcrAndAnglesBatch { reply_to: send };
        self.sender
            .send(message)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        match recv.recv() {
            Ok(v) => match v {
                Ok(b) => {
                    // Nothing to do
                    Ok(b)
                }
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => {
                return Err(Error::ActorDied { source: e });
            }
        }
    }

    pub fn retrieve_pending_angles_batch(&self, received_gcs: Vec<u64>) -> Result<Vec<u8>, Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::RetrievePendingAnglesBatch {
            received_gcs,
            reply_to: send,
        };
        self.sender
            .send(message)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        match recv.recv() {
            Ok(v) => match v {
                Ok(b) => {
                    // Nothing to do
                    Ok(b)
                }
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => {
                return Err(Error::ActorDied { source: e });
            }
        }
    }

    pub fn set_angles(&self, angles: [u8; 4]) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::SetAngles {
            angles,
            reply_to: send,
        };
        let _ = self.sender.send(message);
        match recv.recv() {
            Ok(v) => match v {
                Ok(_) => {
                    // Nothing to do
                    Ok(())
                }
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => {
                return Err(Error::ActorDied { source: e });
            }
        }
    }

    pub fn generate_angles_for_gcs(&self, received_gcs: Vec<u64>) -> Result<Vec<u8>, Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::GenerateAnglesForGcs {
            received_gcs,
            reply_to: send,
        };
        self.sender
            .send(message)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        match recv.recv() {
            Ok(v) => match v {
                Ok(b) => {
                    // Nothing to do
                    Ok(b)
                }
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => {
                return Err(Error::ActorDied { source: e });
            }
        }
    }
}
