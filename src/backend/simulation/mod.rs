pub mod builder;
pub mod errors;
pub mod hardware;

use async_trait::async_trait;

use crate::backend::protocols::random::CorrelationsRandom;
use crate::backend::role::SimulatorMode; // SimulatorMode is still needed
use rand_pcg::Pcg64Mcg;
use std::time::Instant;

use self::hardware::errors::HardwareError;
use self::hardware::modulator_state::ModulatorState;
use self::hardware::Hardware;
// Removed: use crate::backend::protocols::errors::ProtocolError;
// Removed: use rand::Rng;

pub const BATCH_SIZE: usize = 1024;

#[derive(Debug, PartialEq)]
pub struct Simulator {
    pub(crate) angles: Vec<u8>,
    pub(crate) current_fifo_size: usize,
    /// Total qubit detection efficiency
    pub eta: f64,
    /// Size of the physical FIFO, for realistic HardwareError, "Size" means number of bytes.
    pub(crate) fifo_max_size: u64,
    /// Offset is taken care of automatically.
    /// Equivalent to Bob broadcasting his global counter in the real world.
    /// Probably not required ...
    pub(crate) global_counter: u64,
    pub(crate) hw: Hardware,
    pub(crate) modulator_state: ModulatorState,
    pub now: Instant,
    // Replaced by batch-oriented processing
    pub(crate) pending_angles_batch: Option<Vec<u8>>,
    /// Qubit error rate
    pub qb_err: f64,
    pub(crate) rng: Pcg64Mcg,
    pub simulator_mode: SimulatorMode, // Added simulator_mode field
    pub(crate) time_of_last_read: f64, // Stores 1024 generated angle values
    pub(crate) time_of_start: Option<Instant>, // To track time for potential future use or logging
}

#[async_trait]
pub trait VqSim {
    /// Initializes the simulator state for starting a generation sequence.
    /// Resets counters and sets the modulator state.
    fn start_session(&mut self) -> Result<(), HardwareError>;

    /// Stops the current generation sequence and resets state.
    fn stop_session(&mut self) -> Result<(), HardwareError>;

    /// Generates a batch of GCR (Global Counter + Result) data and corresponding angles.
    /// The GCR data is returned, and angles are stored internally.
    /// GCs are deterministic (incrementing sequence). Clicks and angles are random.
    async fn generate_gcr_and_angles_batch(&mut self) -> Result<Vec<[u8; 8]>, HardwareError>;

    /// Called after the reader has received GC values from the controller.
    /// This method retrieves the internally stored batch of angles corresponding
    /// to the previously generated GCR data.
    fn retrieve_pending_angles_batch(
        &mut self,
        received_gc_values: Vec<u64>,
    ) -> Result<Vec<u8>, HardwareError>;

    // set_angles remains for configuration purposes
    fn set_angles(&mut self, angles: [u8; 4]) -> Result<(), HardwareError>;
}

#[async_trait]
impl VqSim for Simulator {
    fn start_session(&mut self) -> Result<(), HardwareError> {
        tracing::info!("Simulator: Start session command received. Initializing for generation.");
        self.global_counter = 0; // Reset GC for the new session
        self.time_of_start = Some(Instant::now());
        self.modulator_state = ModulatorState::Random; // Ready to generate
        self.pending_angles_batch = None;
        self.reset_time(); // Reset self.now for internal time calculations if any
                           // RNG will use the seed it was initialized with.
                           // To change the seed, a different mechanism would be needed (e.g. a dedicated actor message or config reload).
        // self.reset_seed(self.time_of_start.unwrap().elapsed().as_nanos() as u64); // Keep seed constant for now
        Ok(())
    }

    fn stop_session(&mut self) -> Result<(), HardwareError> {
        tracing::info!("Simulator: Stop session command received. Halting generation.");
        self.modulator_state = ModulatorState::Idle;
        self.time_of_start = None;
        self.pending_angles_batch = None;
        tracing::info!("Simulator modulator state changed to IDLE.");
        Ok(())
    }

    async fn generate_gcr_and_angles_batch(&mut self) -> Result<Vec<[u8; 8]>, HardwareError> {
        if self.modulator_state != ModulatorState::Random {
            return Err(HardwareError::ModulatorStateNotSupported);
        }
        tracing::info!(
            "Simulator: Generating GCR and angles batch ({} items). Current base GC: {}",
            BATCH_SIZE,
            self.global_counter
        );

        // Obtain raw random bytes for events.
        // Each event (GC, click, angle) needs 2 bytes from correlations_random based on user's logic.
        let data = self.correlations_random(BATCH_SIZE * 2).map_err(|e| {
            tracing::error!(
                "Failed to get raw random bytes from correlations_random: {:?}",
                e
            );
            HardwareError::Other {
                reason: format!("correlations_random failed: {}", e),
            }
        })?;

        // Ensure we have enough data for BATCH_SIZE pairs.
        // The user's provided snippet checks data.len() % 2 != 0,
        // but chunks_exact(2) handles this by processing only full chunks.
        // The critical part is having at least BATCH_SIZE * 2 bytes.
        if data.len() < BATCH_SIZE * 2 {
            return Err(HardwareError::Other {
                reason: format!(
                    "correlations_random returned insufficient data: got {}, expected at least {}",
                    data.len(),
                    BATCH_SIZE * 2
                ),
            });
        }

        // Implement the user's specified separation logic
        // We take exactly BATCH_SIZE items from the iterator produced by chunks_exact(2).map(...)
        // to ensure angles_data and click_results_data have BATCH_SIZE elements.
        let (angles_data, click_results_data): (Vec<u8>, Vec<u8>) = data
            .chunks_exact(2)
            .take(BATCH_SIZE) // Ensure we process exactly BATCH_SIZE pairs
            .map(|chunk| {
                let byte1 = chunk[0];
                let byte2 = chunk[1];
                // angle_byte = ((byte1 & 0b110) >> 1) | ((byte2 & 0b110) << 3);
                let angle_byte = ((byte1 & 0x06) >> 1) | ((byte2 & 0x06) << 3);
                // result_byte = (byte1 & 0b001) | ((byte2 & 0b001) << 4);
                let result_byte = (byte1 & 0x01) | ((byte2 & 0x01) << 4);
                (angle_byte, result_byte)
            })
            .unzip();

        self.pending_angles_batch = Some(angles_data);

        let mut gcr_batch = Vec::with_capacity(BATCH_SIZE);
        for i in 0..BATCH_SIZE {
            let gc_value = self.global_counter + i as u64;
            // The click_results_data contains the `result_byte` which is a u8.
            // The `encode_gcr` function takes a u8 for the result bit.
            // The `split_gcr` function extracts a single bit `(buf_gcr[6] >> 1) & 1;`.
            // So, we should pass only the relevant bit from result_byte to encode_gcr.
            // Assuming the LSB of result_byte is the intended single click result bit.
            let result_byte_for_gcr = click_results_data[i]; 
            let gcr_item = self.encode_gcr(gc_value, result_byte_for_gcr);
            gcr_batch.push(gcr_item);
        }
        self.global_counter += BATCH_SIZE as u64; // Advance base GC for next batch

        tracing::info!(
            "Simulator: Generated batch. Next base GC: {}. Pending angles: {} bytes.",
            self.global_counter,
            self.pending_angles_batch.as_ref().map_or(0, |v| v.len())
        );
        Ok(gcr_batch)
    }

    fn retrieve_pending_angles_batch(
        &mut self,
        received_gc_values: Vec<u64>,
    ) -> Result<Vec<u8>, HardwareError> {
        tracing::info!(
            "Simulator: Received {} GC values from reader. Retrieving pending angles.",
            received_gc_values.len()
        );
        // Validation of received_gc_values can be added here if necessary.
        if let Some(angles) = self.pending_angles_batch.take() {
            tracing::info!("Simulator: Returning {} pending angle bytes.", angles.len());
            Ok(angles)
        } else {
            tracing::warn!(
                "Simulator: retrieve_pending_angles_batch called but no pending angles found."
            );
            Err(HardwareError::Other {
                reason: "No pending angles batch to retrieve.".to_string(),
            })
        }
    }

    fn set_angles(&mut self, angles_config: [u8; 4]) -> Result<(), HardwareError> {
        self.angles = angles_config.to_vec(); // These are configuration angles (bases)
        Ok(())
    }
}

impl Simulator {
    /// Encodes a Global Counter (GC) and a single result bit into an 8-byte GCR format.
    /// Inverse of the user-provided `split_gcr` function.
    /// `split_gcr` implies:
    ///   `gc_val = (original_gc / 2)` stored in most of the 8 bytes.
    ///   `buf[6]` bit 0 stores `original_gc % 2`.
    ///   `buf[6]` bit 1 stores `result_bit`.
    fn encode_gcr(&self, gc: u64, result_bit: u8) -> [u8; 8] {
        let shifted_gc = gc >> 1; // gc / 2
        let gc_lsb = (gc & 1) as u8; // gc % 2

        let mut buffer = shifted_gc.to_le_bytes();

        // Clear bits 0 and 1 of buffer[6] then set them
        buffer[6] = (buffer[6] & 0b1111_1100) | gc_lsb | ((result_bit & 1) << 1);

        buffer
    }

    // /// return time elapsed since start in seconds at nanoseconds.
    // fn get_current_time_with_nanos(&self) -> f64 {
    //     let duration = self.now.elapsed();
    //     duration.as_secs() as f64 + duration.subsec_nanos() as f64 * 1e-9
    // }
    // /// Restart RNG with a new seed.
    // fn reset_seed(&mut self, seed: u64) {
    //     self.rng = Pcg64Mcg::seed_from_u64(seed);
    // }
    /// Reset time to now
    pub fn reset_time(&mut self) {
        self.now = Instant::now();
    }
    /// Update the value of eta
    pub fn set_eta(&mut self, eta: f64) {
        self.eta = eta;
    }
    // /// Set the global counter of the simulator - replaced by internal management
    // pub fn set_gc(&mut self, gc: u64) {
    //     self.global_counter = gc;
    //     self.reset_seed(gc);
    // }
    /// Update the value of qber
    pub fn set_qber(&mut self, qber: f64) {
        self.qb_err = qber;
    }
}
