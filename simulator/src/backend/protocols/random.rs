#[cfg(test)]
mod tests {
    use configs::backend::{QberConfig, SourceSettingOffset};
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;
    use sim_lib::hardware::modes::SimulatorMode;
    use sim_lib::hardware::modulator_state::ModulatorState;
    use sim_lib::simulation::builder::SimulatorBuilder;
    use sim_lib::BATCH;
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
            let source_setting_offset = SourceSettingOffset::QuarterTurn;
            let num_events: usize = 100_000;

            let mut sim_a = SimulatorBuilder::new()
                .with_rng(Pcg64Mcg::seed_from_u64(seed))
                .with_mode(SimulatorMode::Source)
                .with_qb_err(QberConfig::Fixed { value: qber })
                .with_angles(test_angles.clone())
                .with_source_setting_offset(source_setting_offset)
                .with_modulator_state(ModulatorState::Random)
                .build();

            let mut sim_b = SimulatorBuilder::new()
                .with_rng(Pcg64Mcg::seed_from_u64(seed))
                .with_mode(SimulatorMode::Detector)
                .with_qb_err(QberConfig::Fixed { value: qber })
                .with_angles(test_angles.clone())
                .with_source_setting_offset(source_setting_offset)
                .with_modulator_state(ModulatorState::Random)
                .build();

            // 2. EXECUTION & DATA GATHERING
            let mut correlation_stats: HashMap<(u8, u8), (u32, u32)> = HashMap::new();
            for batch_idx in 0..num_events.div_ceil(BATCH) {
                let batch_a = sim_a.generate_correlation_batch().unwrap();
                let batch_b = sim_b.generate_correlation_batch().unwrap();
                let events_in_batch = BATCH.min(num_events - batch_idx * BATCH);

                for i in 0..events_in_batch {
                    let result = batch_a.results[i];
                    assert_eq!(result, batch_b.results[i], "Results must be identical");

                    let angle_a = test_angles[batch_a.alice_state_index[i] as usize];
                    let angle_b = test_angles[batch_b.bob_state_index[i] as usize];

                    let stats = correlation_stats
                        .entry((angle_a, angle_b))
                        .or_insert((0, 0));
                    if !result {
                        stats.0 += 1;
                    } else {
                        stats.1 += 1;
                    }
                }
            }

            // 3. VERIFICATION
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
                let total_angle_offset =
                    (angle_a as u32 + angle_b as u32 + source_setting_offset.phase_steps() as u32)
                        as u8
                        & 127;
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
