#![allow(unused_imports)]

use core::time;
use std::{thread, time::Instant};

use crate::backend::role::SimulatorMode;
use itertools::izip; // Add direct import for SimulatorMode

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
async fn qkd_statistics_ok() {
    // Test the following statistics for QKD simulation between two parties:
    //
    // 1. perfect correlation of the result bit
    // 2. 50% of basis match
    // 3. qber is what it is supposed to be
    // 4. two consecutive reads don't mess up correlations
    // 5. going to idle and coming back does not mess up the correlations

    let qb_err = 0.05;
    let hw = HardwareBuilder::new().with_pulse_distance(1e-9).build();
    let test_config_angles = vec![0u8, 32u8, 64u8, 96u8]; // Define angles used in this test

    let mut sim_a = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(102)) // Added explicit RNG seeding
        .with_mode(SimulatorMode::Source)
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(test_config_angles.clone()) // Use defined test angles
        .build();
    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(102))
        .with_mode(SimulatorMode::Detector) // Example: Bob is Detector
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(test_config_angles.clone()) // Use defined test angles
        .build();

    println!(
        "Simulator A initial GC (not directly used for seeding in new model): {:?}",
        sim_a.global_counter // This is the initial GC from builder, start_session will reset it.
    );

    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();

    let mut angles_a_all = Vec::new();
    let mut results_a_all = Vec::new();
    let mut angles_b_all = Vec::new();
    let mut results_b_all = Vec::new();

    // Helper to decode GCR into result bits
    // The result bit is (buf_gcr[6] >> 1) & 1;
    // Our encode_gcr takes a u8 result_bit and stores (result_bit & 1) << 1 in buf[6].
    // So, to get the original (result_bit & 1), we do (gcr_item[6] >> 1) & 1.
    let extract_result_from_gcr = |gcr_item: &[u8; 8]| (gcr_item[6] >> 1) & 1;

    for _ in 0..3 {
        // Simulate 3 batches of data
        let gcr_a = sim_a.generate_gcr_and_angles_batch().await.unwrap();
        angles_a_all.extend(sim_a.retrieve_pending_angles_batch(vec![]).unwrap());
        results_a_all.extend(gcr_a.iter().map(extract_result_from_gcr));

        let gcr_b = sim_b.generate_gcr_and_angles_batch().await.unwrap();
        angles_b_all.extend(sim_b.retrieve_pending_angles_batch(vec![]).unwrap());
        results_b_all.extend(gcr_b.iter().map(extract_result_from_gcr));
    }

    // go idle
    sim_a.stop_session().unwrap();
    sim_b.stop_session().unwrap();

    // Restart sessions
    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();

    // read one more batch
    let gcr_a = sim_a.generate_gcr_and_angles_batch().await.unwrap();
    angles_a_all.extend(sim_a.retrieve_pending_angles_batch(vec![]).unwrap());
    results_a_all.extend(gcr_a.iter().map(extract_result_from_gcr));

    let gcr_b = sim_b.generate_gcr_and_angles_batch().await.unwrap();
    angles_b_all.extend(sim_b.retrieve_pending_angles_batch(vec![]).unwrap());
    results_b_all.extend(gcr_b.iter().map(extract_result_from_gcr));

    // analyze statistics
    assert_eq!(angles_a_all.len(), angles_b_all.len());
    assert_eq!(results_a_all.len(), angles_a_all.len());
    assert_eq!(results_b_all.len(), angles_b_all.len());

    let l = angles_a_all.len();
    println!("length : {}", l);
    let mut num_correct = 0;
    let mut num_basismatch = 0;
    let mut num_result_matching = 0;

    // The `test_config_angles` vec is used as the map from index to actual angle.
    let angle_map = &test_config_angles;

    // loop through angles and results of alice and bob
    for i in 0..l {
        let res_a = results_a_all[i];
        let res_b = results_b_all[i]; // res_a and res_b should be identical due to correlations_random output

        // angles_a_all[i] and angles_b_all[i] are 2-bit indices (0-3)
        // as returned by `retrieve_pending_angles_batch` which gets them from `correlations_random`
        // where `output[i] = chosen_basis_index << 1; output[i] |= result;`
        // and then `angles_data.push(byte_val >> 1);`
        let angle_idx_a = angles_a_all[i];
        let angle_idx_b = angles_b_all[i];

        // Map indices to actual angle values
        let actual_angle_a = angle_map[angle_idx_a as usize];
        let actual_angle_b = angle_map[angle_idx_b as usize];

        num_result_matching += (res_a == res_b) as u32;

        // The result bit is res_a (final result from correlations_random, QBER already applied).
        let r = res_a;
        // Calculate combined_angle_info using actual angle values
        let combined_angle_info = (actual_angle_a as u32 + actual_angle_b as u32) % 128;

        if combined_angle_info == 0 {
            // This condition implies a "basis match" for the purpose of this test's statistics.
            // Expected result for sum 0 (e.g., 0+0, 32+96, 64+64, 96+32) is 0 (cos^2(0) = 1, high probability of 0).
            num_basismatch += 1;
            if r == 0 {
                num_correct += 1;
            }
        } else if combined_angle_info == 64 {
            // This condition implies another "basis match" for statistical purposes.
            // Expected result for sum 64 (e.g. 0+64, 32+32, 64+0, 96+96) is 1 (cos^2(PI/2) = 0, high probability of 1).
            // Note: The original test logic for result bit checking (r == 1 for sum 64) is kept.
            num_basismatch += 1;
            if r == 1 {
                num_correct += 1
            }
        }
    }
    let num_errors = num_basismatch - num_correct;
    println!("error: {} correct: {}", num_errors, num_correct);
    println!("num basis match {}", num_basismatch as f64 / l as f64);
    let measured_qber = num_errors as f64 / (num_correct + num_errors) as f64;
    println!("measured qber: {}", measured_qber);
    assert_eq!(num_result_matching, l as u32);
    assert!((num_basismatch as f64 / l as f64 - 0.5).abs() < 0.02);
    assert!((measured_qber - qb_err).abs() < 0.009);
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
    // This test verifies that a simulator pair, one following the "Source" (Alice) workflow
    // and the other the "Detector" (Bob) workflow, produce correctly correlated results
    // according to BB84 statistics.

    // 1. Setup
    let qb_err = 0.05;
    let hw = HardwareBuilder::new().with_pulse_distance(1e-9).build();
    let test_config_angles = vec![0u8, 32u8, 64u8, 96u8];
    let seed = 102; // Use a specific seed for reproducibility

    // Alice (Source)
    let mut sim_a = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed))
        .with_mode(SimulatorMode::Source)
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(test_config_angles.clone())
        .build();

    // Bob (Detector)
    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(seed)) // Same seed is crucial
        .with_mode(SimulatorMode::Detector)
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(test_config_angles.clone())
        .build();

    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();

    // Helper to decode a GCR value into its constituent (u64, u8) -> (GC, result_bit)
    // This is the inverse of the `encode_gcr` method in the Simulator.
    let split_gcr = |buf_gcr: &[u8; 8]| -> (u64, u8) {
        let mut temp_buf = *buf_gcr;
        temp_buf[6] &= 0b1111_1100; // Clear the two LSBs (result bit and GC LSB)
        let gc_upper_part = u64::from_le_bytes(temp_buf); // This is the GC shifted right by 1
        let gc_lsb = (buf_gcr[6] & 1) as u64;
        let result_bit = ((buf_gcr[6] >> 1) & 1) as u8;
        let original_gc = (gc_upper_part << 1) | gc_lsb; // Reconstruct the original GC
        (original_gc, result_bit)
    };

    let mut angles_a_all = Vec::new();
    let mut angles_b_all = Vec::new();
    let mut results_b_all = Vec::new();

    // Generate a few batches of data to get good statistics
    for _ in 0..4 {
        // 2. Bob's (Detector) workflow
        let gcr_b_batch = sim_b.generate_gcr_and_angles_batch().await.unwrap();
        let angles_b_batch = sim_b.retrieve_pending_angles_batch(vec![]).unwrap();

        let mut gcs_for_alice = Vec::with_capacity(gcr_b_batch.len());
        let mut current_results_b = Vec::with_capacity(gcr_b_batch.len());

        for gcr in gcr_b_batch.iter() {
            let (gc, result) = split_gcr(gcr);
            gcs_for_alice.push(gc);
            current_results_b.push(result);
        }

        // 3. Alice's (Source) workflow
        let angles_a_batch = sim_a
            .generate_angles_for_gcs(gcs_for_alice)
            .await
            .unwrap();

        angles_a_all.extend(angles_a_batch);
        angles_b_all.extend(angles_b_batch);
        results_b_all.extend(current_results_b);
    }

    sim_a.stop_session().unwrap();
    sim_b.stop_session().unwrap();

    // 4. Verification
    assert_eq!(angles_a_all.len(), angles_b_all.len());
    assert_eq!(results_b_all.len(), angles_a_all.len());

    let l = angles_a_all.len();
    println!("asymmetric workflow length : {}", l);
    let mut num_correct = 0;
    let mut num_basismatch = 0;

    let angle_map = &test_config_angles;

    for i in 0..l {
        let res_b = results_b_all[i];
        let angle_idx_a = angles_a_all[i];
        let angle_idx_b = angles_b_all[i];

        let actual_angle_a = angle_map[angle_idx_a as usize];
        let actual_angle_b = angle_map[angle_idx_b as usize];

        let r = res_b; // The result is determined by Bob's measurement
        let combined_angle_info = (actual_angle_a as u32 + actual_angle_b as u32) % 128;

        if combined_angle_info == 0 {
            num_basismatch += 1;
            if r == 0 {
                num_correct += 1;
            }
        } else if combined_angle_info == 64 {
            num_basismatch += 1;
            if r == 1 {
                num_correct += 1
            }
        }
    }
    let num_errors = num_basismatch - num_correct;
    println!(
        "asymmetric workflow error: {} correct: {}",
        num_errors, num_correct
    );
    println!(
        "asymmetric workflow num basis match {}",
        num_basismatch as f64 / l as f64
    );
    let measured_qber = num_errors as f64 / (num_correct + num_errors) as f64;
    println!("asymmetric workflow measured qber: {}", measured_qber);
    assert!((num_basismatch as f64 / l as f64 - 0.5).abs() < 0.02);
    assert!((measured_qber - qb_err).abs() < 0.009);
}
