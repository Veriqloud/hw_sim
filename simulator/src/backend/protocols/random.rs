use crate::backend::protocols::errors::ProtocolError;
use crate::backend::simulation::hardware::modulator_state::ModulatorState;
use crate::backend::simulation::Simulator;
use rand::Rng;
use once_cell::sync::Lazy;
use std::f64::consts::PI;

mod cr_constants {
    // We work on batches of const size that are appended to v. It's faster that way.
    pub const BATCH: usize = 1 << 10;
}

/// A trait for generating correlated random data for QKD simulation.
pub trait CorrelationsRandom {
    /// Generates encoded event data for a single party (Source or Detector).
    /// The output `Vec<u8>` encodes this party's basis choice and the shared result for each event.
    fn generate_encoded_party_data(&mut self, l: usize) -> Result<Vec<u8>, ProtocolError>;

    /// Generates a complete batch of QKD data, including choices and results for both parties.
    fn generate_qkd_batch(&mut self) -> Result<crate::backend::service::QkdBatch, ProtocolError>;
}

impl CorrelationsRandom for Simulator {
    fn generate_encoded_party_data(&mut self, l: usize) -> Result<Vec<u8>, ProtocolError> {
        // The output vector to store the encoded results.
        let mut output_bytes: Vec<u8> = Vec::with_capacity(l);

        // Process in batches for efficiency.
        for _ in 0..(l.div_ceil(cr_constants::BATCH)) {
            let (alice_indices, bob_indices, click_results) =
                self._generate_correlation_batch()?;

            let my_indices = match self.simulator_mode {
                crate::backend::role::SimulatorMode::Source => &alice_indices,
                crate::backend::role::SimulatorMode::Detector => &bob_indices,
            };

            for i in 0..cr_constants::BATCH {
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

    fn generate_qkd_batch(&mut self) -> Result<crate::backend::service::QkdBatch, ProtocolError> {
        let (alice_indices, bob_indices, click_results) = self._generate_correlation_batch()?;

        let angles_vec = &self.angles;
        let mut batch = crate::backend::service::QkdBatch {
            click_results,
            alice_angles: [0; 1024],
            bob_angles: [0; 1024],
        };

        for i in 0..cr_constants::BATCH {
            batch.alice_angles[i] = angles_vec[alice_indices[i]];
            batch.bob_angles[i] = angles_vec[bob_indices[i]];
        }

        Ok(batch)
    }
}

impl Simulator {
    /// The core logic for generating a single batch of correlated events.
    /// This private helper function simulates a two-party quantum communication protocol.
    /// For each event, both parties randomly choose a basis (angle). The sum of their angles
    /// determines the probability of the measurement outcome (0 or 1). Because both parties'
    /// It returns the raw data for a full batch: the chosen basis indices for both parties
    /// and the shared measurement results.
    fn _generate_correlation_batch(
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
                    reason: "Modulator state must be Random for generating correlations.".to_string(),
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
        let mut alice_basis_rand = [0u16; cr_constants::BATCH];
        self.rng.fill(&mut alice_basis_rand);

        // Random numbers to select Bob's basis for each event.
        let mut bob_basis_rand = [0u16; cr_constants::BATCH];
        self.rng.fill(&mut bob_basis_rand);

        // Random numbers to determine the measurement outcome based on probability.
        let mut result_rand = [0u16; cr_constants::BATCH];
        self.rng.fill(&mut result_rand);

        // Random numbers to simulate the Quantum Bit Error Rate (QBER).
        let mut qber_rand = [0u16; cr_constants::BATCH];
        self.rng.fill(&mut qber_rand);

        let mut alice_indices = [0; cr_constants::BATCH];
        let mut bob_indices = [0; cr_constants::BATCH];
        let mut click_results = [0; cr_constants::BATCH];

        for i in 0..cr_constants::BATCH {
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
}

/// A lazily-initialized static lookup table for detection probabilities.
/// The state is |psi> = cos(alpha)|0> + sin(alpha)|1>, where alpha is derived from the angle index.
/// The probability of measuring the initial state is cos^2(alpha).
///
/// The angle index (0-127) maps to a physical angle from 0 to PI.
///
/// The table holds `cos^2(angle)` scaled to `u16::MAX` for each index.
static OVERLAP_PROBABILITIES: Lazy<[u16; 128]> = Lazy::new(|| {
    let mut buf = [0u16; 128];
    for (i, elt) in buf.iter_mut().enumerate() {
        let angle_rad = (i as f64 / 128.0) * PI;
        *elt = (angle_rad.cos().powi(2) * (u16::MAX as f64)) as u16;
    }
    buf
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        role::SimulatorMode,
        simulation::{builder::SimulatorBuilder, hardware::modulator_state::ModulatorState},
    };
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;
    use std::collections::HashMap;
    use std::f64::consts::PI;

    #[test]
    fn test_quantum_correlation_and_qber() {
        // This test verifies that for any given combination of angles, the measured
        // deviation from the ideal quantum result matches the configured `qber`.
        // The `qber` is modeled as a simple bit-flip probability.

        for &qber in &[0.0, 0.05, 0.1, 0.25] {
            println!("\n--- Testing with QBER = {} ---", qber);

            // 1. SETUP
            let seed = 42;
            let test_angles = vec![0u8, 32u8, 64u8, 96u8];
            let num_events = 100_000;

            let mut sim_a = SimulatorBuilder::new()
                .with_rng(Pcg64Mcg::seed_from_u64(seed))
                .with_mode(SimulatorMode::Source)
                .with_qb_err(qber)
                .with_angles(test_angles.clone())
                .with_modulator_state(ModulatorState::Random)
                .build();

            let mut sim_b = SimulatorBuilder::new()
                .with_rng(Pcg64Mcg::seed_from_u64(seed))
                .with_mode(SimulatorMode::Detector)
                .with_qb_err(qber)
                .with_angles(test_angles.clone())
                .with_modulator_state(ModulatorState::Random)
                .build();

            // 2. EXECUTION
            let output_a = sim_a.generate_encoded_party_data(num_events).unwrap();
            let output_b = sim_b.generate_encoded_party_data(num_events).unwrap();

            // 3. DATA GATHERING
            let mut correlation_stats: HashMap<(u8, u8), (u32, u32)> = HashMap::new();
            for i in 0..num_events {
                let result = output_a[i] & 1;
                assert_eq!(result, output_b[i] & 1, "Results must be identical");

                let angle_idx_a = (output_a[i] >> 1) as usize;
                let angle_idx_b = (output_b[i] >> 1) as usize;
                let angle_a = test_angles[angle_idx_a];
                let angle_b = test_angles[angle_idx_b];

                let stats = correlation_stats
                    .entry((angle_a, angle_b))
                    .or_insert((0, 0));
                if result == 0 {
                    stats.0 += 1;
                } else {
                    stats.1 += 1;
                }
            }

            // 4. VERIFICATION
            println!("  - Verifying error rate for all angle combinations:");
            let mut sorted_keys: Vec<_> = correlation_stats.keys().collect();
            sorted_keys.sort();

            for key in sorted_keys {
                let (angle_a, angle_b) = *key;
                let (zeros, ones) = correlation_stats[key];
                let total = zeros + ones;
                if total == 0 {
                    continue;
                }
                let measured_prob_of_1 = ones as f64 / total as f64;

                // Calculate the ideal probability of a '1' based on the scalar product.
                // The protocol adds a +32 offset to simulate starting from |+> state.
                let total_angle_offset = (angle_a as u32 + angle_b as u32 + 32) as u8 & 127;
                let angle_rad = (total_angle_offset as f64 / 128.0) * PI;
                let ideal_prob_of_1 = angle_rad.sin().powi(2);

                // The final probability of a '1' is P(final=1) = P(ideal=1)*(1-qber) + P(ideal=0)*qber.
                // We can rearrange this to solve for the error rate, qber:
                // qber = (P(final=1) - P(ideal=1)) / (1 - 2*P(ideal=1))
                // The denominator is cos(2*angle), which is zero when the ideal probability is 0.5.
                if (ideal_prob_of_1 - 0.5).abs() < 1e-9 {
                    // Case: Incompatible bases (e.g., 45 degrees).
                    // The ideal outcome is 50/50 random, so applying a bit-flip error
                    // results in a final probability that is still 50/50.
                    // We cannot measure qber here, but we can verify the 50/50 outcome.
                    println!(
                        "    - Angles(A:{:2}, B:{:2}) -> P(1) measured: {:.4}, theoretical: 0.5000 (qber is not measurable)",
                        angle_a, angle_b, measured_prob_of_1
                    );
                    assert!(
                        (measured_prob_of_1 - 0.5).abs() < 0.02,
                        "For 45-degree angles, probability of 1 should be 0.5"
                    );
                } else {
                    // Case: All other bases.
                    // We can calculate the measured error rate and compare it to the configured qber.
                    let measured_qber =
                        (measured_prob_of_1 - ideal_prob_of_1) / (1.0 - 2.0 * ideal_prob_of_1);
                    println!(
                        "    - Angles(A:{:2}, B:{:2}) -> Measured error rate: {:.4}",
                        angle_a, angle_b, measured_qber
                    );
                    assert!(
                        (measured_qber - qber).abs() < 0.02,
                        "Measured error rate should match configured QBER for angles ({}, {})",
                        angle_a,
                        angle_b
                    );
                }
            }
        }
    }
}
