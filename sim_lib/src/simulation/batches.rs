use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
};

use crate::{BATCH, ServiceCorrelationsRandom, errors::ProtocolError, simulation::Simulator};

#[derive(Debug, Clone, Copy)]
pub struct QkdBatch {
    pub click_results: [u8; 1024],
    pub alice_angles: [u8; 1024],
    pub bob_angles: [u8; 1024],
}

pub trait QkdSession {
    fn stop(&mut self);
    fn start(&mut self) -> mpsc::Receiver<QkdBatch>;
    fn next_batch(&mut self) -> Result<QkdBatch, ProtocolError>;
}

pub struct QkdService {
    simulator: Arc<Mutex<Simulator>>,
    stop_tx: Option<mpsc::Sender<()>>,
}

impl QkdService {
    pub fn new(mut simulator: Simulator) -> Self {
        simulator.rate_limiting_enabled = false;
        Self {
            simulator: Arc::new(Mutex::new(simulator)),
            stop_tx: None,
        }
    }
}

impl QkdSession for QkdService {
    fn stop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            // The send may fail if the receiver is already dropped, which is fine.
            let _ = stop_tx.send(());
        }
    }

    /// Starts a new background thread that continuously generates `QkdBatch`es.
    /// Returns a receiver to get the generated batches.
    /// Calling `start` again will stop the previous session and start a new one.
    fn start(&mut self) -> mpsc::Receiver<QkdBatch> {
        self.stop(); // Ensure any previous session is stopped.

        let (batch_tx, batch_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        self.stop_tx = Some(stop_tx);

        let sim_arc = Arc::clone(&self.simulator);

        thread::spawn(move || {
            loop {
                // Non-blocking check for a stop signal.
                // If `try_recv` returns Ok or a Disconnected error, we should stop.
                match stop_rx.try_recv() {
                    Ok(_) | Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {} // Continue
                }

                let batch_result = {
                    // Lock the simulator to generate a single batch.
                    let mut sim_guard = sim_arc.lock().expect("Simulator mutex poisoned");
                    // Use the high-level batch generation method from the `CorrelationsRandom` trait.
                    sim_guard.generate_qkd_batch()
                };

                match batch_result {
                    Ok(batch) => {
                        if batch_tx.send(batch).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Error generating QKD batch in streaming mode: {:?}. Stopping.",
                            e
                        );
                        break;
                    }
                }
            }
        });

        batch_rx
    }

    /// Generates a single `QkdBatch` on-demand.
    /// This will block if a streaming session is currently generating a batch.
    fn next_batch(&mut self) -> Result<QkdBatch, ProtocolError> {
        let mut sim_guard = self.simulator.lock().expect("Simulator mutex poisoned");
        sim_guard.generate_qkd_batch()
    }
}

impl ServiceCorrelationsRandom for Simulator {
    fn generate_qkd_batch(&mut self) -> Result<QkdBatch, ProtocolError> {
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
