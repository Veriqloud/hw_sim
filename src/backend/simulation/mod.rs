pub mod builder;
pub mod errors;

use async_trait::async_trait;

use crate::backend::protocols::random::CorrelationsRandom;
use crate::backend::role::{Multiparty, Role};
use libhardware::errors::HardwareError;
use libhardware::{Hardware, ModulatorState};
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
    pub(crate) fifo_max_size: u64,
    pub(crate) current_fifo_size: usize,
}

#[async_trait]
pub trait VqSim {
    fn fifo_idle(&mut self) -> Result<(), HardwareError>;
    fn get_global_counter(&mut self) -> Option<u64>;
    async fn read_angles(&mut self) -> Result<[u8; 1024], HardwareError>;
    async fn generate_bytes(&mut self) -> Result<Vec<u8>, HardwareError>;
    fn set_angles(&mut self, angles: [u8; 8]) -> Result<(), HardwareError>;
    fn start_at_gc(&mut self, gc: u64) -> Result<(), HardwareError>;
    fn set_role(&mut self, nb_parties: u32, position: u32) -> Result<(), HardwareError>;
}

#[async_trait]
impl VqSim for Simulator {
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
    async fn read_angles(&mut self) -> Result<[u8; 1024], HardwareError> {
        match &self.modulator_state {
            ModulatorState::Idle => {
                if self.current_fifo_size < 1024 {
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
                    self.current_fifo_size -= 1024;
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

                let size = l + self.current_fifo_size;
                tracing::debug!("Fifo size before generation: {}", self.current_fifo_size);
                tracing::debug!("Fifo size after generation: {size}");
                if size as u64 > self.fifo_max_size {
                    return Err(HardwareError::FifoOverflow);
                }
                if size < 1024 {
                    let n = 1024 - size;
                    let t =
                        (n as f64 / self.eta + self.hw.gc_offset as f64) * self.hw.pulse_distance;
                    println!("Need to wait t = {} microsec to generate {} bytes", t, n);
                    let task = tokio::time::sleep(tokio::time::Duration::from_micros(t as u64));

                    let (_, res) = tokio::join!(task, self.generate_bytes()); // self.generate_bytes()).await;
                    match res {
                        Ok(v) => {
                            self.current_fifo_size = 0;
                            let v = v.try_into().unwrap();
                            return Ok(v);
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                }
                self.time_of_last_read = current_time;
                //self.qb_err = (current_time % 7 as f64) * 0.01 + 0.02;
                let v = self.correlations_random(1024).map_err(|e| {
                    println!("ERROR: {:?}", e.to_string());
                    HardwareError::Other {
                        reason: e.to_string(),
                    }
                })?;
                self.current_fifo_size = size - 1024;
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
        self.reset_seed(gc);
        self.set_gc(gc);
        self.reset_time();
        self.modulator_state = ModulatorState::Random;
        Ok(())
    }

    fn set_angles(&mut self, angles: [u8; 8]) -> Result<(), HardwareError> {
        self.angles = angles.to_vec();
        Ok(())
    }

    async fn generate_bytes(&mut self) -> Result<Vec<u8>, HardwareError> {
        self.correlations_random(1024).map_err(|e| {
            println!("ERROR: {:?}", e.to_string());
            HardwareError::Other {
                reason: e.to_string(),
            }
        })
    }

    fn set_role(&mut self, nb_parties: u32, position: u32) -> Result<(), HardwareError> {
        self.role = Role::OneOfMany(Multiparty {
            number_of_parties: nb_parties,
            position,
        });
        Ok(())
    }
}

impl Simulator {
    /// return time elapsed since start in seconds at nanoseconds.
    fn get_current_time_with_nanos(&self) -> f64 {
        let duration = self.now.elapsed();
        duration.as_secs() as f64 + duration.subsec_nanos() as f64 * 1e-9
    }
    /// Restart RNG with a new seed.
    fn reset_seed(&mut self, seed: u64) {
        self.rng = Pcg64Mcg::seed_from_u64(seed);
    }
    /// Reset time to now
    pub fn reset_time(&mut self) {
        self.now = Instant::now();
    }
    /// Update the value of eta
    pub fn set_eta(&mut self, eta: f64) {
        self.eta = eta;
    }
    /// Set the global counter of the simulator
    pub fn set_gc(&mut self, gc: u64) {
        self.global_counter = gc;
        self.reset_seed(gc);
    }
    /// Update the value of qber
    pub fn set_qber(&mut self, qber: f64) {
        self.qb_err = qber;
    }
    /// Update the Role of the simulator
    pub fn set_role(&mut self, nb_parties: u32, position: u32) {
        self.role = Role::OneOfMany(Multiparty {
            number_of_parties: nb_parties,
            position,
        });
    }
}
