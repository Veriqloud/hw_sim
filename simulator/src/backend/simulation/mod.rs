pub mod builder;
pub mod errors;
pub mod hardware;

use async_trait::async_trait;

use crate::backend::protocols::random::CorrelationsRandom;
use crate::backend::role::SimulatorMode; // SimulatorMode is still needed
use rand_pcg::Pcg64Mcg;
use std::time::{Duration, Instant};

use self::hardware::errors::HardwareError;
use self::hardware::modulator_state::ModulatorState;
use self::hardware::Hardware;
// Removed: use crate::backend::protocols::errors::ProtocolError;
// Removed: use rand::Rng;

pub const BATCH_SIZE: usize = 1024;

#[derive(Debug, PartialEq)]
pub struct Simulator {
    pub(crate) angles: Vec<u8>,
    /// Total qubit detection efficiency
    pub eta: f64,
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
    pub(crate) time_of_start: Option<Instant>, // To track time for potential future use or logging
    pub(crate) last_event_count: u64, // Tracks total events generated in a session
    pub(crate) use_gcr_padding: bool,
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

    /// Generates a batch of angles based on received GCs (primarily for Source mode).
    /// This method does not generate GCRs or affect the internal global_counter.
    async fn generate_angles_for_gcs(
        &mut self,
        received_gcs: Vec<u64>, // Used to determine BATCH_SIZE, actual values not used in random generation
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
        self.last_event_count = 0; // Reset event counter for the new session
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

        let time_of_start = self.time_of_start
            .ok_or_else(|| HardwareError::Other {
                reason: "Simulator session not started (time_of_start is None).".to_string(),
            })?;

        // --- Rate Limiting Logic ---
        // Calculate the theoretical time at which the *next* batch should be finished.
        let target_event_count = self.last_event_count + BATCH_SIZE as u64;
        let target_duration_from_start = if self.eta > 0.0 {
            // Time = (Number of events / eta) * pulse_distance
            let time_in_secs = (target_event_count as f64 / self.eta) * self.hw.pulse_distance;
            Duration::from_secs_f64(time_in_secs)
        } else {
            // If eta is 0, no events are ever generated. We can just proceed without delay.
            Duration::ZERO
        };

        if target_duration_from_start > Duration::ZERO {
            let elapsed_since_start = time_of_start.elapsed();
            if elapsed_since_start < target_duration_from_start {
                let sleep_duration = target_duration_from_start - elapsed_since_start;
                tracing::debug!("Rate limiting: sleeping for {:?}", sleep_duration);
            tokio::time::sleep(sleep_duration).await;
        }
        }

        // The base global counter for this batch is simply the number of events
        // generated before this batch.
        let base_gc_for_batch = self.last_event_count;
        
        tracing::debug!(
            "Generating batch. Target event count: {}, Target time: {:?}, Current elapsed: {:?}",
            target_event_count,
            target_duration_from_start,
            time_of_start.elapsed()
        );
        tracing::info!(
            "Simulator: Generating GCR and angles batch ({} items). Base GC for this batch: {}",
            BATCH_SIZE,
            base_gc_for_batch
        );

        // Obtain raw random bytes for events. Each byte from correlations_random is one event (angle + result).
        let data = self.correlations_random(BATCH_SIZE).map_err(|e| {
            tracing::error!(
                "Failed to get raw random bytes from correlations_random: {:?}",
                e
            );
            HardwareError::Other {
                reason: format!("correlations_random failed: {}", e),
            }
        })?;

        if data.len() < BATCH_SIZE {
            return Err(HardwareError::Other {
                reason: format!(
                    "correlations_random returned insufficient data: got {}, expected {}",
                    data.len(),
                    BATCH_SIZE
                ),
            });
        }

        let mut angles_data = Vec::with_capacity(BATCH_SIZE);
        let mut click_results_data = Vec::with_capacity(BATCH_SIZE);

        for byte_val in data {
            // Angle is in bits 7-1, result is in bit 0, as per correlations_random encoding: res | (angle << 1)
            angles_data.push(byte_val >> 1);
            click_results_data.push(byte_val & 1);
        }

        self.pending_angles_batch = Some(angles_data);

        let capacity = if self.use_gcr_padding {
            2 * BATCH_SIZE
        } else {
            BATCH_SIZE
        };
        let mut gcr_batch = Vec::with_capacity(capacity);
        for i in 0..BATCH_SIZE {
            let gc_value = base_gc_for_batch + i as u64;
            // click_results_data[i] is now a single bit (0 or 1).
            let result_bit_for_gcr = click_results_data[i];
            tracing::debug!(
                "Simulator: Encoding GC={}, ResultBit={} for GCR item #{}",
                gc_value,
                result_bit_for_gcr,
                i
            );
            let gcr_item = self.encode_gcr(gc_value, result_bit_for_gcr);

            gcr_batch.push(gcr_item);
            if self.use_gcr_padding {
                // The external `gc` program expects a 16-byte record per GCR.
                // The first 8 bytes are the GCR, the next 8 are padding.
                gcr_batch.push([0u8; 8]);
            }
        }
        self.last_event_count += BATCH_SIZE as u64; // Increment total generated events

        tracing::info!(
            "Simulator: Generated batch. Total events generated: {}. Pending angles: {} bytes.",
            self.last_event_count,
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

    async fn generate_angles_for_gcs(
        &mut self,
        received_gcs: Vec<u64>,
    ) -> Result<Vec<u8>, HardwareError> {
        if self.modulator_state != ModulatorState::Random {
            return Err(HardwareError::ModulatorStateNotSupported);
        }
        let current_batch_size = received_gcs.len();

        if current_batch_size == 0 {
            // Or handle as appropriate, e.g., return empty Vec or specific error
            return Err(HardwareError::Other {
                reason: "Received empty GC batch for angle generation.".to_string(),
            });
        }

        tracing::info!(
            "Simulator (Source Mode Flow): Generating angles batch ({} items) based on received GCs.",
            current_batch_size
        );

        // Obtain raw random bytes for events. Each byte from correlations_random is one event (angle + result).
        let data = self.correlations_random(current_batch_size).map_err(|e| {
            tracing::error!(
                "Failed to get raw random bytes from correlations_random: {:?}",
                e
            );
            HardwareError::Other {
                reason: format!("correlations_random failed: {}", e),
            }
        })?;

        if data.len() < current_batch_size {
            return Err(HardwareError::Other {
                reason: format!(
                    "correlations_random returned insufficient data: got {}, expected {}",
                    data.len(),
                    current_batch_size
                ),
            });
        }

        // Extract angles directly. Result bits are not used in this flow by the caller.
        let angles_data: Vec<u8> = data.iter().map(|byte_val| byte_val >> 1).collect();

        self.last_event_count += current_batch_size as u64; // Increment total generated events

        tracing::info!(
            "Simulator (Source Mode Flow): Generated {} angle bytes.",
            angles_data.len()
        );
        Ok(angles_data)
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
    /// Reset time to now
    pub fn reset_time(&mut self) {
        self.now = Instant::now();
    }
}
