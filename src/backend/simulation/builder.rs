use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;
use std::time::Instant;

use crate::backend::config::Configuration;
use crate::backend::role::Multiparty;
use crate::backend::role::Role;

use super::hardware::builder::HardwareBuilder;
use super::hardware::modulator_state::ModulatorState;
use super::hardware::Hardware;
use super::Simulator;

pub struct SimulatorBuilder {
    /// Total qubit detection efficiency
    pub eta: f64,
    /// Size of the physical FIFO, for realistic HardwareError, "Size" means number of bytes.
    pub fifo_max_size: u64,
    /// Offset is taken care of automatically.
    /// Equivalent to Bob broadcasting his global counter in the real world.
    /// Probably not required ...
    pub global_counter: u64,
    pub hw: Hardware,
    pub current_fifo_size: usize,
    pub modulator_state: ModulatorState,
    pub angles: Vec<u8>,
    pub now: Instant,
    /// Qubit error rate
    pub qb_err: f64,
    pub rng: Pcg64Mcg,
    pub role: Role,
    pub time_of_last_read: f64,
}

impl SimulatorBuilder {
    pub fn new() -> SimulatorBuilder {
        SimulatorBuilder::default()
    }

    pub fn from_config(conf: Configuration) -> Simulator {
        let hw = HardwareBuilder::new()
            .with_pulse_distance(conf.pulse_distance)
            .build();
        SimulatorBuilder::default()
            .with_hardware(hw)
            .with_angles(conf.angles.to_owned())
            .with_role(Role::OneOfMany(Multiparty {
                number_of_parties: conf.number_of_parties,
                position: conf.position,
            }))
            .with_qb_err(conf.qberr)
            .with_eta(conf.eta)
            .with_rng(Pcg64Mcg::seed_from_u64(conf.seed))
            .build()
    }

    pub fn build(&self) -> Simulator {
        Simulator {
            hw: self.hw.to_owned(),
            role: self.role.to_owned(),
            rng: self.rng.to_owned(),
            eta: self.eta,
            qb_err: self.qb_err,
            now: self.now,
            time_of_last_read: self.time_of_last_read,
            global_counter: self.global_counter,
            modulator_state: self.modulator_state.to_owned(),
            fifo_max_size: self.fifo_max_size,
            current_fifo_size: self.current_fifo_size,
            angles: self.angles.to_owned(),
        }
    }

    pub fn with_angles(&mut self, angles: Vec<u8>) -> &mut Self {
        self.angles = angles;
        self
    }

    pub fn with_hardware(&mut self, hw: Hardware) -> &mut Self {
        self.hw = hw;
        self
    }

    pub fn with_role(&mut self, role: Role) -> &mut Self {
        self.role = role;
        self
    }

    pub fn with_rng(&mut self, rng: Pcg64Mcg) -> &mut Self {
        self.rng = rng;
        self
    }

    pub fn with_eta(&mut self, eta: f64) -> &mut Self {
        self.eta = eta;
        self
    }

    pub fn with_qb_err(&mut self, qb_err: f64) -> &mut Self {
        self.qb_err = qb_err;
        self
    }

    pub fn with_now(&mut self, now: Instant) -> &mut Self {
        self.now = now;
        self
    }

    pub fn with_time_of_last_read(&mut self, time_of_last_read: f64) -> &mut Self {
        self.time_of_last_read = time_of_last_read;
        self
    }

    pub fn with_global_counter(&mut self, global_counter: u64) -> &mut Self {
        self.global_counter = global_counter;
        self
    }

    pub fn with_modulator_state(&mut self, state: ModulatorState) -> &mut Self {
        self.modulator_state = state;
        self
    }

    pub fn with_fifo_max_size(&mut self, fifo_max_size: u64) -> &mut Self {
        self.fifo_max_size = fifo_max_size;
        self
    }
}

impl Default for SimulatorBuilder {
    fn default() -> Self {
        SimulatorBuilder {
            eta: Default::default(),
            fifo_max_size: 50_000_000,
            global_counter: Default::default(),
            hw: Default::default(),
            current_fifo_size: 0,
            modulator_state: Default::default(),
            angles: Default::default(),
            now: Instant::now(),
            qb_err: Default::default(),
            rng: Pcg64Mcg::seed_from_u64(10),
            role: Default::default(),
            time_of_last_read: Default::default(),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use crate::backend::role::Role;
    use crate::backend::simulation::builder::SimulatorBuilder;
    use crate::backend::simulation::hardware::builder::HardwareBuilder;
    use crate::backend::simulation::hardware::modulator_state::ModulatorState;
    use crate::backend::simulation::Simulator;
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;
    use std::time::Instant;

    #[test]
    fn test_builder() {
        let now = Instant::now();
        let hw = HardwareBuilder::new()
            .with_pulse_distance(1e-9)
            .with_gc_offset(3)
            .build();
        let sim = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_role(Role::default())
            .with_rng(Pcg64Mcg::seed_from_u64(5))
            .with_eta(13.)
            .with_qb_err(42.)
            .with_now(now)
            .with_time_of_last_read(55.)
            .with_global_counter(99)
            .with_modulator_state(ModulatorState::Random)
            .with_fifo_max_size(10_000)
            .with_angles(vec![0, 32, 34, 96])
            .build();

        assert_eq!(
            Simulator {
                hw,
                role: Role::OneOfMany(crate::backend::role::Multiparty {
                    number_of_parties: 1,
                    position: 0
                }),
                rng: Pcg64Mcg::seed_from_u64(5),
                eta: 13.,
                qb_err: 42.,
                now,
                time_of_last_read: 55.,
                global_counter: 99,
                modulator_state: ModulatorState::Random,
                fifo_max_size: 10_000,
                current_fifo_size: 0,
                angles: vec![0, 32, 34, 96],
            },
            sim
        )
    }
}
