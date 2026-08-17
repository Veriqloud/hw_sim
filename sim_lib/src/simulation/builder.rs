use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;
use std::time::Instant;

use configs::backend::{Configuration, DecoyStatesConfig, QberConfig};

use crate::{
    hardware::{
        Hardware, builder::HardwareBuilder, modes::SimulatorMode, modulator_state::ModulatorState,
    },
    simulation::Simulator,
}; // Keep SimulatorMode

pub struct SimulatorBuilder {
    pub eta: f64,
    pub hw: Hardware,
    pub modulator_state: ModulatorState,
    pub angles: Vec<u8>,
    pub now: Instant,
    pub qb_err: QberConfig,
    pub rng: Pcg64Mcg,
    pub seed: u64,
    pub mode: SimulatorMode,
    pub use_gcr_padding: bool,
    pub use_rate_limiter: bool,
    pub decoy_states: Option<DecoyStatesConfig>,
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
            .with_qb_err(conf.qberr.to_owned())
            .with_eta(conf.eta)
            .with_rng(Pcg64Mcg::seed_from_u64(conf.seed))
            .with_seed(conf.seed)
            .with_mode(mode)
            .with_decoy_states(conf.decoy_states.clone())
            .build()
    }

    pub fn build(&self) -> Simulator {
        Simulator {
            hw: self.hw.to_owned(),
            simulator_mode: self.mode,
            rng: self.rng.to_owned(),
            qber_oscillation_rng: Pcg64Mcg::seed_from_u64(self.seed),
            eta: self.eta,
            qb_err: self.qb_err.to_owned(),
            now: self.now,
            modulator_state: self.modulator_state.to_owned(),
            angles: self.angles.to_owned(),
            time_of_start: None,
            use_gcr_padding: self.use_gcr_padding,
            rate_limiting_enabled: self.use_rate_limiter,
            last_event_count: 0,
            is_under_attack: false,
            decoy_states: self.decoy_states.clone(),
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

    pub fn with_qb_err(&mut self, qb_err: QberConfig) -> &mut Self {
        self.qb_err = qb_err;
        self
    }

    pub fn with_now(&mut self, now: Instant) -> &mut Self {
        self.now = now;
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

    pub fn with_rate_limiter(&mut self, use_limiter: bool) -> &mut Self {
        self.use_rate_limiter = use_limiter;
        self
    }

    pub fn with_decoy_states(&mut self, config: Option<DecoyStatesConfig>) -> &mut Self {
        self.decoy_states = config;
        self
    }
}

impl Default for SimulatorBuilder {
    fn default() -> Self {
        SimulatorBuilder {
            eta: Default::default(),
            hw: Default::default(),
            modulator_state: Default::default(),
            angles: Default::default(),
            now: Instant::now(),
            qb_err: Default::default(),
            rng: Pcg64Mcg::seed_from_u64(42),
            seed: 42,
            mode: Default::default(),
            use_gcr_padding: true,
            use_rate_limiter: true,
            decoy_states: None,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;
    use std::time::Instant;

    use crate::{
        hardware::{
            builder::HardwareBuilder, modes::SimulatorMode, modulator_state::ModulatorState,
        },
        simulation::{Simulator, builder::SimulatorBuilder},
    };

    use configs::backend::QberConfig;

    #[test]
    fn test_builder() {
        let now = Instant::now();
        let hw = HardwareBuilder::new()
            .with_pulse_distance(1e-9)
            .with_gc_offset(3)
            .build();
        let sim = SimulatorBuilder::new()
            .with_hardware(hw.clone())
            .with_mode(SimulatorMode::Detector)
            .with_rng(Pcg64Mcg::seed_from_u64(5))
            .with_eta(13.)
            .with_qb_err(QberConfig::Fixed { value: 42. })
            .with_now(now)
            .with_modulator_state(ModulatorState::Random)
            .with_seed(5)
            .with_angles(vec![0, 32, 34, 96])
            .with_gcr_padding(false)
            .with_rate_limiter(false)
            .build();

        assert_eq!(
            Simulator {
                hw,
                simulator_mode: SimulatorMode::Detector,
                rng: Pcg64Mcg::seed_from_u64(5),
                qber_oscillation_rng: Pcg64Mcg::seed_from_u64(5),
                eta: 13.,
                qb_err: QberConfig::Fixed { value: 42. },
                now,
                modulator_state: ModulatorState::Random,
                angles: vec![0, 32, 34, 96],
                time_of_start: None,
                use_gcr_padding: false,
                rate_limiting_enabled: false,
                last_event_count: 0,
                is_under_attack: false,
                decoy_states: None,
            },
            sim
        )
    }
}
