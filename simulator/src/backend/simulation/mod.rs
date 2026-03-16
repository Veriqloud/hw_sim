pub mod builder;
pub mod errors;

use crate::backend::protocols::random::{
    cr_constants, SimCorrelationsRandom, OVERLAP_PROBABILITIES,
};
use crate::backend::role::SimulatorMode;
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use std::time::{Duration, Instant};

use self::hardware::errors::HardwareError;
use self::hardware::modulator_state::ModulatorState;
use self::hardware::Hardware;
use crate::backend::protocols::errors::ProtocolError;
