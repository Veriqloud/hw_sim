use once_cell::sync::Lazy;
use snafu::Whatever;
use std::f64::consts::PI;

use crate::{errors::SimulationError, simulation::batches::QkdBatch};

mod errors;
mod hardware;
mod simulation;

pub const BATCH_SIZE: usize = 1024;

// We work on batches of const size that are appended to v. It's faster that way.
pub const BATCH: usize = 1 << 10;

pub(crate) static OVERLAP_PROBABILITIES: Lazy<[u16; 128]> = Lazy::new(|| {
    let mut buf = [0u16; 128];
    for (i, elt) in buf.iter_mut().enumerate() {
        let angle_rad = (i as f64 / 128.0) * PI;
        *elt = (angle_rad.cos().powi(2) * (u16::MAX as f64)) as u16;
    }
    buf
});

pub trait ServiceCorrelationsRandom {
    /// Generates a complete batch of QKD data, including choices and results for both parties.
    fn generate_qkd_batch(&mut self) -> Result<QkdBatch, SimulationError>;
}
