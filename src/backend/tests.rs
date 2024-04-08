#![allow(unused_imports)]

use core::time;
use std::{thread, time::Instant};

use itertools::izip;

use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;

use crate::backend::{
    protocols::random::CorrelationsRandom,
    role::{Multiparty, Role},
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

    let va1 = sim_a.read_angles().await.unwrap();
    let vb1 = sim_b.read_angles().await.unwrap();
    assert_eq!(va1, vb1);

    let va2 = sim_a.read_angles().await.unwrap();
    let vb2 = sim_b.read_angles().await.unwrap();
    assert_eq!(va2, vb2);
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
        .with_eta(1e-2)
        .with_qb_err(qb_err)
        .with_angles(vec![0, 32, 64, 96])
        .with_role(Role::OneOfMany(Multiparty {
            number_of_parties: 3,
            position: 2,
        }))
        .build();

    println!(
        "Simulator A : {:?}",
        (sim_a.global_counter, sim_a.hw.gc_offset)
    );

    let gc_a = sim_a.get_global_counter().unwrap();
    sim_a.start_at_gc(gc_a).unwrap();
    sim_b.start_at_gc(gc_a).unwrap();
    sim_c.start_at_gc(gc_a).unwrap();

    let mut a = sim_a.read_angles().await.unwrap().to_vec();
    let mut b = sim_b.read_angles().await.unwrap().to_vec();
    let mut c = sim_c.read_angles().await.unwrap().to_vec();

    a.extend(sim_a.read_angles().await.unwrap());
    b.extend(sim_b.read_angles().await.unwrap());
    c.extend(sim_c.read_angles().await.unwrap());

    a.extend(sim_a.read_angles().await.unwrap());
    b.extend(sim_b.read_angles().await.unwrap());
    c.extend(sim_c.read_angles().await.unwrap());
    // go idle
    sim_a.fifo_idle().unwrap();
    sim_b.fifo_idle().unwrap();
    sim_c.fifo_idle().unwrap();

    let gc_a = sim_a.get_global_counter().unwrap() + 1000;
    sim_a.start_at_gc(gc_a).unwrap();
    sim_b.start_at_gc(gc_a).unwrap();
    sim_c.start_at_gc(gc_a).unwrap();

    // read several times to one last time
    a.extend(sim_a.read_angles().await.unwrap());
    b.extend(sim_b.read_angles().await.unwrap());
    c.extend(sim_c.read_angles().await.unwrap());

    // analyze statistics
    assert_eq!(a.len(), b.len());
    assert_eq!(b.len(), c.len());

    let l = a.len();
    println!("length : {}", l);
    let mut num_correct = 0;
    let mut num_basismatch = 0;
    let mut num_result_matching = 0;

    // loop through angles of alice and bob
    for (e1, e2, e3) in izip!(a.iter(), b.iter(), c.iter(),) {
        num_result_matching += (((e1 & 0b1) == (e2 & 0b1)) && ((e1 & 0b1) == (e3 & 0b1))) as u32;
        // basis match
        let r = e1 & 0b1;
        let angle = ((e1 >> 1) as u32 + (e2 >> 1) as u32 + (e3 >> 1) as u32) % 128;
        if angle == 0 {
            num_basismatch += 1;
            if r == 0 {
                num_correct += 1
            }
        } else if angle == 64 {
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
    assert!((measured_qber - qb_err).abs() < 0.02);
}
