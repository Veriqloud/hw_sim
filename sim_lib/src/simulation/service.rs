use crate::{
    ServiceCorrelationsRandom,
    errors::SimulationError,
    simulation::{Simulator, batches::QkdBatch},
};

pub struct SimulatorService {
    sim: Simulator,
}

impl SimulatorService {
    pub fn new(sim: Simulator) -> Self {
        SimulatorService { sim }
    }
}

impl ServiceCorrelationsRandom for SimulatorService {
    fn generate_qkd_batch(&mut self) -> Result<QkdBatch, SimulationError> {
        Ok(self.sim.generate_correlation_batch()?)
    }

    fn init_session(&mut self) -> Result<(), SimulationError> {
        self.sim.initialize_session()
    }
}
