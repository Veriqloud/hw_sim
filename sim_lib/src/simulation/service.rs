use crate::{
    BATCH, ServiceCorrelationsRandom,
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
        // Alice and bob produce indices of angles from the "angle table".
        let (alice_indices, bob_indices, click_results, _decoy_states, _click_mask) =
            self.sim.generate_correlation_batch()?;

        let angles_vec = &self.sim.angles;
        let mut batch = QkdBatch {
            click_results,
            alice_angles: [0; 1024],
            bob_angles: [0; 1024],
        };

        for i in 0..BATCH {
            batch.alice_angles[i] = angles_vec[alice_indices[i]];
            batch.bob_angles[i] = angles_vec[bob_indices[i]];
        }

        Ok(batch)
    }

    fn init_session(&mut self) -> Result<(), SimulationError> {
        self.sim.initialize_session()
    }
}
