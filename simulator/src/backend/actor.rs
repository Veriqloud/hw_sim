use std::sync::mpsc::{self, Sender};

use sim_lib::{errors::SimulationError, simulation::Simulator};

use super::errors::{self, Error};

pub struct Actor {
    receiver: mpsc::Receiver<ActorMessage>,
    simulator: Simulator,
    pending_angles: Option<Vec<u8>>,
}

impl Actor {
    pub fn new(simulator: Simulator, receiver: mpsc::Receiver<ActorMessage>) -> Self {
        Actor {
            receiver,
            simulator,
            pending_angles: None,
        }
    }

    fn handle_message(&mut self, msg: ActorMessage) -> Result<(), SimulationError> {
        match msg {
            ActorMessage::StartSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StartSession");
                let result = self.simulator.initialize_session();
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::StopSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StopSession");
                let result = self.simulator.setup_session_end();
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::GenerateGcrAndAnglesBatch { reply_to } => {
                tracing::debug!("SimulatorActor: Processing GenerateGcrAndAnglesBatch");
                let result = self.simulator.generate_gcr_and_angles_batch().map(|(gcr, angles)| {
                    self.pending_angles = Some(angles);
                    gcr
                });
                let _ = reply_to.send(result);
                Ok(())
            }
            ActorMessage::RetrievePendingAnglesBatch {
                received_gcs: _,
                reply_to,
            } => {
                tracing::debug!("SimulatorActor: Processing RetrievePendingAnglesBatch");
                let result = self.pending_angles.take().ok_or_else(|| SimulationError::HardwareError {
                    source: sim_lib::errors::HardwareError::Other {
                        reason: "No pending angles batch to retrieve.".to_string(),
                    },
                });
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
            ActorMessage::StartAttack { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StartAttack");
                self.simulator.start_attack();
                let _ = reply_to.send(Ok(()));
                Ok(())
            }
            ActorMessage::StopAttack { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StopAttack");
                self.simulator.stop_attack();
                let _ = reply_to.send(Ok(()));
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
    StartAttack {
        reply_to: Sender<Result<(), SimulationError>>,
    },
    StopAttack {
        reply_to: Sender<Result<(), SimulationError>>,
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

    pub fn start_attack(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::StartAttack { reply_to: send };
        self.sender
            .send(message)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        match recv.recv() {
            Ok(v) => match v {
                Ok(_) => Ok(()),
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => Err(Error::ActorDied { source: e }),
        }
    }

    pub fn stop_attack(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        let message = ActorMessage::StopAttack { reply_to: send };
        self.sender
            .send(message)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        match recv.recv() {
            Ok(v) => match v {
                Ok(_) => Ok(()),
                Err(e) => Err(Error::Simulation { source: e }),
            },
            Err(e) => Err(Error::ActorDied { source: e }),
        }
    }
}
