use crossbeam_channel::Sender;
use std::thread::JoinHandle;

use crate::{BATCH, ServiceCorrelationsRandom, errors::SimulationError, simulation::Simulator};

#[derive(Debug, Clone, Copy)]
pub struct QkdBatch {
    pub click_results: [u8; 1024],
    pub alice_angles: [u8; 1024],
    pub bob_angles: [u8; 1024],
    // Logical timestamp of that qkdbatch.
    pub logical_timestamp: Option<usize>,
}

/// Handle to manage a running QKD session.
pub struct SessionHandle {
    stop_tx: Sender<()>,
    thread_handle: Option<JoinHandle<()>>,
}

impl SessionHandle {
    /// Signals the session to stop and waits for the thread to exit.
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    /// Sends a stop signal without waiting for the thread to exit.
    pub fn stop_async(&mut self) {
        let _ = self.stop_tx.send(());
    }
}

impl ServiceCorrelationsRandom for Simulator {
    fn generate_qkd_batch(
        &mut self,
        batch_logical_timestamp: Option<usize>,
    ) -> Result<QkdBatch, SimulationError> {
        // Alice and bob produce indices of angles from the "angle table"
        let (alice_indices, bob_indices, click_results) = self.generate_correlation_batch()?;

        let angles_vec = &self.angles;
        let mut batch = QkdBatch {
            click_results,
            alice_angles: [0; 1024],
            bob_angles: [0; 1024],
            logical_timestamp: batch_logical_timestamp,
        };

        for i in 0..BATCH {
            batch.alice_angles[i] = angles_vec[alice_indices[i]];
            batch.bob_angles[i] = angles_vec[bob_indices[i]];
        }

        Ok(batch)
    }

    fn init_session(&mut self) -> Result<(), SimulationError> {
        self.initialize_session()
    }

    fn setup_session_end(&self) -> Result<(), SimulationError> {
        self.setup_session_end()
    }
}
