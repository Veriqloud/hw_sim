use crossbeam_channel::{Receiver, Sender, bounded};
use std::thread::{self, JoinHandle};

use crate::{BATCH, ServiceCorrelationsRandom, errors::SimulationError, simulation::Simulator};

#[derive(Debug, Clone, Copy)]
pub struct QkdBatch {
    pub click_results: [u8; 1024],
    pub alice_angles: [u8; 1024],
    pub bob_angles: [u8; 1024],
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

/// Spawns a new QKD session in a background thread.
/// Takes ownership of the `Simulator` to allow parallel, independent instances.
/// Returns a handle to stop the session and a receiver for the generated batches.
pub fn spawn_session(
    mut simulator: impl ServiceCorrelationsRandom + 'static,
    buffer_size: usize,
) -> Result<(SessionHandle, Receiver<QkdBatch>), SimulationError> {
    if let Err(e) = simulator.start_session() {
        return Err(e);
    };

    let (batch_tx, batch_rx) = bounded(buffer_size);
    let (stop_tx, stop_rx) = bounded(1);

    let handle = thread::spawn(move || {
        loop {
            // Generate the batch (cannot be interrupted internally)
            let batch_result = simulator.generate_qkd_batch();
            let batch = match batch_result {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("session failed: {e}");
                    return;
                }
            };

            if stop_rx.try_recv().is_ok() {
                break;
            }

            if let Err(e) = batch_tx.send(batch) {
                tracing::error!(
                    "failed to send qkd batch over results channel. Ending session: {e}"
                );
                return;
            };
        }

        if let Err(e) = simulator.stop_session() {
            tracing::error!("failed to stop session: {e}");
            return;
        }
    });

    let handle = SessionHandle {
        stop_tx,
        thread_handle: Some(handle),
    };

    Ok((handle, batch_rx))
}

impl ServiceCorrelationsRandom for Simulator {
    fn generate_qkd_batch(&mut self) -> Result<QkdBatch, SimulationError> {
        let (alice_indices, bob_indices, click_results) = self.generate_correlation_batch()?;

        let angles_vec = &self.angles;
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

    fn start_session(&mut self) -> Result<(), SimulationError> {
        self.start_session()
    }

    fn stop_session(&mut self) -> Result<(), SimulationError> {
        self.stop_session()
    }
}
