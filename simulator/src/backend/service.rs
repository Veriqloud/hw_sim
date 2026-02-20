use crate::backend::protocols::errors::ProtocolError;
use crate::backend::protocols::random::CorrelationsRandom;
use crate::backend::simulation::Simulator;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

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

/// `QkdService` provides a high-level interface to run a QKD session using a single `Simulator`.
/// It acts as a "black box" that produces correlated QKD batches, abstracting away the
/// simulation details. This design allows a future implementation to use real hardware
/// (with an adapter) without changing the service consumer.
///
/// It can generate batches of QKD data on-demand or as a continuous stream.
pub struct QkdService {
    simulator: Arc<Mutex<Simulator>>,
    stop_tx: Option<mpsc::Sender<()>>,
}

impl QkdService {
    /// Creates a new `QkdService` that takes ownership of a `Simulator`.
    /// The simulator should be configured for `ModulatorState::Random` to generate correlations.
    pub fn new(simulator: Simulator) -> Self {
        Self {
            simulator: Arc::new(Mutex::new(simulator)),
            stop_tx: None,
        }
    }
}

impl QkdSession for QkdService {
    /// Stops the continuous generation of `QkdBatch`es if it is running.
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

        thread::spawn(move || loop {
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
                    // If sending fails, the receiver was dropped, so the consumer is gone.
                    if batch_tx.send(batch).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    // Log the error and stop generation.
                    tracing::error!("Error generating QKD batch in streaming mode: {:?}. Stopping.", e);
                    break;
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