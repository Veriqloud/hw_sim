#![allow(unused_imports)]

use core::time;
use std::{thread, time::Instant};

use itertools::izip;

use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;

use crate::backend::{
    protocols::random::CorrelationsRandom,
    role::{Multiparty, Role, SimulatorMode}, // SimulatorMode is correctly imported here
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
        .with_mode(SimulatorMode::Detector) // Added mode
        .with_eta(1e-2)
        .with_qb_err(0 as f64)
        .with_role(Role::OneOfMany(Multiparty {
            number_of_parties: 3,
            position: 0,
        }))
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .with_now(now)
        .build();
    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_rng(Pcg64Mcg::seed_from_u64(42))
        .with_mode(SimulatorMode::Detector) // Added mode
        .with_eta(1e-2)
        .with_qb_err(0 as f64)
        .with_role(Role::OneOfMany(Multiparty {
            number_of_parties: 3,
            position: 0,
        }))
        .with_angles(vec![0, 32, 64, 96])
        .with_modulator_state(ModulatorState::Random)
        .with_now(now)
        .build();

    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();

    // Batch 1
    let gcr_a1 = sim_a.generate_gcr_and_angles_batch().await.unwrap();
    let angles_a1 = sim_a.retrieve_pending_angles_batch(vec![]).unwrap(); // Dummy GCs

    let gcr_b1 = sim_b.generate_gcr_and_angles_batch().await.unwrap();
    let angles_b1 = sim_b.retrieve_pending_angles_batch(vec![]).unwrap(); // Dummy GCs

    assert_eq!(gcr_a1, gcr_b1, "GCR data for batch 1 should be identical");
    assert_eq!(
        angles_a1, angles_b1,
        "Angle data for batch 1 should be identical"
    );

    // Batch 2
    let gcr_a2 = sim_a.generate_gcr_and_angles_batch().await.unwrap();
    let angles_a2 = sim_a.retrieve_pending_angles_batch(vec![]).unwrap(); // Dummy GCs

    let gcr_b2 = sim_b.generate_gcr_and_angles_batch().await.unwrap();
    let angles_b2 = sim_b.retrieve_pending_angles_batch(vec![]).unwrap(); // Dummy GCs

    assert_eq!(gcr_a2, gcr_b2, "GCR data for batch 2 should be identical");
    assert_eq!(
        angles_a2, angles_b2,
        "Angle data for batch 2 should be identical"
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
        .with_mode(SimulatorMode::Source) // Example: Alice is Source
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(vec![0, 32, 64, 96])
        .with_role(Role::OneOfMany(Multiparty {
            number_of_parties: 3,
            position: 0,
        }))
        .build();
    let mut sim_b = SimulatorBuilder::new()
        .with_hardware(hw.clone())
        .with_mode(SimulatorMode::Detector) // Example: Bob is Detector
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(vec![0, 32, 64, 96])
        .with_role(Role::OneOfMany(Multiparty {
            number_of_parties: 3,
            position: 1,
        }))
        .build();
    let mut sim_c = SimulatorBuilder::new()
        .with_hardware(hw)
        .with_mode(SimulatorMode::Detector) // Example: Charlie is Detector
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(vec![0, 32, 64, 96])
        .with_role(Role::OneOfMany(Multiparty {
            number_of_parties: 3,
            position: 2,
        }))
        .build();

    println!(
        "Simulator A initial GC (not directly used for seeding in new model): {:?}",
        sim_a.global_counter // This is the initial GC from builder, start_session will reset it.
    );

    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();
    sim_c.start_session().unwrap();

    let mut angles_a_all = Vec::new();
    let mut results_a_all = Vec::new();
    let mut angles_b_all = Vec::new();
    let mut results_b_all = Vec::new();
    let mut angles_c_all = Vec::new();
    let mut results_c_all = Vec::new();

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

        let gcr_c = sim_c.generate_gcr_and_angles_batch().await.unwrap();
        angles_c_all.extend(sim_c.retrieve_pending_angles_batch(vec![]).unwrap());
        results_c_all.extend(gcr_c.iter().map(extract_result_from_gcr));
    }

    // go idle
    sim_a.stop_session().unwrap();
    sim_b.stop_session().unwrap();
    sim_c.stop_session().unwrap();

    // Restart sessions
    sim_a.start_session().unwrap();
    sim_b.start_session().unwrap();
    sim_c.start_session().unwrap();

    // read one more batch
    let gcr_a = sim_a.generate_gcr_and_angles_batch().await.unwrap();
    angles_a_all.extend(sim_a.retrieve_pending_angles_batch(vec![]).unwrap());
    results_a_all.extend(gcr_a.iter().map(extract_result_from_gcr));

    let gcr_b = sim_b.generate_gcr_and_angles_batch().await.unwrap();
    angles_b_all.extend(sim_b.retrieve_pending_angles_batch(vec![]).unwrap());
    results_b_all.extend(gcr_b.iter().map(extract_result_from_gcr));

    let gcr_c = sim_c.generate_gcr_and_angles_batch().await.unwrap();
    angles_c_all.extend(sim_c.retrieve_pending_angles_batch(vec![]).unwrap());
    results_c_all.extend(gcr_c.iter().map(extract_result_from_gcr));

    // analyze statistics
    assert_eq!(angles_a_all.len(), angles_b_all.len());
    assert_eq!(angles_b_all.len(), angles_c_all.len());
    assert_eq!(results_a_all.len(), angles_a_all.len());
    assert_eq!(results_b_all.len(), angles_b_all.len());
    assert_eq!(results_c_all.len(), angles_c_all.len());

    let l = angles_a_all.len();
    println!("length : {}", l);
    let mut num_correct = 0;
    let mut num_basismatch = 0;
    let mut num_result_matching = 0;

    // loop through angles and results of alice, bob, and charlie
    for i in 0..l {
        let res_a = results_a_all[i];
        let res_b = results_b_all[i];
        let res_c = results_c_all[i];

        let angle_a = angles_a_all[i];
        let angle_b = angles_b_all[i];
        let angle_c = angles_c_all[i];

        num_result_matching += ((res_a == res_b) && (res_a == res_c)) as u32;

        // The old test's angle logic:
        // let r = e1 & 0b1; (result bit)
        // let angle = ((e1 >> 1) as u32 + (e2 >> 1) as u32 + (e3 >> 1) as u32) % 128;
        // Here, (e1 >> 1) was the angle part. Now, angle_a, angle_b, angle_c are the angle parts.
        // The result bit is res_a (assuming all parties get the same result bit if bases match).
        let r = res_a; // Use Alice's result bit for basis match check
        let combined_angle_info = (angle_a as u32 + angle_b as u32 + angle_c as u32) % 128;

        if combined_angle_info == 0 {
            // Basis match condition from old test
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
