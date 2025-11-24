use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;
use std::time::Instant;

use crate::backend::role::SimulatorMode;
use configs::backend::Configuration; // Keep SimulatorMode

use super::hardware::builder::HardwareBuilder;
use super::hardware::modulator_state::ModulatorState;
use super::hardware::Hardware;
use super::Simulator;

pub struct SimulatorBuilder {
    /// Total qubit detection efficiency
    pub eta: f64,
    /// Offset is taken care of automatically.
    /// Equivalent to Bob broadcasting his global counter in the real world.
    /// Probably not required ...
    pub global_counter: u64,
    pub hw: Hardware,
    pub modulator_state: ModulatorState,
    pub angles: Vec<u8>,
    pub now: Instant,
    /// Qubit error rate
    pub qb_err: f64,
    pub rng: Pcg64Mcg,
    pub seed: u64,
    pub mode: SimulatorMode, // Mode is still needed
    pub use_gcr_padding: bool,
}

impl SimulatorBuilder {
    pub fn new() -> SimulatorBuilder {
        SimulatorBuilder::default()
    }

    pub fn from_config(conf: &Configuration, mode: SimulatorMode) -> Simulator {
        let hw = HardwareBuilder::new()
            .with_pulse_distance(conf.pulse_distance)
            .build();
        SimulatorBuilder::default()
            .with_hardware(hw)
            .with_angles(conf.angles.to_owned())
            .with_qb_err(conf.qberr)
            .with_eta(conf.eta)
            .with_rng(Pcg64Mcg::seed_from_u64(conf.seed))
            .with_seed(conf.seed)
            .with_mode(mode) // Set the mode from the passed argument
            .build()
    }

    pub fn build(&self) -> Simulator {
        Simulator {
            hw: self.hw.to_owned(),
            simulator_mode: self.mode, // Ensure mode is assigned from builder's mode field
            rng: self.rng.to_owned(),
            eta: self.eta,
            seed: self.seed,
            qb_err: self.qb_err,
            now: self.now,
            global_counter: self.global_counter,
            modulator_state: self.modulator_state.to_owned(),
            angles: self.angles.to_owned(),
            pending_angles_batch: None, // Initialize to None
            time_of_start: None,        // Initialize to None
            use_gcr_padding: self.use_gcr_padding,
            last_event_count: 0, // Initialize to 0
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

    pub fn with_mode(&mut self, mode: SimulatorMode) -> &mut Self {
        self.mode = mode;
        self
    }

    pub fn with_rng(&mut self, rng: Pcg64Mcg) -> &mut Self {
        self.rng = rng;
        self
    }

    pub fn with_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = seed;
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

    pub fn with_global_counter(&mut self, global_counter: u64) -> &mut Self {
        self.global_counter = global_counter;
        self
    }

    pub fn with_modulator_state(&mut self, state: ModulatorState) -> &mut Self {
        self.modulator_state = state;
        self
    }

    pub fn with_gcr_padding(&mut self, use_padding: bool) -> &mut Self {
        self.use_gcr_padding = use_padding;
        self
    }
}

impl Default for SimulatorBuilder {
    fn default() -> Self {
        SimulatorBuilder {
            eta: Default::default(),
            global_counter: Default::default(),
            hw: Default::default(),
            modulator_state: Default::default(),
            angles: Default::default(),
            now: Instant::now(),
            qb_err: Default::default(),
            rng: Pcg64Mcg::seed_from_u64(42),
            seed: 42,
            mode: SimulatorMode::default(), // Add mode default
            use_gcr_padding: true,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use crate::backend::role::SimulatorMode; // Ensure SimulatorMode is imported
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
            .with_mode(SimulatorMode::Detector) // Add with_mode for testing
            .with_rng(Pcg64Mcg::seed_from_u64(5))
            .with_eta(13.)
            .with_qb_err(42.)
            .with_now(now)
            .with_global_counter(99)
            .with_modulator_state(ModulatorState::Random)
            .with_seed(5)
            .with_angles(vec![0, 32, 34, 96])
            .with_gcr_padding(false)
            .build();

        assert_eq!(
            Simulator {
                hw,
                simulator_mode: SimulatorMode::Detector, // Add mode to assertion
                rng: Pcg64Mcg::seed_from_u64(5),
                eta: 13.,
                qb_err: 42.,
                now,
                seed: 5,
                global_counter: 99,
                modulator_state: ModulatorState::Random,
                angles: vec![0, 32, 34, 96],
                pending_angles_batch: None,
                time_of_start: None,
                use_gcr_padding: false,
                last_event_count: 0,
                rate_limiting: true,
            },
            sim
        )
    }
}
