use once_cell::sync::Lazy;
use std::f64::consts::PI;

use crate::{errors::SimulationError, simulation::batches::QkdBatch};

pub mod errors;
pub mod hardware;
pub mod simulation;

pub const BATCH_SIZE: usize = 1024;

// We work on batches of const size that are appended to v. It's faster that way.
pub const BATCH: usize = 1 << 10;

/// A lazily-initialized static lookup table for detection probabilities.
/// The state is |psi> = cos(alpha)|0> + sin(alpha)|1>, where alpha is derived from the angle index.
/// The probability of measuring the initial state is cos^2(alpha).
///
/// The angle index (0-127) maps to a physical angle from 0 to PI.
///
/// The table holds `cos^2(angle)` scaled to `u16::MAX` for each index.
pub(crate) static OVERLAP_PROBABILITIES: Lazy<[u16; 128]> = Lazy::new(|| {
    let mut buf = [0u16; 128];
    for (i, elt) in buf.iter_mut().enumerate() {
        let angle_rad = (i as f64 / 128.0) * PI;
        *elt = (angle_rad.cos().powi(2) * (u16::MAX as f64)) as u16;
    }
    buf
});

pub trait ServiceCorrelationsRandom: Send {
    fn init_session(&mut self) -> Result<(), SimulationError>;
    fn setup_session_end(&self) -> Result<(), SimulationError>;
    /// Generates a complete batch of QKD data, including choices and results for both parties.
    fn generate_qkd_batch(
        &mut self,
        batch_logical_timestamp: Option<usize>,
    ) -> Result<QkdBatch, SimulationError>;
}
