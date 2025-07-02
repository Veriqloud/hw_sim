#![allow(unused_imports)]

use core::time;
use std::{collections::HashMap, f64::consts::PI, thread, time::Instant};

use crate::backend::role::SimulatorMode;

use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;

use crate::backend::{
    protocols::random::CorrelationsRandom,
    // role::{Multiparty, Role}, // Removed Multiparty and Role
    // role::SimulatorMode, // Keep SimulatorMode - This line is removed as it's imported above
    simulation::{
        builder::SimulatorBuilder,
        hardware::{builder::HardwareBuilder, modulator_state::ModulatorState},
        Simulator, VqSim,
    },
};

#[tokio::test]
async fn generate_bytes() {
    // test correctness of consecutive calls to correlations_random
    let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
    let now = Instant::now();
    let mut sim_a: Simulator = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(42))
        .with_mode(SimulatorMode::Source) // Added mode
        .with_eta(1e-2)
        .with_qb_err(0 as f64)
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .with_now(now)
        .with_gcr_padding(false)
        .build();
    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(42))
        .with_mode(SimulatorMode::Source) // Changed to Source for identical comparison
        .with_eta(1e-2)
        .with_qb_err(0 as f64)
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .with_now(now)
        .with_gcr_padding(false)
        .build();

    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();

    // Batch 1
    let gcr_a1_raw = sim_a.generate_gcr_and_angles_batch().await.unwrap();
    let angles_a1 = sim_a.retrieve_pending_angles_batch(vec![]).unwrap();

    let gcr_b1_raw = sim_b.generate_gcr_and_angles_batch().await.unwrap();
    let angles_b1 = sim_b.retrieve_pending_angles_batch(vec![]).unwrap();

    // Helper to decode GCR into result bits.
    // (buf_gcr[6] >> 1) & 1 extracts the result bit encoded by Simulator::encode_gcr
    let extract_result = |gcr_item: &[u8; 8]| (gcr_item[6] >> 1) & 1;

    let results_a1: Vec<u8> = gcr_a1_raw.iter().map(|gcr| extract_result(gcr)).collect();
    let results_b1: Vec<u8> = gcr_b1_raw.iter().map(|gcr| extract_result(gcr)).collect();

    assert_eq!(
        angles_a1, angles_b1,
        "Angle data for batch 1 should be identical"
    );
    assert_eq!(
        results_a1, results_b1,
        "Result bits for batch 1 should be identical"
    );

    // Batch 2
    let gcr_a2_raw = sim_a.generate_gcr_and_angles_batch().await.unwrap();
    let angles_a2 = sim_a.retrieve_pending_angles_batch(vec![]).unwrap();

    let gcr_b2_raw = sim_b.generate_gcr_and_angles_batch().await.unwrap();
    let angles_b2 = sim_b.retrieve_pending_angles_batch(vec![]).unwrap();

    let results_a2: Vec<u8> = gcr_a2_raw.iter().map(|gcr| extract_result(gcr)).collect();
    let results_b2: Vec<u8> = gcr_b2_raw.iter().map(|gcr| extract_result(gcr)).collect();

    assert_eq!(
        angles_a2, angles_b2,
        "Angle data for batch 2 should be identical"
    );
    assert_eq!(
        results_a2, results_b2,
        "Result bits for batch 2 should be identical"
    );

    sim_a.stop_session().unwrap();
    sim_b.stop_session().unwrap();
}

#[tokio::test]
async fn source_angle_generation_consistency() {
    let seed = 12345;
    let common_angles = vec![0, 32, 64, 96];
    let hw_config = HardwareBuilder::new().with_pulse_distance(1e-9).build();

    // Simulator 1: Using generate_gcr_and_angles_batch flow
    let mut sim_gcr_source = SimulatorBuilder::new()
        .with_hardware(hw_config.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Source)
        .with_eta(1.0) // Eta doesn't affect angle generation itself
        .with_qb_err(0.0) // QBER doesn't affect angle choice
        .with_angles(common_angles.clone())
        .with_modulator_state(ModulatorState::Random)
        .with_gcr_padding(false)
        .build();

    sim_gcr_source.start_session().unwrap();
    let _gcr_data = sim_gcr_source
        .generate_gcr_and_angles_batch()
        .await
        .unwrap();
    let angles_from_gcr_flow = sim_gcr_source
        .retrieve_pending_angles_batch(vec![]) // Dummy GCs, not used by retrieve
        .unwrap();
    sim_gcr_source.stop_session().unwrap();

    // Simulator 2: Using generate_angles_for_gcs flow
    let mut sim_direct_angles_source = SimulatorBuilder::new()
        .with_hardware(hw_config.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed)) // Same seed
        .with_mode(SimulatorMode::Source) // Same mode
        .with_eta(1.0)
        .with_qb_err(0.0)
        .with_angles(common_angles.clone())
        .with_modulator_state(ModulatorState::Random)
        .build();

    sim_direct_angles_source.start_session().unwrap();
    // Create a dummy vector of GCs with the expected batch size.
    // The actual GC values don't influence random angle generation in generate_angles_for_gcs.
    let dummy_gcs: Vec<u64> = (0..angles_from_gcr_flow.len() as u64).collect();
    let angles_from_direct_flow = sim_direct_angles_source
        .generate_angles_for_gcs(dummy_gcs)
        .await
        .unwrap();
    sim_direct_angles_source.stop_session().unwrap();

    assert_eq!(
        angles_from_gcr_flow.len(),
        angles_from_direct_flow.len(),
        "Angle batches should have the same length"
    );
    assert_eq!(
        angles_from_gcr_flow, angles_from_direct_flow,
        "Angles generated by both flows for a Source simulator should be identical given the same seed and config"
    );
}

#[tokio::test]
async fn qkd_statistics_asymmetric_workflow_ok() {
    // This test verifies that for any given combination of angles, the measured
    // deviation from the ideal quantum result matches the configured `qb_err`.
    // The `qb_err` is modeled as a simple bit-flip probability.

    let qb_err = 0.05;
    let hw = HardwareBuilder::new().with_pulse_distance(1e-9).build();
    let test_config_angles = vec![0u8, 32u8, 64u8, 96u8];
    let seed = 102;

    let mut sim_a = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Source)
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(test_config_angles.clone())
        .with_gcr_padding(false)
        .build();

    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Detector)
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(test_config_angles.clone())
        .with_gcr_padding(false)
        .build();

    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();

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
        let gcr_b_batch = sim_b.generate_gcr_and_angles_batch().await.unwrap();
        let angles_b_batch = sim_b.retrieve_pending_angles_batch(vec![]).unwrap();

        let mut gcs_for_alice = Vec::with_capacity(gcr_b_batch.len());
        let mut current_results_b = Vec::with_capacity(gcr_b_batch.len());

        for gcr in gcr_b_batch.iter() {
            let (gc, result) = split_gcr(gcr);
            gcs_for_alice.push(gc);
            current_results_b.push(result);
        }

        let angles_a_batch = sim_a.generate_angles_for_gcs(gcs_for_alice).await.unwrap();

        angles_a_all.extend(angles_a_batch);
        angles_b_all.extend(angles_b_batch);
        results_b_all.extend(current_results_b);
    }

    sim_a.stop_session().unwrap();
    sim_b.stop_session().unwrap();

    // --- Data Gathering ---
    let l = results_b_all.len();
    let mut correlation_stats: HashMap<(u8, u8), (u32, u32)> = HashMap::new();
    let angle_map = &test_config_angles;

    println!("SIZE:ANGLES_A_ALL ALICE {}", &angles_a_all.len());

    println!("SIZE:ANGLES_B_ALL BOB {}", &angles_b_all.len());

    println!("SIZE:RESULTS BOB {}", &results_b_all.len());

    for i in 0..l {
        let result = results_b_all[i];
        let angle_idx_a = angles_a_all[i];
        let angle_idx_b = angles_b_all[i];
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

        // The protocol adds a +32 offset to simulate starting from |+> state.
        let total_angle_offset = (angle_a as u32 + angle_b as u32 + 32) as u8 & 127;
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
