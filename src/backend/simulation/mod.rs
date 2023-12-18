pub mod builder;
pub mod errors;


use crate::backend::protocols::random::CorrelationsRandom;
use crate::backend::role::{Multiparty, Role};
use libhardware::errors::HardwareError;
use libhardware::{Backend, Hardware, ModulatorState};
use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;
use std::time::Instant;

#[derive(Debug, PartialEq, Clone)]
pub struct Simulator {
    pub(crate) hw: Hardware,
    pub role: Role,
    pub(crate) rng: Pcg64Mcg,
    /// Total qubit detection efficiency
    pub eta: f64,
    /// Qubit error rate
    pub qb_err: f64,
    pub now: Instant,
    pub(crate) time_of_last_read: f64,
    /// Offset is taken care of automatically.
    /// Equivalent to Bob broadcasting his global counter in the real world.
    /// Probably not required ...
    pub(crate) global_counter: u64,
    pub(crate) modulator_state: ModulatorState,
    pub(crate) angles: Vec<u8>,
    /// Size of the physical FIFO, for realistic HardwareError, "Size" means number of bytes.
    pub(crate) fifo_size: u64,
    /// in Idle Alice remembers this many global counters upon result signal from Bob
    pub(crate) size_of_idle_fifo: u32,
    pub(crate) lfifo_initial: usize,
}

impl Backend for Simulator {
    /// Read all angles and measurement results since last read.
    ///
    /// This function will generate the right amount of states based on the real time that passed since
    /// the last call of `read_angles()` or `set_modulator_state()`.
    ///
    /// The return vector contains bytes with the encoding:
    ///
    /// - bit 0 is the measurement result
    /// - bit 1 is the basis
    /// - bit 2 is the state
    fn read_angles(&mut self) -> Result<[u8; 1024], HardwareError> {
        match &self.modulator_state {
            ModulatorState::Idle => {
                if self.lfifo_initial < 1024 {
                    Err(HardwareError::Other {
                        reason: "Not enough bytes left in the fifo !".to_string(),
                    })
                } else {
                    let v = self.correlations_random(1024).map_err(|e| {
                        println!("ERROR : {:?}", e.to_string());
                        HardwareError::Other {
                            reason: e.to_string(),
                        }
                    })?;
                    self.lfifo_initial -= 1024;
                    Ok(v.try_into().unwrap())
                }
            }
            ModulatorState::Random => {
                let current_time = self.get_current_time_with_nanos();
                tracing::debug!("Current time : {:#?}", &current_time);
                let t = current_time - self.time_of_last_read;
                tracing::debug!("Last read time : {:#?}", self.time_of_last_read);
                let l =
                    ((t / self.hw.pulse_distance - self.hw.gc_offset as f64) * self.eta) as usize;
                println!("Amount of time passed since last read: {} ", t);
                println!(
                    " pulse distance: {}, gc_offset: {}, eta: {} ",
                    self.hw.pulse_distance, self.hw.gc_offset, self.eta
                );
                println!("The Simulator is supposed to have generated : {} bytes", l);

                let size = l + self.lfifo_initial;
                tracing::debug!("Fifo size before generation: {}", self.lfifo_initial);
                tracing::debug!("Fifo size after generation: {size}");
                if size as u64 > self.fifo_size {
                    return Err(HardwareError::FifoOverflow);
                }
                if size < 1024 {
                    let n = 1024 - size;
                    let t =
                        (n as f64 / self.eta + self.hw.gc_offset as f64) * self.hw.pulse_distance;
                    println!("Need to wait t = {} to generate {} bytes", t, n);

                    // Compute the expected time to wait for the remaining bytes to be generated !
                    // t = n / key_rate = n * (eta / pulse )
                    // Wait for duration t and generate the bytes
                    // set lfifo_initial
                    return Err(HardwareError::Other {
                        reason: "Not enough bytes".to_string(),
                    });
                }
                self.time_of_last_read = current_time;
                //self.qb_err = (current_time % 7 as f64) * 0.01 + 0.02;
                let v = self.correlations_random(1024).map_err(|e| {
                    println!("ERROR: {:?}", e.to_string());
                    HardwareError::Other {
                        reason: e.to_string(),
                    }
                })?;
                self.lfifo_initial = size - 1024;
                Ok(v.try_into().unwrap())
            }
            _ => Err(HardwareError::ModulatorStateNotSupported),
        }
    }
    /// Return the current global counter
    fn get_global_counter(&mut self) -> Option<u64> {
        ((self.get_current_time_with_nanos() / self.hw.pulse_distance) as u64)
            .checked_add(self.hw.gc_offset)
    }

    fn fifo_idle(&mut self) -> Result<(), HardwareError> {
        self.modulator_state = ModulatorState::Idle;
        Ok(())
    }

    fn start_at_gc(&mut self, gc: u64) -> Result<(), HardwareError> {
        self.set_gc(gc);
        self.reset_time();
        self.modulator_state = ModulatorState::Random;
        Ok(())
    }

    fn set_angles(&mut self, angles: [u8; 8]) -> Result<(), HardwareError> {
        self.angles = angles.to_vec();
        Ok(())
    }
}

impl Simulator {
    /// Set the global counter of the simulator
    pub fn set_gc(&mut self, gc: u64) {
        self.global_counter = gc;
    }
    /// Update the Role of the simulator
    pub fn set_role(&mut self, nb_parties: u32, position: u32) {
        self.role = Role::OneOfMany(Multiparty {
            number_of_parties: nb_parties,
            position,
        });
    }
    /// Update the value of eta
    pub fn set_eta(&mut self, eta: f64) {
        self.eta = eta;
    }
    /// Update the value of qber
    pub fn set_qber(&mut self, qber: f64) {
        self.qb_err = qber;
    }
    /// Reset time to now
    pub fn reset_time(&mut self) {
        self.now = Instant::now();
    }
    /// return time elapsed since start in seconds at nanoseconds.
    fn get_current_time_with_nanos(&self) -> f64 {
        let duration = self.now.elapsed();
        duration.as_secs() as f64 + duration.subsec_nanos() as f64 * 1e-9
    }
    /// get the number of values in the fifo between time t and at_glober_counter. Return None if this value is negative.
    fn get_l(&self, t: f64, at_global_counter: u64) -> Option<u32> {
        match ((t / self.hw.pulse_distance) as u64).checked_sub(at_global_counter) {
            Some(v) => v
                .checked_sub(self.hw.gc_offset)
                .map(|v| (v as f64 * self.eta) as u32),
            None => None,
        }
    }
    /// Restart RNG with a new seed.
    fn reset_seed(&mut self, seed: u64) {
        self.rng = Pcg64Mcg::seed_from_u64(seed);
    }
}

#[cfg(test)]
pub mod tests {
    use crate::backend::role::Multiparty;
    use crate::backend::role::Role;
    use crate::backend::simulation::builder::SimulatorBuilder;
    use crate::backend::simulation::Simulator;
    use libhardware::builder::HardwareBuilder;
    use libhardware::Backend;
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;
    use std::time::Duration;
    use std::time::Instant;

    use std::thread;
    use std::time;

    #[test]
    fn test_get_time() {
        let now = Instant::now();
        thread::sleep(time::Duration::from_millis(1100));
        let sim = SimulatorBuilder::new().with_now(now).build();
        println!("ct: {:}", sim.get_current_time_with_nanos());
    }

    #[test]
    fn test_builder_defaults() {
        let now = Instant::now();
        let sim = SimulatorBuilder::new().with_now(now).build();

        assert_eq!(
            Simulator {
                hw: Default::default(),
                role: Default::default(),
                rng: Pcg64Mcg::seed_from_u64(10),
                eta: 0.0,
                fifo_size: 50_000_000,
                size_of_idle_fifo: 1_000_000,
                now,
                time_of_last_read: 0.0,
                global_counter: 0,
                qb_err: 0.0,
                modulator_state: Default::default(),
                angles: Default::default(),
                lfifo_initial: 0,
            },
            sim
        )
    }

    #[test]
    fn test_read_angles_ko() {
        let now = Instant::now();
        let mut sim = SimulatorBuilder::new().with_now(now).build();
        let res = sim.read_angles();
        assert_eq!(
            res.unwrap_err(),
            libhardware::HardwareError::Other {
                reason: "Not enough bytes".to_string()
            }
        );
    }

    #[test]
    fn test_read_angles_qkd_ok() {
        let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
        let mut sim = SimulatorBuilder::new()
            .with_role(Role::Sender)
            .with_eta(1e-2)
            .with_qb_err(0 as f64)
            .with_hardware(hw)
            .with_modulator_state(libhardware::ModulatorState::Qkd)
            .with_now(Instant::now())
            .build();
        thread::sleep(Duration::from_millis(2));
        assert!(sim.read_angles().is_ok());
        println!("SIMULATOR : {:?}", &sim);
        thread::sleep(Duration::from_millis(2));
        assert!(sim.read_angles().is_ok());
        println!("SIMULATOR : {:?}", &sim);
    }

    #[test]
    fn test_read_angles_random_ok() {
        let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
        let mut sim = SimulatorBuilder::new()
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: 3,
                position: 2,
            }))
            .with_eta(1e-2)
            .with_qb_err(0 as f64)
            .with_hardware(hw)
            .with_modulator_state(libhardware::ModulatorState::Random)
            .with_now(Instant::now())
            .build();
        thread::sleep(Duration::from_millis(2));
        assert!(sim.read_angles().is_ok());
        println!("SIMULATOR : {:?}", &sim);
    }
}
