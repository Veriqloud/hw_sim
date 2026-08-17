use std::sync::mpsc::{self, Sender};

use sim_lib::{
    errors::SimulationError,
    simulation::{batches::QkdBatch, GenerationProgress, Simulator},
};

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

    fn handle_message(&mut self, msg: ActorMessage) {
        match msg {
            ActorMessage::StartSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StartSession");
                if let Err(e) = reply_to.send(self.simulator.initialize_session()) {
                    tracing::error!("SimulatorActor: Failed to send StartSession reply: {}", e);
                }
            }
            ActorMessage::StopSession { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StopSession");
                if let Err(e) = reply_to.send(self.simulator.setup_session_end()) {
                    tracing::error!("SimulatorActor: Failed to send StopSession reply: {}", e);
                }
            }
            ActorMessage::GenerateQkdBatch { reply_to } => {
                tracing::debug!("SimulatorActor: Processing GenerateQkdBatch");
                if let Err(e) = reply_to.send(self.simulator.generate_batch()) {
                    tracing::error!("SimulatorActor: Failed to send GenerateQkdBatch reply: {}", e);
                }
            }
            ActorMessage::GenerationProgress { reply_to } => {
                if let Err(e) = reply_to.send(self.simulator.generation_progress()) {
                    tracing::error!(
                        "SimulatorActor: Failed to send GenerationProgress reply: {}",
                        e
                    );
                }
            }
            ActorMessage::DiscardBatches { count, reply_to } => {
                tracing::debug!("SimulatorActor: Discarding {} batches", count);
                if let Err(e) = reply_to.send(self.simulator.discard_batches(count)) {
                    tracing::error!("SimulatorActor: Failed to send DiscardBatches reply: {}", e);
                }
            }
            ActorMessage::SetAngles { angles, reply_to } => {
                tracing::debug!("SimulatorActor: Processing SetAngles");
                if let Err(e) = reply_to.send(self.simulator.set_angles(angles)) {
                    tracing::error!("SimulatorActor: Failed to send SetAngles reply: {}", e);
                }
            }
            ActorMessage::StartAttack { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StartAttack");
                self.simulator.start_attack();
                if let Err(e) = reply_to.send(Ok(())) {
                    tracing::error!("SimulatorActor: Failed to send StartAttack reply: {}", e);
                }
            }
            ActorMessage::StopAttack { reply_to } => {
                tracing::debug!("SimulatorActor: Processing StopAttack");
                self.simulator.stop_attack();
                if let Err(e) = reply_to.send(Ok(())) {
                    tracing::error!("SimulatorActor: Failed to send StopAttack reply: {}", e);
                }
            }
        }
    }
}

pub enum ActorMessage {
    StartSession {
        reply_to: Sender<Result<(), SimulationError>>,
    },
    StopSession {
        reply_to: Sender<Result<(), SimulationError>>,
    },
    GenerateQkdBatch {
        reply_to: Sender<Result<QkdBatch, SimulationError>>,
    },
    GenerationProgress {
        reply_to: Sender<GenerationProgress>,
    },
    DiscardBatches {
        count: u64,
        reply_to: Sender<Result<(), SimulationError>>,
    },
    SetAngles {
        angles: [u8; 4],
        reply_to: Sender<Result<(), SimulationError>>,
    },
    StartAttack {
        reply_to: Sender<Result<(), SimulationError>>,
    },
    StopAttack {
        reply_to: Sender<Result<(), SimulationError>>,
    },
}

pub fn run_simulator_actor(mut actor: Actor) {
    while let Ok(msg) = actor.receiver.recv() {
        actor.handle_message(msg);
    }
}

#[derive(Clone)]
pub struct ActorHandle {
    sender: mpsc::Sender<ActorMessage>,
    pub use_gcr_padding: bool,
}

impl ActorHandle {
    pub fn new(simulator: Simulator) -> Self {
        let use_gcr_padding = simulator.use_gcr_padding();
        let (sender, receiver) = mpsc::channel();
        let actor = Actor::new(simulator, receiver);
        std::thread::spawn(move || run_simulator_actor(actor));
        Self {
            sender,
            use_gcr_padding,
        }
    }

    fn call<T>(&self, msg: ActorMessage, recv: mpsc::Receiver<T>) -> Result<T, Error> {
        self.sender
            .send(msg)
            .map_err(|e| errors::Error::ActorSend { e: e.to_string() })?;
        recv.recv().map_err(|e| Error::ActorDied { source: e })
    }

    pub fn start_session(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        self.call(ActorMessage::StartSession { reply_to: send }, recv)?
            .map_err(|e| Error::Simulation { source: e })
    }

    pub fn stop_session(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        self.call(ActorMessage::StopSession { reply_to: send }, recv)?
            .map_err(|e| Error::Simulation { source: e })
    }

    pub fn generate_qkd_batch(&self) -> Result<QkdBatch, Error> {
        let (send, recv) = mpsc::channel();
        self.call(ActorMessage::GenerateQkdBatch { reply_to: send }, recv)?
            .map_err(|e| Error::Simulation { source: e })
    }

    pub fn generation_progress(&self) -> Result<GenerationProgress, Error> {
        let (send, recv) = mpsc::channel();
        self.call(ActorMessage::GenerationProgress { reply_to: send }, recv)
    }

    pub fn discard_batches(&self, count: u64) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        self.call(
            ActorMessage::DiscardBatches {
                count,
                reply_to: send,
            },
            recv,
        )?
        .map_err(|e| Error::Simulation { source: e })
    }

    pub fn set_angles(&self, angles: [u8; 4]) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        self.call(
            ActorMessage::SetAngles {
                angles,
                reply_to: send,
            },
            recv,
        )?
        .map_err(|e| Error::Simulation { source: e })
    }

    pub fn start_attack(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        self.call(ActorMessage::StartAttack { reply_to: send }, recv)?
            .map_err(|e| Error::Simulation { source: e })
    }

    pub fn stop_attack(&self) -> Result<(), Error> {
        let (send, recv) = mpsc::channel();
        self.call(ActorMessage::StopAttack { reply_to: send }, recv)?
            .map_err(|e| Error::Simulation { source: e })
    }
}
