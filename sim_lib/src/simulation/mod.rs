use configs::backend::{DecoyStatesConfig, QberConfig};
use rand::RngExt;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GenerationProgress {
    pub event_count: u64,
    pub batch_pulse_count: u64,
}

#[derive(Debug, PartialEq)]
pub struct Simulator {
    pub(crate) angles: Vec<u8>,
    pub eta: f64,
    pub(crate) hw: Hardware,
    pub(crate) modulator_state: ModulatorState,
    pub now: Instant,
    pub qb_err: QberConfig,
    pub(crate) rng: Pcg64Mcg,
    pub(crate) qber_oscillation_rng: Pcg64Mcg,
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
    fn batch_pulse_count(&self) -> u64 {
        let effective_eta = self.decoy_effective_eta().unwrap_or(self.eta);
        if effective_eta > 0.0 {
            (BATCH_SIZE as f64 / effective_eta).round() as u64
        } else {
            BATCH_SIZE as u64
        }
    }

    pub fn generation_progress(&self) -> GenerationProgress {
        GenerationProgress {
            event_count: self.last_event_count,
            batch_pulse_count: self.batch_pulse_count(),
        }
    }

    pub fn discard_batches(&mut self, count: u64) -> Result<(), SimulationError> {
        for _ in 0..count {
            self.generate_batch()?;
        }
        Ok(())
    }

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

    /// Generates a single batch of correlated QKD events (bases + results).
    ///
    /// Returns a `QkdBatch` with `base_gc = 0` and `gc_step = 1`. These are
    /// placeholder values — callers that only need the correlation data (angles,
    /// results, decoy states) can use this directly. For hardware-accurate GC
    /// timestamps and rate limiting use `generate_batch()` instead.
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
            base_gc: 0,
            gc_step: 1,
            alice_state_index,
            bob_state_index,
            results,
            decoy_states,
        })
    }

    pub fn use_gcr_padding(&self) -> bool {
        self.use_gcr_padding
    }

    /// Reset time to now
    pub fn reset_time(&mut self) {
        self.now = Instant::now();
    }

    /// Initializes the simulator state for starting a generation sequence.
    /// Resets counters and sets the modulator state without restarting the random streams.
    pub fn initialize_session(&mut self) -> Result<(), SimulationError> {
        tracing::info!("Simulator: Start session command received. Initializing for generation.");
        self.time_of_start = Some(Instant::now());
        self.modulator_state = ModulatorState::Random; // Ready to generate
        self.reset_time(); // Reset self.now for internal time calculations if any
        self.last_event_count = 0; // Reset event counter for the new session
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

    /// Generates one batch of correlated QKD events, applies rate limiting, and
    /// advances the internal event counter. This is the primitive the actor calls.
    pub fn generate_batch(&mut self) -> Result<QkdBatch, SimulationError> {
        if self.modulator_state != ModulatorState::Random {
            return Err(SimulationError::HardwareError {
                source: HardwareError::ModulatorStateNotSupported,
            });
        }

        let time_of_start = self.time_of_start.ok_or_else(|| HardwareError::Other {
            reason: "Simulator session not started (time_of_start is None).".to_string(),
        })?;

        // How many laser pulses does one batch of BATCH_SIZE clicks consume?
        // On average: BATCH_SIZE / effective_eta pulses.
        // last_event_count tracks the total pulse count (= the running GC offset from
        // session start), so it grows at the laser repetition rate, not the click rate.
        let batch_pulse_count = self.batch_pulse_count();
        // Average pulse-counter gap between two consecutive detection events.
        let gc_step = (batch_pulse_count / BATCH_SIZE as u64).max(1);

        tracing::info!("Simulator: Generating batch ({} items, {} pulses).", BATCH_SIZE, batch_pulse_count);

        let base_gc = self.hw.gc_offset + self.last_event_count;
        let inner = self.generate_correlation_batch().map_err(|e| {
            tracing::error!("generate_correlation_batch failed: {:?}", e);
            HardwareError::Other {
                reason: format!("generate_correlation_batch failed: {}", e),
            }
        })?;
        let batch = QkdBatch { base_gc, gc_step, ..inner };

        // Advance pulse counter: next batch starts batch_pulse_count laser pulses later.
        self.last_event_count += batch_pulse_count;

        // Rate limiting: target_time = pulse_count × pulse_distance
        // (equivalent to the old clicks × pulse_distance / eta, but expressed directly
        // in pulse units now that last_event_count tracks pulses).
        let target_duration_from_start =
            Duration::from_secs_f64(self.last_event_count as f64 * self.hw.pulse_distance);

        if self.rate_limiting_enabled && target_duration_from_start > Duration::ZERO {
            let elapsed_since_start = time_of_start.elapsed();
            if elapsed_since_start < target_duration_from_start {
                let sleep_duration = target_duration_from_start - elapsed_since_start;
                tracing::debug!("Rate limiting: sleeping for {:?}", sleep_duration);
                std::thread::sleep(sleep_duration);
            }
        }

        Ok(batch)
    }


    // set_angles remains for configuration purposes
    pub fn set_angles(&mut self, angles_config: [u8; 4]) -> Result<(), SimulationError> {
        self.angles = angles_config.to_vec(); // These are configuration angles (bases)
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use crate::simulation::builder::SimulatorBuilder;
    use configs::backend::QberConfig;
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;

    #[test]
    fn session_restart_preserves_rng_state() {
        let make_simulator = || {
            SimulatorBuilder::new()
                .with_angles(vec![0, 32, 64, 96])
                .with_rng(Pcg64Mcg::seed_from_u64(42))
                .with_seed(42)
                .with_qb_err(QberConfig::Uniform {
                    min: 0.01,
                    max: 0.10,
                })
                .build()
        };
        let mut restarted = make_simulator();
        let mut uninterrupted = make_simulator();

        restarted.initialize_session().unwrap();
        uninterrupted.initialize_session().unwrap();
        let first_batch = restarted.generate_correlation_batch().unwrap();
        let expected_first_batch = uninterrupted.generate_correlation_batch().unwrap();
        assert_eq!(first_batch, expected_first_batch);

        restarted.setup_session_end().unwrap();
        restarted.initialize_session().unwrap();
        let second_batch = restarted.generate_correlation_batch().unwrap();
        let expected_second_batch = uninterrupted.generate_correlation_batch().unwrap();

        assert_eq!(second_batch, expected_second_batch);
        assert_ne!(
            first_batch.alice_state_index, second_batch.alice_state_index,
            "A session restart must not replay the first random batch"
        );
    }

    #[test]
    fn discarding_missing_batches_resynchronizes_random_streams() {
        let make_simulator = || {
            SimulatorBuilder::new()
                .with_angles(vec![0, 32, 64, 96])
                .with_rng(Pcg64Mcg::seed_from_u64(42))
                .with_seed(42)
                .with_qb_err(QberConfig::Uniform {
                    min: 0.01,
                    max: 0.10,
                })
                .with_rate_limiter(false)
                .build()
        };
        let mut ahead = make_simulator();
        let mut lagging = make_simulator();

        ahead.initialize_session().unwrap();
        lagging.initialize_session().unwrap();
        for _ in 0..5 {
            ahead.generate_batch().unwrap();
        }

        let ahead_progress = ahead.generation_progress();
        let lagging_progress = lagging.generation_progress();
        let missing_batches = (ahead_progress.event_count - lagging_progress.event_count)
            / ahead_progress.batch_pulse_count;
        lagging.discard_batches(missing_batches).unwrap();

        assert_eq!(ahead.generation_progress(), lagging.generation_progress());
        assert_eq!(
            ahead.generate_correlation_batch().unwrap(),
            lagging.generate_correlation_batch().unwrap()
        );
    }

    #[test]
    fn test_under_attack_qber() {
        let make_simulator = || {
            SimulatorBuilder::new()
                .with_angles(vec![0, 32, 64, 96])
                .with_qb_err(QberConfig::Fixed { value: 0.0 })
                .build()
        };
        let mut sim = make_simulator();
        let mut control = make_simulator();

        sim.initialize_session().unwrap();
        control.initialize_session().unwrap();
        sim.start_attack();
        let results_attack = sim.generate_correlation_batch().unwrap().results;
        let results_control = control.generate_correlation_batch().unwrap().results;

        let mut diffs = 0;
        for i in 0..1024 {
            if results_control[i] != results_attack[i] {
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
