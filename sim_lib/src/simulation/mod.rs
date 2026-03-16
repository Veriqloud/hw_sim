use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use std::time::{Duration, Instant};

use crate::{
    BATCH, BATCH_SIZE, OVERLAP_PROBABILITIES,
    errors::{HardwareError, ProtocolError, SimulationError},
    hardware::{Hardware, modes::SimulatorMode, modulator_state::ModulatorState},
};

pub mod batches;

#[derive(Debug, PartialEq)]
pub struct Simulator {
    pub(crate) angles: Vec<u8>,
    pub eta: f64,
    pub(crate) global_counter: u64,
    pub(crate) hw: Hardware,
    pub(crate) modulator_state: ModulatorState,
    pub now: Instant,
    // Replaced by batch-oriented processing
    pub(crate) pending_angles_batch: Option<Vec<u8>>,
    pub qb_err: f64,
    pub(crate) rng: Pcg64Mcg,
    pub(crate) seed: u64,
    pub simulator_mode: SimulatorMode,
    pub(crate) time_of_start: Option<Instant>,
    pub(crate) last_event_count: u64,
    pub(crate) use_gcr_padding: bool,
    pub rate_limiting_enabled: bool,
}

impl Simulator {
    /// The core logic for generating a single batch of correlated events.
    /// This private helper function simulates a two-party quantum communication protocol.
    /// For each event, both parties randomly choose a basis (angle). The sum of their angles
    /// determines the probability of the measurement outcome (0 or 1). Because both parties'
    /// It returns the raw data for a full batch: the chosen basis indices for both parties
    /// and the shared measurement results.
    fn generate_correlation_batch(
        &mut self,
    ) -> Result<([usize; 1024], [usize; 1024], [u8; 1024]), ProtocolError> {
        // Ensure the simulator is in the correct state for this protocol.
        let (angles_vec, num_angles) = match &self.modulator_state {
            ModulatorState::Random => {
                let angles = &self.angles;
                (angles.as_slice(), angles.len() as u16)
            }
            _ => {
                return Err(ProtocolError::Role {
                    reason: "Modulator state must be Random for generating correlations."
                        .to_string(),
                });
            }
        };

        // Pre-calculate the probability lookup table for measurement outcomes.
        let overlap_probabilities = &OVERLAP_PROBABILITIES;
        // Convert the QBER (a float from 0.0 to 1.0) to a u16 threshold for random comparison.
        let qber_threshold: u16 = (self.qb_err * (u16::MAX as f64)) as u16;

        // --- Random Number Generation ---
        // These random numbers determine the choices and outcomes for the batch.
        // Since the RNG is seeded, both Alice's and Bob's simulators will generate
        // the identical streams of random numbers, ensuring their results are correlated.

        // Random numbers to select Alice's basis for each event.
        let mut alice_basis_rand = [0u16; BATCH];
        self.rng.fill(&mut alice_basis_rand);

        // Random numbers to select Bob's basis for each event.
        let mut bob_basis_rand = [0u16; BATCH];
        self.rng.fill(&mut bob_basis_rand);

        // Random numbers to determine the measurement outcome based on probability.
        let mut result_rand = [0u16; BATCH];
        self.rng.fill(&mut result_rand);

        // Random numbers to simulate the Quantum Bit Error Rate (QBER).
        let mut qber_rand = [0u16; BATCH];
        self.rng.fill(&mut qber_rand);

        let mut alice_indices = [0; BATCH];
        let mut bob_indices = [0; BATCH];
        let mut click_results = [0; BATCH];

        for i in 0..BATCH {
            let alice_basis_index = (alice_basis_rand[i] % num_angles) as usize;
            let bob_basis_index = (bob_basis_rand[i] % num_angles) as usize;

            // Calculate the total angle. Angles are u8 offsets in a 128-step circle.
            // The +32 simulates Alice sending a |+> state instead of |0>.
            let total_angle_offset = (angles_vec[alice_basis_index] as u32
                + angles_vec[bob_basis_index] as u32
                + 32) as u8
                & 127;

            // Determine the measurement result based on the total angle.
            // `overlap_probabilities` holds pre-calculated cos^2 values scaled to u16::MAX.
            // This value represents the probability of a '0' outcome.
            let probability_of_0 = overlap_probabilities[total_angle_offset as usize];
            let mut result = (result_rand[i] > probability_of_0) as u8;

            // Simulate Quantum Bit Error Rate (QBER).
            if qber_rand[i] < qber_threshold {
                result ^= 1;
            }

            alice_indices[i] = alice_basis_index;
            bob_indices[i] = bob_basis_index;
            click_results[i] = result;
        }

        Ok((alice_indices, bob_indices, click_results))
    }

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

    /// Initializes the simulator state for starting a generation sequence.
    /// Resets counters and sets the modulator state.
    pub fn start_session(&mut self) -> Result<(), SimulationError> {
        tracing::info!("Simulator: Start session command received. Initializing for generation.");
        self.global_counter = 0; // Reset GC for the new session
        self.time_of_start = Some(Instant::now());
        self.modulator_state = ModulatorState::Random; // Ready to generate
        self.pending_angles_batch = None;
        self.reset_time(); // Reset self.now for internal time calculations if any
        self.last_event_count = 0; // Reset event counter for the new session
        self.rng = Pcg64Mcg::seed_from_u64(self.seed); // Re-seed the RNG
        Ok(())
    }

    /// Stops the current generation sequence and resets state.
    pub fn stop_session(&mut self) -> Result<(), SimulationError> {
        tracing::info!("Simulator: Stop session command received. Halting generation.");
        self.modulator_state = ModulatorState::Idle;
        self.time_of_start = None;
        self.pending_angles_batch = None;
        tracing::info!("Simulator modulator state changed to IDLE.");
        Ok(())
    }

    fn generate_encoded_party_data(&mut self, l: usize) -> Result<Vec<u8>, ProtocolError> {
        // The output vector to store the encoded results.
        let mut output_bytes: Vec<u8> = Vec::with_capacity(l);

        // Process in batches for efficiency.
        for _ in 0..(l.div_ceil(BATCH)) {
            let (alice_indices, bob_indices, click_results) = self.generate_correlation_batch()?;

            let my_indices = match self.simulator_mode {
                SimulatorMode::Source => &alice_indices,
                SimulatorMode::Detector => &bob_indices,
            };

            for i in 0..BATCH {
                let my_basis_index = my_indices[i];
                let result = click_results[i];
                output_bytes.push(((my_basis_index as u8) << 1) | result);
            }
        }

        // Trim the vector to the exact requested length `l`.
        output_bytes.truncate(l);
        output_bytes.shrink_to_fit();
        Ok(output_bytes)
    }

    /// Generates a batch of GCR (Global Counter + Result) data and corresponding angles.
    /// The GCR data is returned, and angles are stored internally.
    /// GCs are deterministic (incrementing sequence). Clicks and angles are random.
    pub fn generate_gcr_and_angles_batch(&mut self) -> Result<Vec<[u8; 8]>, SimulationError> {
        if self.modulator_state != ModulatorState::Random {
            return Err(SimulationError::HardwareError {
                source: HardwareError::ModulatorStateNotSupported,
            });
        }

        let time_of_start = self.time_of_start.ok_or_else(|| HardwareError::Other {
            reason: "Simulator session not started (time_of_start is None).".to_string(),
        })?;

        // The base global counter for this batch is simply the number of events
        // generated before this batch.
        let base_gc_for_batch = self.last_event_count;
        tracing::info!(
            "Simulator: Generating GCR and angles batch ({} items). Base GC for this batch: {}",
            BATCH_SIZE,
            base_gc_for_batch
        );

        // Obtain raw random bytes for events. Each byte from correlations_random is one event (angle + result).
        let data = self.generate_encoded_party_data(BATCH_SIZE).map_err(|e| {
            tracing::error!(
                "Failed to get raw random bytes from generate_encoded_party_data: {:?}",
                e
            );
            HardwareError::Other {
                reason: format!("generate_encoded_party_data failed: {}", e),
            }
        })?;

        if data.len() < BATCH_SIZE {
            return Err(SimulationError::HardwareError {
                source: HardwareError::Other {
                    reason: format!(
                        "correlations_random returned insufficient data: got {}, expected {}",
                        data.len(),
                        BATCH_SIZE
                    ),
                },
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

        // This logic is now placed *after* all CPU-intensive work for the batch.
        // Calculate the theoretical time at which this batch should be finished.
        let target_event_count = self.last_event_count; // Use the count *after* this batch
        let target_duration_from_start = if self.eta > 0.0 {
            // Time = (Number of events * pulse_distance) / eta
            let time_in_secs = (target_event_count as f64 * self.hw.pulse_distance) / self.eta;
            Duration::from_secs_f64(time_in_secs)
        } else {
            Duration::ZERO
        };

        if self.rate_limiting_enabled && target_duration_from_start > Duration::ZERO {
            let elapsed_since_start = time_of_start.elapsed();
            if elapsed_since_start < target_duration_from_start {
                let sleep_duration = target_duration_from_start - elapsed_since_start;
                tracing::debug!("Rate limiting: sleeping for {:?}", sleep_duration);
                std::thread::sleep(sleep_duration);
            }
        }

        Ok(gcr_batch)
    }

    /// Called after the reader has received GC values from the controller.
    /// This method retrieves the internally stored batch of angles corresponding
    /// to the previously generated GCR data.
    pub fn retrieve_pending_angles_batch(
        &mut self,
        received_gc_values: Vec<u64>, // Used to determine BATCH_SIZE, actual values not used in random generation
    ) -> Result<Vec<u8>, SimulationError> {
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
            Err(SimulationError::HardwareError {
                source: HardwareError::Other {
                    reason: "No pending angles batch to retrieve.".to_string(),
                },
            })
        }
    }

    // set_angles remains for configuration purposes
    pub fn set_angles(&mut self, angles_config: [u8; 4]) -> Result<(), SimulationError> {
        self.angles = angles_config.to_vec(); // These are configuration angles (bases)
        Ok(())
    }

    /// Generates a batch of angles based on received GCs (primarily for Source mode).
    /// This method does not generate GCRs or affect the internal global_counter.
    pub fn generate_angles_for_gcs(
        &mut self,
        received_gcs: Vec<u64>,
    ) -> Result<Vec<u8>, SimulationError> {
        if self.modulator_state != ModulatorState::Random {
            return Err(SimulationError::HardwareError {
                source: HardwareError::ModulatorStateNotSupported,
            });
        }
        let current_batch_size = received_gcs.len();

        if current_batch_size == 0 {
            // Or handle as appropriate, e.g., return empty Vec or specific error
            return Err(SimulationError::HardwareError {
                source: HardwareError::Other {
                    reason: "Received empty GC batch for angle generation.".to_string(),
                },
            });
        }

        tracing::info!(
            "Simulator (Source Mode Flow): Generating angles batch ({} items) based on received GCs.",
            current_batch_size
        );

        // Obtain raw random bytes for events. Each byte from correlations_random is one event (angle + result).
        let data = self
            .generate_encoded_party_data(current_batch_size)
            .map_err(|e| {
                tracing::error!(
                    "Failed to get raw random bytes from generate_encoded_party_data: {:?}",
                    e
                );
                HardwareError::Other {
                    reason: format!("generate_encoded_party_data failed: {}", e),
                }
            })?;

        if data.len() < current_batch_size {
            return Err(SimulationError::HardwareError {
                source: HardwareError::Other {
                    reason: format!(
                        "correlations_random returned insufficient data: got {}, expected {}",
                        data.len(),
                        current_batch_size
                    ),
                },
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
