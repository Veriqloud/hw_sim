use configs::backend::{DecoyStatesConfig, QberConfig};
use rand::{RngExt, SeedableRng};
use rand_pcg::Pcg64Mcg;
use std::time::{Duration, Instant};

use bitvec::prelude::*;

use crate::{
    BATCH, BATCH_BYTES, BATCH_SIZE, OVERLAP_PROBABILITIES,
    errors::{HardwareError, ProtocolError, SimulationError},
    hardware::{Hardware, modes::SimulatorMode, modulator_state::ModulatorState},
    simulation::batches::QkdBatch,
};

pub mod batches;
pub mod builder;
pub mod service;

#[derive(Debug, PartialEq)]
pub struct Simulator {
    pub(crate) angles: Vec<u8>,
    pub eta: f64,
    pub(crate) global_counter: u64,
    pub(crate) hw: Hardware,
    pub(crate) modulator_state: ModulatorState,
    pub now: Instant,
    pub qb_err: QberConfig,
    pub(crate) rng: Pcg64Mcg,
    pub(crate) qber_oscillation_rng: Pcg64Mcg,
    pub(crate) seed: u64,
    pub simulator_mode: SimulatorMode,
    pub(crate) time_of_start: Option<Instant>,
    pub(crate) last_event_count: u64,
    pub(crate) use_gcr_padding: bool,
    pub rate_limiting_enabled: bool,
    pub is_under_attack: bool,
    /// Decoy-state parameters. None means decoy mode is disabled.
    pub decoy_states: Option<DecoyStatesConfig>,
}

impl Simulator {
    /// Average click probability across both intensities, used as the effective
    /// detection efficiency for rate limiting in decoy-state mode.
    ///
    /// p_avg = p1·(1 − e^{−µ1·η}) + (1−p1)·(1 − e^{−µ2·η})
    ///
    /// Returns None when decoy mode is disabled.
    fn decoy_effective_eta(&self) -> Option<f64> {
        self.decoy_states.as_ref().map(|ds| {
            ds.p1 * (1.0 - (-ds.mu1 * self.eta).exp())
                + (1.0 - ds.p1) * (1.0 - (-ds.mu2 * self.eta).exp())
        })
    }

    pub fn start_attack(&mut self) {
        self.is_under_attack = true;
        tracing::warn!("QKD Attack started: QBER forced to 50%");
    }

    pub fn stop_attack(&mut self) {
        self.is_under_attack = false;
        tracing::info!("QKD Attack stopped: returning to configured QBER");
    }

    /// The core logic for generating a single batch of correlated events.
    /// This private helper function simulates a two-party quantum communication protocol.
    /// For each event, both parties randomly choose a basis (angle). The sum of their angles
    /// determines the probability of the measurement outcome (0 or 1). Because both parties'
    /// It returns the raw data for a full batch: the chosen basis indices for both parties
    /// and the shared measurement results.
    pub fn generate_correlation_batch(&mut self) -> Result<QkdBatch, ProtocolError> {
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

        // --- QBER Oscillation (using dedicated RNG) ---
        // We use a separate RNG so that changing QBER parameters doesn't
        // desynchronize the main RNG sequence used for angles and measurement results.
        let current_qb_err = if self.is_under_attack {
            0.5
        } else {
            match &self.qb_err {
                QberConfig::Fixed { value } => *value,
                QberConfig::Uniform { min, max } => {
                    self.qber_oscillation_rng.random_range(*min..=*max)
                }
                QberConfig::Gaussian { mean, std_dev } => {
                    // Box-Muller transform for normal distribution
                    let u1: f64 = self.qber_oscillation_rng.random();
                    let u2: f64 = self.qber_oscillation_rng.random();
                    let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                    (mean + z0 * std_dev).clamp(0.0, 1.0)
                }
            }
        };

        // Convert the current QBER (a float from 0.0 to 1.0) to a u16 threshold for random comparison.
        let qber_threshold: u16 = (current_qb_err * (u16::MAX as f64)) as u16;

        // --- Random Number Generation (using main RNG) ---
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

        let mut decoy_rand = [0u16; BATCH];
        // P(choose mu1) threshold scaled to u16::MAX — Some only when decoy mode is active.
        let p1_threshold: Option<u16> = self.decoy_states.as_ref().map(|ds| {
            self.rng.fill(&mut decoy_rand);
            (ds.p1 * u16::MAX as f64) as u16
        });

        let mut alice_state_index = [0u8; BATCH];
        let mut bob_state_index = [0u8; BATCH];
        let mut results = BitArray::<[u8; BATCH_BYTES], Lsb0>::ZERO;
        let mut decoy_states = BitArray::<[u8; BATCH_BYTES], Lsb0>::ZERO;

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
            let mut result = result_rand[i] > probability_of_0;

            // Simulate Quantum Bit Error Rate (QBER).
            if qber_rand[i] < qber_threshold {
                result = !result;
            }

            alice_state_index[i] = alice_basis_index as u8;
            bob_state_index[i] = bob_basis_index as u8;
            results.set(i, result);

            if let Some(threshold) = p1_threshold {
                // false = signal (mu1), true = decoy (mu2).
                decoy_states.set(i, decoy_rand[i] >= threshold);
            }
            // In non-decoy mode decoy_states[i] stays false.
        }

        Ok(QkdBatch {
            alice_state_index,
            bob_state_index,
            results,
            decoy_states,
        })
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
    pub fn initialize_session(&mut self) -> Result<(), SimulationError> {
        tracing::info!("Simulator: Start session command received. Initializing for generation.");
        self.global_counter = 0; // Reset GC for the new session
        self.time_of_start = Some(Instant::now());
        self.modulator_state = ModulatorState::Random; // Ready to generate
        self.reset_time(); // Reset self.now for internal time calculations if any
        self.last_event_count = 0; // Reset event counter for the new session
        self.rng = Pcg64Mcg::seed_from_u64(self.seed); // Re-seed the RNG
        self.qber_oscillation_rng = Pcg64Mcg::seed_from_u64(self.seed); // Re-seed the oscillation RNG
        Ok(())
    }

    /// Stops the current generation sequence and resets state.
    pub fn setup_session_end(&mut self) -> Result<(), SimulationError> {
        tracing::info!("Simulator: Stop session command received. Halting generation.");
        self.modulator_state = ModulatorState::Idle;
        self.time_of_start = None;
        tracing::info!("Simulator modulator state changed to IDLE.");
        Ok(())
    }

    /// Generates a batch of GCR (Global Counter + Result) data and corresponding angles.
    /// Returns `(gcr_batch, angles_data)` together.
    /// GCs are deterministic (incrementing sequence). Clicks and angles are random.
    pub fn generate_gcr_and_angles_batch(&mut self) -> Result<(Vec<[u8; 8]>, Vec<u8>), SimulationError> {
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

        let batch = self.generate_correlation_batch().map_err(|e| {
            tracing::error!("generate_correlation_batch failed: {:?}", e);
            HardwareError::Other {
                reason: format!("generate_correlation_batch failed: {}", e),
            }
        })?;

        let angles_data = match self.simulator_mode {
            SimulatorMode::Source => batch.to_alice_fifo(),
            SimulatorMode::Detector => batch.to_bob_angle_fifo(),
        };

        let capacity = if self.use_gcr_padding {
            2 * BATCH_SIZE
        } else {
            BATCH_SIZE
        };
        let mut gcr_batch = Vec::with_capacity(capacity);
        for (i, result) in batch.results.iter().enumerate() {
            let gc_value = base_gc_for_batch + i as u64;
            tracing::debug!(
                "Simulator: Encoding GC={}, ResultBit={} for GCR item #{}",
                gc_value,
                *result as u8,
                i
            );
            let gcr_item = self.encode_gcr(gc_value, *result as u8);

            gcr_batch.push(gcr_item);
            if self.use_gcr_padding {
                // The external `gc` program expects a 16-byte record per GCR.
                // The first 8 bytes are the GCR, the next 8 are padding.
                gcr_batch.push([0u8; 8]);
            }
        }
        self.last_event_count += BATCH_SIZE as u64; // Increment total generated events

        tracing::info!(
            "Simulator: Generated batch. Total events generated: {}. Angles: {} bytes.",
            self.last_event_count,
            angles_data.len()
        );

        // Rate limiting: target_time = clicks × pulse_distance / effective_eta
        //
        // effective_eta is eta in standard mode.  In decoy-state mode it is the
        // theoretical average click probability across both intensities:
        //   p_avg = p1·(1−e^{−µ1η}) + (1−p1)·(1−e^{−µ2η})
        let effective_eta = self.decoy_effective_eta().unwrap_or(self.eta);
        let target_duration_from_start = if effective_eta > 0.0 {
            let time_in_secs =
                (self.last_event_count as f64 * self.hw.pulse_distance) / effective_eta;
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

        Ok((gcr_batch, angles_data))
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

        let batch = self.generate_correlation_batch().map_err(|e| {
            tracing::error!("generate_correlation_batch failed: {:?}", e);
            HardwareError::Other {
                reason: format!("generate_correlation_batch failed: {}", e),
            }
        })?;

        let angles_data = match self.simulator_mode {
            SimulatorMode::Source => batch.to_alice_fifo(),
            SimulatorMode::Detector => batch.to_bob_angle_fifo(),
        };

        self.last_event_count += current_batch_size as u64; // Increment total generated events

        tracing::info!(
            "Simulator (Source Mode Flow): Generated {} angle bytes.",
            angles_data.len()
        );
        Ok(angles_data)
    }
}

#[cfg(test)]
mod tests {
    use crate::simulation::builder::SimulatorBuilder;
    use configs::backend::QberConfig;

    #[test]
    fn test_under_attack_qber() {
        let mut sim = SimulatorBuilder::new()
            .with_angles(vec![0, 32, 64, 96])
            .with_qb_err(QberConfig::Fixed { value: 0.0 }) // Normal QBER is 0%
            .build();

        // 1. Normal session
        sim.initialize_session().unwrap();
        let results_normal = sim.generate_correlation_batch().unwrap().results;

        // 2. Attack session (reset session to get the same RNG sequence)
        sim.initialize_session().unwrap();
        sim.start_attack();
        let results_attack = sim.generate_correlation_batch().unwrap().results;

        // 3. Calculate actual QBER (fraction of flipped bits)
        let mut diffs = 0;
        for i in 0..1024 {
            if results_normal[i] != results_attack[i] {
                diffs += 1;
            }
        }

        let calculated_qber = diffs as f64 / 1024.0;
        tracing::info!("Calculated attack QBER: {}", calculated_qber);

        // With 1024 samples, QBER should be around 0.5.
        assert!(
            calculated_qber > 0.4 && calculated_qber < 0.6,
            "Attack QBER should be around 0.5, got {}",
            calculated_qber
        );

        assert!(sim.is_under_attack);
        sim.stop_attack();
        assert!(!sim.is_under_attack);
    }
}
