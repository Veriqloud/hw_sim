#[cfg(test)]
pub mod tests {
    use std::{thread, time};

    use crate::role::{Multiparty, Role};
    use crate::simulator::builder::SimulatorBuilder;
    use itertools::izip;
    use libhardware::builder::HardwareBuilder;

    use crate::protocols::random::CorrelationsRandom;
    use libhardware::{Backend, HardwareError, ModulatorState};

    #[test]
    fn correlations_random() {
        // test correctness of consecutive calls to correlations_random
        let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
        let mut sim_a = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_eta(1e-2)
            .with_qb_err(5e-2)
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: 3,
                position: 0,
            }))
            .build();
        let mut sim_b = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_eta(1e-2)
            .with_qb_err(5e-2)
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: 3,
                position: 0,
            }))
            .build();

        let gc_a = sim_a.get_global_counter().unwrap() + 2000;
        let _la = sim_a
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();
        let _lb = sim_b
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();
        let (va, _leftovera) = sim_a.correlations_random(1100).unwrap();
        let (vb1, leftoverb1) = sim_b.correlations_random(1000).unwrap();
        sim_b.leftover = leftoverb1;
        let (vb2, _leftoverb2) = sim_b.correlations_random(100).unwrap();
        let mut vb = vb1.clone();
        vb.extend(vb2);

        assert_eq!(va, vb);
    }

    #[test]
    fn qkd_statistics_using_modulator_state_random() {
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
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: 3,
                position: 0,
            }))
            .build();
        let mut sim_b = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_eta(1e-2)
            .with_qb_err(qb_err)
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: 3,
                position: 1,
            }))
            .build();
        let mut sim_c = SimulatorBuilder::new()
            .with_hardware(hw)
            .with_eta(1e-2)
            .with_qb_err(qb_err)
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: 3,
                position: 2,
            }))
            .build();
        println!("gc: {:?}", sim_b.get_global_counter());

        let gc_a = sim_a.get_global_counter().unwrap() + 0;
        let _la = sim_a
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();
        let _lb = sim_b
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();
        let _lc = sim_c
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();

        // sleep and read
        thread::sleep(time::Duration::from_millis(50));
        let mut a = sim_a.read_angles().unwrap();
        let mut b = sim_b.read_angles().unwrap();
        let mut c = sim_c.read_angles().unwrap();
        println!("a: {:?}", &c[0..50]);

        // sleep and read again
        thread::sleep(time::Duration::from_millis(50));
        a.extend(sim_a.read_angles().unwrap());
        b.extend(sim_b.read_angles().unwrap());
        c.extend(sim_c.read_angles().unwrap());

        // truncate all three to the same lenth for simplicity later
        let l = a.len().min(b.len());
        let l = c.len().min(l);
        a.truncate(l);
        b.truncate(l);
        c.truncate(l);

        // go idle
        let gc_a = sim_a.get_global_counter().unwrap() + 0;
        sim_a
            .set_modulator_state(ModulatorState::Idle, gc_a)
            .unwrap();
        sim_b
            .set_modulator_state(ModulatorState::Idle, gc_a)
            .unwrap();
        sim_c
            .set_modulator_state(ModulatorState::Idle, gc_a)
            .unwrap();

        thread::sleep(time::Duration::from_millis(50));
        // start again
        let gc_a = sim_a.get_global_counter().unwrap() + 1000;
        sim_a
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();
        sim_b
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();
        sim_c
            .set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc_a)
            .unwrap();

        // read one last time
        thread::sleep(time::Duration::from_millis(150));
        a.extend(sim_a.read_angles().unwrap());
        b.extend(sim_b.read_angles().unwrap());
        c.extend(sim_c.read_angles().unwrap());

        // analyze statistics
        let l = a.len().min(b.len());
        let l = c.len().min(l);
        println!("length : {}", l);
        let mut num_correct = 0;
        let mut num_basismatch = 0;
        let mut num_result_matching = 0;

        let garbage = _la.max(_lb.max(_lc)) as usize;
        println!("garbage length : {}", garbage);
        // loop through angles of alice and bob
        for (e1, e2, e3) in izip!(
            a[garbage..l].iter(),
            b[garbage..l].iter(),
            c[garbage..l].iter(),
        ) {
            num_result_matching +=
                (((e1 & 0b1) == (e2 & 0b1)) && ((e1 & 0b1) == (e3 & 0b1))) as u32;
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
        let l = l - garbage;
        let num_errors = num_basismatch - num_correct;
        println!("error: {} correct: {}", num_errors, num_correct);
        println!("num basis match {}", num_basismatch as f64 / l as f64);
        let measured_qber = num_errors as f64 / (num_correct + num_errors) as f64;
        println!("measured qber: {}", measured_qber);
        assert_eq!(num_result_matching, l as u32);
        assert!((num_basismatch as f64 / l as f64 - 0.5).abs() < 0.01);
        assert!((measured_qber - qb_err).abs() < 0.01);
    }

    #[test]
    fn qkd_statistics() {
        // Test the following statistics for QKD simulation between two parties:
        //
        // 1. perfect correlation of the result bit
        // 2. 50% of basis match
        // 3. qber is what it is supposed to be
        // 4. two consecutive reads don't mess up correlations

        let qb_err = 0.05;
        let hw = HardwareBuilder::new().with_pulse_distance(1e-9).build();

        let mut sim_a = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_eta(1e-2)
            .with_qb_err(qb_err)
            .with_role(Role::Sender)
            .build();
        let mut sim_b = SimulatorBuilder::new()
            .with_hardware(hw)
            .with_eta(1e-2)
            .with_qb_err(qb_err)
            .with_role(Role::Receiver)
            .build();
        println!("gc: {:?}", sim_b.get_global_counter());

        let gc_a = sim_a.get_global_counter().unwrap() + 1000;
        let _la = sim_a
            .set_modulator_state(ModulatorState::Qkd, gc_a)
            .unwrap();
        let _lb = sim_b
            .set_modulator_state(ModulatorState::Qkd, gc_a)
            .unwrap();

        // sleep and read
        thread::sleep(time::Duration::from_millis(50));
        let mut a = sim_a.read_angles().unwrap();
        let mut b = sim_b.read_angles().unwrap();

        // sleep and read again
        thread::sleep(time::Duration::from_millis(50));
        a.extend(sim_a.read_angles().unwrap());
        b.extend(sim_b.read_angles().unwrap());

        // analyze statistics
        let l = a.len().min(b.len());
        println!("length : {}", l);
        let mut num_errors = 0;
        let mut num_correct = 0;
        let mut num_basismatch = 0;
        let mut num_result_unmatching = 0;

        // loop through angles of alice and bob
        for (e1, e2) in a[..l].iter().zip(b[..l].iter()) {
            num_result_unmatching += e1 & 0b1 ^ e2 & 0b1;
            // basis match
            if e1 & 0b10 == e2 & 0b10 {
                num_basismatch += 1;
                // state match
                if e1 & 0b100 == e2 & 0b100 {
                    // result correct
                    if e1 & 0b1 == 0 {
                        num_correct += 1
                    } else {
                        num_errors += 1
                    };
                } else {
                    // result correct
                    if e1 & 0b1 == 1 {
                        num_correct += 1
                    } else {
                        num_errors += 1
                    };
                }
            }
        }
        assert_eq!(num_result_unmatching, 0);
        assert!((num_basismatch as f64 / l as f64 - 0.5).abs() < 0.01);
        println!("error: {} correct: {}", num_errors, num_correct);
        let measured_qber = num_errors as f64 / (num_correct + num_errors) as f64;
        println!("measured qber: {}", measured_qber);
        assert!((measured_qber - qb_err).abs() < 0.01);
    }

    #[test]
    fn read_angles_not_empty() {
        let qb_err = 0.05;
        let hw = HardwareBuilder::new().with_pulse_distance(1e-9).build();

        let mut sim_a = SimulatorBuilder::new()
            .with_role(Role::Sender)
            .with_eta(1e-2)
            .with_qb_err(qb_err)
            .with_hardware(hw)
            .build();

        let gc_a = sim_a.get_global_counter().unwrap() + 1000;
        sim_a
            .set_modulator_state(ModulatorState::Qkd, gc_a)
            .unwrap();

        // sleep and read
        thread::sleep(time::Duration::from_millis(50));
        let mut a = sim_a.read_angles().unwrap();

        // sleep and read again
        thread::sleep(time::Duration::from_millis(50));
        a.extend(sim_a.read_angles().unwrap());

        assert!(!a.is_empty());
    }

    #[test]
    // test error for
    //
    // 1. fifo overflow
    // 2. at_global_counter overflow
    //
    fn hardware_error() {
        let dt = 1e-10;
        let hw = HardwareBuilder::new().with_pulse_distance(dt).build();

        let mut sim = SimulatorBuilder::new()
            .with_role(Role::Sender)
            .with_eta(1.0)
            .with_qb_err(0.05)
            .with_hardware(hw)
            .build();
        let time_to_fail = sim.fifo_size as f64 * dt * 1000.; // in ms
        println!("time to fail: {}", time_to_fail);
        let gc_a = sim.get_global_counter().unwrap() + 1000;
        sim.set_modulator_state(ModulatorState::Qkd, gc_a).unwrap();
        thread::sleep(time::Duration::from_millis(time_to_fail as u64 + 20));
        match sim.read_angles() {
            Ok(_) => panic!(),
            Err(e) => {
                assert_eq!(e, HardwareError::FifoOverflow);
            }
        };

        let _gc_a = sim.get_global_counter().unwrap() + 1000;
        match sim.set_modulator_state(ModulatorState::Qkd, 0) {
            Ok(_) => panic!(),
            Err(e) => {
                assert_eq!(e, HardwareError::ResetFifoAtThisGcOverflow)
            }
        }
    }

    use num_format::{Locale, ToFormattedString};
    #[test]
    fn gc() {
        let hw = HardwareBuilder::new().with_pulse_distance(1.25e-8).build();

        let mut sim = SimulatorBuilder::new()
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: 3,
                position: 0,
            }))
            .with_eta(1e-2)
            .with_qb_err(0.05)
            .with_hardware(hw)
            .build();
        let mut gc = sim.get_global_counter().unwrap();
        println!("gc = {:?}", gc);
        gc += 160_000_000; // gc + 2 sec
        println!(
            "set modulator state at gc = {:?}",
            (gc as i32).to_formatted_string(&Locale::en)
        );
        sim.set_modulator_state(ModulatorState::Random(vec![0, 32, 64, 96]), gc)
            .unwrap();
        let gc = sim.get_global_counter().unwrap();
        println!("gc = {:?}", gc);
        thread::sleep(time::Duration::from_secs(2));
        let gc = sim.get_global_counter().unwrap();
        println!("gc = {:?}", (gc as i32).to_formatted_string(&Locale::en));
        if let Ok(a) = sim.read_angles() {
            println!(
                "receive {} angles",
                (a.len() as i32).to_formatted_string(&Locale::en)
            );
        }
        let gc = sim.get_global_counter().unwrap();
        println!(
            "gc at stop = {:?}",
            (gc as i32).to_formatted_string(&Locale::en)
        );
        sim.set_modulator_state(ModulatorState::Idle, gc).unwrap();
    }
}
