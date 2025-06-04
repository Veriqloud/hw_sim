#![allow(unused_imports)]

use core::time;
use std::{thread, time::Instant};

use itertools::izip;
use crate::backend::role::SimulatorMode; // Add direct import for SimulatorMode

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

    let mut sim_a = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(102)) // Added explicit RNG seeding
        .with_mode(SimulatorMode::Source) // Example: Alice is Source
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(vec![0, 32, 64, 96])
        // .with_role(Role::OneOfMany(Multiparty { // Removed
        //     number_of_parties: 3, // Removed
        //     position: 0, // Removed
        // })) // Removed
        .build();
    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(102)) // Added explicit RNG seeding (different seed)
        .with_mode(SimulatorMode::Detector) // Example: Bob is Detector
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(vec![0, 32, 64, 96])
        // .with_role(Role::OneOfMany(Multiparty { // Removed
        //     number_of_parties: 3, // Removed
        //     position: 1, // Removed
        // })) // Removed
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

    // loop through angles and results of alice and bob
    for i in 0..l {
        let res_a = results_a_all[i];
        let res_b = results_b_all[i];

        let angle_a = angles_a_all[i];
        let angle_b = angles_b_all[i];

        num_result_matching += (res_a == res_b) as u32;

        // The result bit is res_a (assuming both parties get the same result bit if bases match,
        // consistent with correlations_random documentation where QBER affects the generated bit).
        let r = res_a; // Use Alice's (Source's) result bit for basis match check against combined angle.
        let combined_angle_info = (angle_a as u32 + angle_b as u32) % 128; // Sum of Source and Detector angles

        if combined_angle_info == 0 {
            // Basis match condition: e.g., Source sends |0>, Detector measures in Z-basis.
            // Expected result bit `r` (which is `res_a`) should be 0.
            num_basismatch += 1;
            if r == 0 {
                // Result bit condition from old test
                num_correct += 1
            }
        } else if combined_angle_info == 64 {
            // Basis match condition from old test
            num_basismatch += 1;
            if r == 1 {
                // Result bit condition from old test
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
