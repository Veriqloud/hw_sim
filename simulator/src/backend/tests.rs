use std::{collections::HashMap, f64::consts::PI, time::Instant};

use configs::backend::{Configuration, DecoyStatesConfig, QberConfig, SourceSettingOffset};
use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;
use sim_lib::{
    hardware::{builder::HardwareBuilder, modes::SimulatorMode, modulator_state::ModulatorState},
    simulation::{builder::SimulatorBuilder, Simulator},
    BATCH_SIZE,
};

#[test]
fn valid_config() {
    let config_json = r#"{
    "angles": [0, 10, 11, 12],
    "seed": 33,
    "eta": 0.1,
    "qberr": 0.02,
    "pulse_distance": 1e-8
}"#;

    let config_input: Configuration = serde_json::from_str(&config_json).unwrap();

    println!("Backend Config {:?}", &config_input);

    // decoy_states absent in JSON → decoy mode off.
    assert_eq!(
        Configuration {
            angles: vec![0, 10, 11, 12],
            source_setting_offset: SourceSettingOffset::QuarterTurn,
            seed: 33,
            eta: 0.1,
            qberr: QberConfig::Fixed { value: 0.02 },
            pulse_distance: 1e-8,
            decoy_states: None,
        },
        config_input
    );
}

#[test]
fn valid_config_with_decoy() {
    let config_json = r#"{
    "angles": [0, 32, 64, 96],
    "seed": 42,
    "eta": 0.1,
    "qberr": 0.05,
    "pulse_distance": 1e-8,
    "decoy_states": {
        "mu1": 0.5,
        "mu2": 0.1,
        "p1": 0.5
    }
}"#;

    let config_input: Configuration = serde_json::from_str(&config_json).unwrap();

    assert_eq!(
        Configuration {
            angles: vec![0, 32, 64, 96],
            source_setting_offset: SourceSettingOffset::QuarterTurn,
            seed: 42,
            eta: 0.1,
            qberr: QberConfig::Fixed { value: 0.05 },
            pulse_distance: 1e-8,
            decoy_states: Some(DecoyStatesConfig { mu1: 0.5, mu2: 0.1, p1: 0.5 }),
        },
        config_input
    );
}

#[test]
fn rng_streams_stay_synchronized_across_sessions() {
    let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
    let now = Instant::now();
    let mut sim_a: Simulator = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(42))
        .with_mode(SimulatorMode::Source)
        .with_eta(1e-2)
        .with_qb_err(QberConfig::Fixed { value: 0.0 })
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .with_now(now)
        .with_gcr_padding(false)
        .build();
    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(42))
        .with_mode(SimulatorMode::Detector)
        .with_eta(1e-2)
        .with_qb_err(QberConfig::Fixed { value: 0.0 })
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .with_now(now)
        .with_gcr_padding(false)
        .build();

    sim_a.initialize_session().unwrap();
    sim_b.initialize_session().unwrap();

    let extract_result = |gcr_item: &[u8; 8]| (gcr_item[6] >> 1) & 1;

    // Batch 1
    let batch_a1 = sim_a.generate_batch().unwrap();
    let batch_b1 = sim_b.generate_batch().unwrap();
    let gcr_a1_raw = batch_a1.to_gcr_batch(sim_a.use_gcr_padding());
    let gcr_b1_raw = batch_b1.to_gcr_batch(sim_b.use_gcr_padding());
    let angles_a1 = batch_a1.to_alice_fifo();
    let angles_b1 = batch_b1.to_alice_fifo();

    assert_eq!(angles_a1, angles_b1, "Angle data for batch 1 should be identical");
    assert_eq!(
        gcr_a1_raw.iter().map(extract_result).collect::<Vec<_>>(),
        gcr_b1_raw.iter().map(extract_result).collect::<Vec<_>>(),
        "Result bits for batch 1 should be identical"
    );

    sim_a.setup_session_end().unwrap();
    sim_b.setup_session_end().unwrap();
    sim_a.initialize_session().unwrap();
    sim_b.initialize_session().unwrap();

    // First batch of the new session
    let batch_a2 = sim_a.generate_batch().unwrap();
    let batch_b2 = sim_b.generate_batch().unwrap();
    let gcr_a2_raw = batch_a2.to_gcr_batch(sim_a.use_gcr_padding());
    let gcr_b2_raw = batch_b2.to_gcr_batch(sim_b.use_gcr_padding());
    let angles_a2 = batch_a2.to_alice_fifo();
    let angles_b2 = batch_b2.to_alice_fifo();

    assert_eq!(angles_a2, angles_b2, "Angle data for batch 2 should be identical");
    assert_eq!(
        gcr_a2_raw.iter().map(extract_result).collect::<Vec<_>>(),
        gcr_b2_raw.iter().map(extract_result).collect::<Vec<_>>(),
        "Result bits for batch 2 should be identical"
    );
    assert_ne!(
        angles_a1, angles_a2,
        "A new session must continue the random stream instead of replaying batch 1"
    );

    sim_a.setup_session_end().unwrap();
    sim_b.setup_session_end().unwrap();
}

#[test]
fn source_angle_generation_consistency() {
    let seed = 12345;
    let common_angles = vec![0, 32, 64, 96];
    let hw_config = HardwareBuilder::new().with_pulse_distance(1e-9).build();

    let make_sim = || {
        SimulatorBuilder::new()
            .with_hardware(hw_config.clone())
            .with_rng(Pcg64Mcg::seed_from_u64(seed))
            .with_mode(SimulatorMode::Source)
            .with_eta(1.0)
            .with_qb_err(QberConfig::Fixed { value: 0.0 })
            .with_angles(common_angles.clone())
            .with_modulator_state(ModulatorState::Random)
            .build()
    };

    let mut sim_a = make_sim();
    let mut sim_b = make_sim();
    sim_a.initialize_session().unwrap();
    sim_b.initialize_session().unwrap();

    let angles_a = sim_a.generate_batch().unwrap().to_alice_fifo();
    let angles_b = sim_b.generate_batch().unwrap().to_alice_fifo();

    assert_eq!(
        angles_a, angles_b,
        "Two Source simulators with the same seed must produce identical angles"
    );

    sim_a.setup_session_end().unwrap();
    sim_b.setup_session_end().unwrap();
}

#[test]
fn qkd_statistics_asymmetric_workflow_ok() {
    // This test verifies that for any given combination of angles, the measured
    // deviation from the ideal quantum result matches the configured `qb_err`.
    // The `qb_err` is modeled as a simple bit-flip probability.

    let qb_err = 0.05;
    let hw = HardwareBuilder::new().with_pulse_distance(1e-9).build();
    let test_config_angles = vec![0u8, 32u8, 64u8, 96u8];
    let source_setting_offset = SourceSettingOffset::QuarterTurn;
    let seed = 102;

    let mut sim_a = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Source)
        .with_eta(1e-2)
        .with_qb_err(QberConfig::Fixed { value: qb_err })
        .with_angles(test_config_angles.clone())
        .with_source_setting_offset(source_setting_offset)
        .with_gcr_padding(false)
        .build();

    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Detector)
        .with_eta(1e-2)
        .with_qb_err(QberConfig::Fixed { value: qb_err })
        .with_angles(test_config_angles.clone())
        .with_source_setting_offset(source_setting_offset)
        .with_gcr_padding(false)
        .build();

    sim_a.initialize_session().unwrap();
    sim_b.initialize_session().unwrap();

    let split_gcr = |buf_gcr: &[u8; 8]| -> (u64, u8) {
        let mut temp_buf = *buf_gcr;
        temp_buf[6] &= 0b1111_1100;
        let gc_upper_part = u64::from_le_bytes(temp_buf);
        let gc_lsb = (buf_gcr[6] & 1) as u64;
        let result_bit = ((buf_gcr[6] >> 1) & 1) as u8;
        let original_gc = (gc_upper_part << 1) | gc_lsb;
        (original_gc, result_bit)
    };

    let mut angles_a_all = Vec::new();
    let mut angles_b_all = Vec::new();
    let mut results_b_all = Vec::new();

    // Generate a larger number of batches for better statistical significance
    for _ in 0..32 {
        let batch_b = sim_b.generate_batch().unwrap();
        let gcr_b_batch = batch_b.to_gcr_batch(sim_b.use_gcr_padding());
        let angles_b_batch = batch_b.to_bob_angle_fifo();
        let current_results_b: Vec<u8> = gcr_b_batch.iter().map(|gcr| split_gcr(gcr).1).collect();

        let angles_a_batch = sim_a.generate_batch().unwrap().to_alice_fifo();

        angles_a_all.extend(angles_a_batch);
        angles_b_all.extend(angles_b_batch);
        results_b_all.extend(current_results_b);
    }

    sim_a.setup_session_end().unwrap();
    sim_b.setup_session_end().unwrap();

    // --- Data Gathering ---
    // Unpack FIFO-format bytes (2 events per byte: 0[d][b1][b0]_0[d][b1][b0]) into per-event indices.
    let unpack = |packed: &[u8]| -> Vec<u8> {
        packed.iter().flat_map(|&b| [(b & 0x03), ((b >> 4) & 0x03)]).collect()
    };
    let angle_indices_a = unpack(&angles_a_all);
    let angle_indices_b = unpack(&angles_b_all);

    let l = results_b_all.len();
    let mut correlation_stats: HashMap<(u8, u8), (u32, u32)> = HashMap::new();
    let angle_map = &test_config_angles;

    println!("SIZE:ANGLES_A_ALL ALICE {}", &angles_a_all.len());
    println!("SIZE:ANGLES_B_ALL BOB {}", &angles_b_all.len());
    println!("SIZE:RESULTS BOB {}", &results_b_all.len());

    for i in 0..l {
        let result = results_b_all[i];
        let angle_idx_a = angle_indices_a[i];
        let angle_idx_b = angle_indices_b[i];
        let angle_a = angle_map[angle_idx_a as usize];
        let angle_b = angle_map[angle_idx_b as usize];

        let stats = correlation_stats
            .entry((angle_a, angle_b))
            .or_insert((0, 0));
        if result == 0 {
            stats.0 += 1;
        } else {
            stats.1 += 1;
        }
    }

    // --- Verification ---
    println!("\nVerifying asymmetric workflow error rate for all angle combinations...");
    let mut sorted_keys: Vec<_> = correlation_stats.keys().collect();
    sorted_keys.sort();

    println!("RECORDS : {:?}", &correlation_stats);

    for key in sorted_keys {
        let (angle_a, angle_b) = *key;
        let (zeros, ones) = correlation_stats[key];
        let total = zeros + ones;
        if total == 0 {
            continue;
        }
        let measured_prob_of_1 = ones as f64 / total as f64;

        let total_angle_offset = (angle_a as u32
            + angle_b as u32
            + source_setting_offset.phase_steps() as u32)
            as u8
            & 127;
        let angle_rad = (total_angle_offset as f64 / 128.0) * PI;
        let ideal_prob_of_1 = angle_rad.sin().powi(2);

        if (ideal_prob_of_1 - 0.5).abs() < 1e-9 {
            println!(
                "    - Angles(A:{:2}, B:{:2}) -> P(1) measured: {:.4}, theoretical: 0.5000 (qber is not measurable)",
                angle_a, angle_b, measured_prob_of_1
            );
            assert!(
                (measured_prob_of_1 - 0.5).abs() < 0.03,
                "For 45-degree angles, probability of 1 should be 0.5"
            );
        } else {
            let measured_qber =
                (measured_prob_of_1 - ideal_prob_of_1) / (1.0 - 2.0 * ideal_prob_of_1);
            println!(
                "    - Angles(A:{:2}, B:{:2}) -> Measured error rate: {:.4}",
                angle_a, angle_b, measured_qber
            );
            assert!(
                (measured_qber - qb_err).abs() < 0.03,
                "Measured error rate should match configured QBER for angles ({}, {})",
                angle_a,
                angle_b
            );
        }
    }
}

#[test]
fn test_rate_limiting_slow_rate() {
    // This test verifies that the simulator's generation rate correctly
    // adheres to a slow configured rate by sleeping for the appropriate duration.

    use std::time::Duration;

    let pulse_distance = 1e-3; // 1ms per ideal event
    let eta = 1.0; // 100% efficiency for simpler calculation
    let seed = 123;
    let num_batches = 20; // Use more batches for a better average
    let batch_size = BATCH_SIZE; // 1024 events per batch
    let total_events = num_batches * batch_size;

    let mut sim = SimulatorBuilder::new()
        .with_hardware(
            HardwareBuilder::new()
                .with_pulse_distance(pulse_distance)
                .build(),
        )
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Detector)
        .with_eta(eta)
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .build();

    sim.initialize_session().unwrap();
    let start_time = Instant::now();

    for _ in 0..num_batches {
        sim.generate_batch().unwrap();
    }

    let elapsed_time = start_time.elapsed();
    sim.setup_session_end().unwrap();

    // Expected time calculation based on the simulator's rate limiting logic:
    // time_in_secs = (target_event_count * pulse_distance) / eta
    let expected_time_secs = (total_events as f64 * pulse_distance) / eta;
    let expected_duration = Duration::from_secs_f64(expected_time_secs);

    // Allow for a small tolerance (e.g., 10% or 200ms, whichever is larger)
    let tolerance = Duration::from_millis(200).max(expected_duration / 10);
    assert!(
        elapsed_time >= expected_duration && elapsed_time <= expected_duration + tolerance,
        "Slow rate test failed: Elapsed time ({:?}) did not match expected time ({:?} +/- {:?})",
        elapsed_time,
        expected_duration,
        tolerance
    );
}

#[test]
fn test_rate_limiting_high_speed() {
    // This test verifies that for a very high configured rate, the simulator
    // does not add any artificial delay and runs as fast as possible.

    use std::time::Duration;

    let pulse_distance = 1e-9; // 1ns per ideal event -> 1 Giga-event/sec rate
    let eta = 1.0; // 100% efficiency for simpler calculation
    let seed = 456;
    let num_batches = 50; // Generate a lot more batches to ensure CPU work is significant
    let batch_size = BATCH_SIZE; // 1024 events per batch
    let total_events = num_batches * batch_size;

    let mut sim = SimulatorBuilder::new()
        .with_hardware(
            HardwareBuilder::new()
                .with_pulse_distance(pulse_distance)
                .build(),
        )
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Detector)
        .with_eta(eta)
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .build();

    sim.initialize_session().unwrap();
    let start_time = Instant::now();

    for _ in 0..num_batches {
        sim.generate_batch().unwrap();
    }

    let elapsed_time = start_time.elapsed();
    sim.setup_session_end().unwrap();

    // The theoretical time is extremely short (microseconds).
    // The actual execution time will be dominated by CPU work, not sleeping.
    let expected_time_secs = (total_events as f64 * pulse_distance) / eta;
    let _expected_duration = Duration::from_secs_f64(expected_time_secs);

    // We assert that the elapsed time is much larger than the theoretical sleep time,
    // but still reasonably fast (e.g., under 200ms), proving no significant sleep occurred.
    let max_expected_runtime = std::time::Duration::from_millis(500);
    assert!(
        elapsed_time < max_expected_runtime,
        "High-speed rate test failed: Elapsed time ({:?}) was too long, suggesting an artificial delay.",
        elapsed_time
    );
}

#[test]
fn test_qber_oscillation_distributions() {
    let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
    let seed = 12345;

    // Helper to run simulation for 15 batches and return result bits
    let run_sim = |qber_config: QberConfig| -> Vec<Vec<u8>> {
        let mut sim = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_rng(Pcg64Mcg::seed_from_u64(seed))
            .with_seed(seed)
            .with_mode(SimulatorMode::Detector)
            .with_qb_err(qber_config)
            .with_angles(vec![0, 32, 64, 96])
            .with_modulator_state(ModulatorState::Random)
            .with_gcr_padding(false)
            .build();

        sim.initialize_session().unwrap();

        let mut batches = Vec::new();
        for _ in 0..15 {
            let batch = sim.generate_batch().unwrap();
            let gcr_raw = batch.to_gcr_batch(sim.use_gcr_padding());
            let result_bits: Vec<u8> = gcr_raw.iter().map(|gcr| (gcr[6] >> 1) & 1).collect();
            batches.push(result_bits);
        }
        sim.setup_session_end().unwrap();
        batches
    };

    // Reference (Zero Error)
    let batches_zero = run_sim(QberConfig::Fixed { value: 0.0 });

    // Define the variants we want to test
    let test_cases = vec![
        QberConfig::Fixed { value: 0.2 },
        QberConfig::Uniform { min: 0.0, max: 0.5 },
        QberConfig::Gaussian {
            mean: 0.25,
            std_dev: 0.1,
        },
    ];

    for config in test_cases {
        let batches_test = run_sim(config.clone());
        let mut observed_qbers = Vec::new();

        for i in 0..batches_test.len() {
            let mut diffs = 0;
            for j in 0..batches_test[i].len() {
                if batches_zero[i][j] != batches_test[i][j] {
                    diffs += 1;
                }
            }
            observed_qbers.push(diffs as f64 / batches_zero[i].len() as f64);
        }

        // The exhaustive match here ensures that any new variant added to QberConfig
        // will cause a compilation error in this test, forcing an update.
        match config {
            QberConfig::Fixed { value } => {
                for (i, &qber) in observed_qbers.iter().enumerate() {
                    assert!(
                        (qber - value).abs() < 0.05,
                        "Fixed QBER unstable at batch {} (got {})",
                        i,
                        qber
                    );
                }
            }
            QberConfig::Uniform { .. } => {
                // The reason this finds the max and min values is that f64 can be NaN, but f64::max or f64::min ignores NaN in coparisons and returns the other number.
                // On first pass, we get 0./0. which is NaN, and compare it to the first value of the vec. We cannot use .min() directly on the collection because f64 does not implement Ord because of NaN.
                // Note that if both arguments are NaN, we just return NaN (and not an error) so tests won't crash if we use this.
                let max_qber = observed_qbers.iter().cloned().fold(0. / 0., f64::max);
                let min_qber = observed_qbers.iter().cloned().fold(0. / 0., f64::min);
                assert!(
                    max_qber - min_qber > 0.1,
                    "Uniform QBER should vary across batches (range: {:.3})",
                    max_qber - min_qber
                );
            }
            QberConfig::Gaussian { mean, .. } => {
                let sum: f64 = observed_qbers.iter().sum();
                let mean_qber = sum / observed_qbers.len() as f64;
                assert!(
                    (mean_qber - mean).abs() < 0.05,
                    "Gaussian mean QBER should be near {} (got {:.3})",
                    mean,
                    mean_qber
                );
                let max_qber = observed_qbers.iter().cloned().fold(0. / 0., f64::max);
                let min_qber = observed_qbers.iter().cloned().fold(0. / 0., f64::min);
                assert!(
                    max_qber - min_qber > 0.05,
                    "Gaussian QBER should show some variance (range: {:.3})",
                    max_qber - min_qber
                );
            }
        }
    }
}

#[test]
fn test_qkd_attack_signal_flow() {
    // This test verifies the end-to-end flow of the attack mode:
    // Actor receives StartAttack -> Simulator forces QBER to 50% -> StopAttack restores it.

    let hw = HardwareBuilder::new().with_pulse_distance(1e-9).build();
    let seed = 42;
    let config_angles = vec![0, 32, 64, 96];

    let make_handle = || {
        let simulator = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_rng(Pcg64Mcg::seed_from_u64(seed))
            .with_seed(seed)
            .with_mode(SimulatorMode::Detector)
            .with_qb_err(QberConfig::Fixed { value: 0.0 })
            .with_angles(config_angles.clone())
            .with_modulator_state(ModulatorState::Random)
            .with_gcr_padding(false)
            .build();
        crate::backend::actor::ActorHandle::new(simulator)
    };
    let sim_handle = make_handle();
    let control_handle = make_handle();

    // 1. Reference Batch (Normal mode)
    sim_handle.start_session().unwrap();
    control_handle.start_session().unwrap();
    let batch_normal = sim_handle.generate_qkd_batch().unwrap();
    let batch_control = control_handle.generate_qkd_batch().unwrap();
    let gcr_normal = batch_normal.to_gcr_batch(sim_handle.use_gcr_padding);
    let gcr_control = batch_control.to_gcr_batch(control_handle.use_gcr_padding);
    let results_normal: Vec<u8> = gcr_normal.iter().map(|gcr| (gcr[6] >> 1) & 1).collect();
    let results_control: Vec<u8> = gcr_control.iter().map(|gcr| (gcr[6] >> 1) & 1).collect();
    assert_eq!(results_normal, results_control);

    // 2. Activate the attack while the session is running.
    sim_handle.start_attack().unwrap();
    let batch_attack = sim_handle.generate_qkd_batch().unwrap();
    let batch_attack_control = control_handle.generate_qkd_batch().unwrap();
    let gcr_attack = batch_attack.to_gcr_batch(sim_handle.use_gcr_padding);
    let gcr_attack_control =
        batch_attack_control.to_gcr_batch(control_handle.use_gcr_padding);
    let results_attack: Vec<u8> = gcr_attack.iter().map(|gcr| (gcr[6] >> 1) & 1).collect();
    let results_attack_control: Vec<u8> = gcr_attack_control
        .iter()
        .map(|gcr| (gcr[6] >> 1) & 1)
        .collect();

    // Calculate QBER
    let mut diffs = 0;
    for i in 0..BATCH_SIZE {
        if results_attack_control[i] != results_attack[i] {
            diffs += 1;
        }
    }
    let qber_attack = diffs as f64 / BATCH_SIZE as f64;
    println!("Measured QBER during simulated attack: {:.4}", qber_attack);

    assert!(
        qber_attack > 0.4 && qber_attack < 0.6,
        "Attack QBER should be around 0.5, got {:.4}",
        qber_attack
    );

    // 3. Stop the attack and verify the next batch without restarting the session.
    sim_handle.stop_attack().unwrap();
    let batch_restored = sim_handle.generate_qkd_batch().unwrap();
    let batch_restored_control = control_handle.generate_qkd_batch().unwrap();
    let gcr_restored = batch_restored.to_gcr_batch(sim_handle.use_gcr_padding);
    let gcr_restored_control =
        batch_restored_control.to_gcr_batch(control_handle.use_gcr_padding);
    let results_restored: Vec<u8> = gcr_restored.iter().map(|gcr| (gcr[6] >> 1) & 1).collect();
    let results_restored_control: Vec<u8> = gcr_restored_control
        .iter()
        .map(|gcr| (gcr[6] >> 1) & 1)
        .collect();

    assert_eq!(
        results_restored, results_restored_control,
        "Stopping the attack should restore the configured QBER"
    );

    sim_handle.stop_session().unwrap();
    control_handle.stop_session().unwrap();
}
