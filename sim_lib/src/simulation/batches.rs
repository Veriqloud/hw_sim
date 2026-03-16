use std::thread::{self, JoinHandle};
use crossbeam_channel::{bounded, select, Receiver, Sender};

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
    mut simulator: Simulator,
    buffer_size: usize,
) -> (SessionHandle, Receiver<QkdBatch>) {
    // Disable rate limiting to generate as fast as possible for the buffer
    simulator.rate_limiting_enabled = false;
    let _ = simulator.start_session();

    // Channel for outputting batches. Bounded so we can use `select!` effectively.
    let (batch_tx, batch_rx) = bounded(buffer_size);

    // Channel for stopping the thread.
    let (stop_tx, stop_rx) = bounded(1);

    let handle = thread::spawn(move || {
        loop {
            // 1. Non-blocking check before we start heavy generation
            if stop_rx.try_recv().is_ok() {
                break;
            }

            // 2. Generate the batch (cannot be interrupted internally)
            let batch_result = simulator.generate_qkd_batch();

            match batch_result {
                Ok(batch) => {
                    // 3. The "Select" mechanism.
                    // Wait to either successfully send the batch OR receive a stop signal.
                    select! {
                        send(batch_tx, batch) -> res => {
                            if res.is_err() {
                                // Receiver was dropped by orchestrator
                                break;
                            }
                        }
                        recv(stop_rx) -> _ => {
                            // Stop signal received. We throw away the generated `batch`
                            // and break the loop.
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Error generating QKD batch: {:?}. Stopping session.", e);
                    break;
                }
            }
        }

        // Ensure simulator state is reset on exit
        let _ = simulator.stop_session();
    });

    let handle = SessionHandle {
        stop_tx,
        thread_handle: Some(handle),
    };

    (handle, batch_rx)
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
}
