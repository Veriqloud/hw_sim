use serde::{Deserialize, Serialize};

/// The state of the phase modulator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum ModulatorState {
    /// Do nothing. Write nothing to the fifo.
    #[default]
    Idle,
    Random,
}
