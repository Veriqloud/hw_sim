pub mod builder;
pub mod errors;

use crate::backend::protocols::bb84::BB84;
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
    // Leftover values we need to keep between calls of read_angles().
    pub(crate) leftover: Vec<u8>,
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
    /// Size of the physical FIFO, for realistic HardwareError, "Size" means number of bytes.
    pub(crate) fifo_size: u64,
    /// in Idle Alice remembers this many global counters upon result signal from Bob
    pub(crate) size_of_idle_fifo: u32,
    pub(crate) lfifo_initial: u32,
}

impl Backend for Simulator {
    /// Set the state of the phase modulator and reset the hardware fifo.
    ///
    /// The reset will happen
    /// at_global_counter. Make sure that at_global_counter is the same across the two parties.
    /// Otherwise there might be an offset in the fifo. If at_global_counter lies in the past, vec will
    /// be padded with zeros at its beginning. The number of zeros is returned here.
    ///
    /// This function should exist for the real hardware with the same arguments and behaviour.
    ///
    /// Errors are not included yet.
    fn set_modulator_state(
        &mut self,
        modulator_state: ModulatorState,
        at_global_counter: u64,
    ) -> Result<u32, HardwareError> {
        self.reset_seed(at_global_counter);
        self.leftover.clear();
        self.lfifo_initial = 0;
        //if !matches!(modulator_state, ModulatorState::Idle) {
        // self.reset_seed(at_global_counter as u64);
        //}
        self.modulator_state = modulator_state;

        let t = self.get_current_time_with_nanos();

        // if the change is in the future, no problem
        if ((t / self.hw.pulse_distance) as u64) < (at_global_counter + self.hw.gc_offset) {
            self.time_of_last_read = t;
            self.lfifo_initial = 0;
            Ok(0)
        }
        // if the change is in the past, we have to generate zeros, as this is what we want to for the real hardware
        else {
            let l = self.get_l(t, at_global_counter);
            if l.is_none() || l.unwrap() > self.size_of_idle_fifo {
                return Err(HardwareError::ResetFifoAtThisGcOverflow);
            }
            let mut v;
            let leftover;
            match &self.modulator_state {
                ModulatorState::Qkd => {
                    let (mut _v, mut _leftover) =
                        self.correlations_bb84(l.unwrap() as usize).map_err(|e| {
                            println!("ERROR : {:?}", e.to_string());
                            HardwareError::Other {
                                reason: e.to_string(),
                            }
                        })?;
                    v = _v;
                    leftover = _leftover;
                }
                ModulatorState::Random(_) => {
                    let (mut _v, mut _leftover) =
                        self.correlations_random(l.unwrap() as usize).map_err(|e| {
                            println!("ERROR : {:?}", e.to_string());
                            HardwareError::Other {
                                reason: e.to_string(),
                            }
                        })?;
                    v = _v;
                    leftover = _leftover;
                }
                ModulatorState::Idle => {
                    return Ok(0);
                }
                _ => {
                    return Err(HardwareError::ModulatorStateNotSupported);
                }
            }
            for e in &mut v {
                *e &= 0b1; // leave the last bit as result;
            }
            self.lfifo_initial = v.len() as u32;
            v.extend(leftover);
            self.leftover = v;
            self.time_of_last_read = t;
            Ok(l.unwrap())
        }
    }

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
    fn read_angles(&mut self) -> Result<Vec<u8>, HardwareError> {
        let current_time = self.get_current_time_with_nanos();
        let t = current_time - self.time_of_last_read;
        let l = ((t / self.hw.pulse_distance - self.hw.gc_offset as f64) * self.eta) as usize;
        if l as u64 > self.fifo_size {
            return Err(HardwareError::FifoOverflow);
        }
        self.time_of_last_read = current_time;
        //self.qb_err = (current_time % 7 as f64) * 0.01 + 0.02;

        match &self.modulator_state {
            ModulatorState::Idle => Ok(vec![0u8; 0]),
            ModulatorState::Qkd => {
                let (v, leftover) = self.correlations_bb84(l).map_err(|e| {
                    println!("ERROR : {:?}", e.to_string());
                    HardwareError::Other {
                        reason: e.to_string(),
                    }
                })?;
                self.leftover = leftover;
                self.lfifo_initial = 0;
                Ok(v)
            }
            ModulatorState::Random(_) => {
                let (v, leftover) = self.correlations_random(l).map_err(|e| {
                    println!("ERROR: {:?}", e.to_string());
                    HardwareError::Other {
                        reason: e.to_string(),
                    }
                })?;
                self.leftover = leftover;
                self.lfifo_initial = 0;
                Ok(v)
            }
            _ => Err(HardwareError::ModulatorStateNotSupported),
        }
    }
    /// Return the current global counter
    fn get_global_counter(&mut self) -> Option<u64> {
        ((self.get_current_time_with_nanos() / self.hw.pulse_distance) as u64)
            .checked_add(self.hw.gc_offset)
    }

    /// Return by how many values increases gc in 0.2 sec
    fn get_gcsafe(&mut self) -> u64 {
        (0.2 / self.hw.pulse_distance) as u64
    }
}

impl Simulator {
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
    use crate::backend::simulation::builder::SimulatorBuilder;
    use crate::backend::simulation::Simulator;
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;
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
                leftover: vec![],
                eta: 0.0,
                fifo_size: 50_000_000,
                size_of_idle_fifo: 1_000_000,
                now,
                time_of_last_read: 0.0,
                global_counter: 0,
                qb_err: 0.0,
                modulator_state: Default::default(),
                lfifo_initial: 0,
            },
            sim
        )
    }
}
