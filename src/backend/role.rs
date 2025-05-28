/// SimulatorMode defines the operational mode of the simulator (Source or Detector).
#[derive(Debug, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum SimulatorMode {
    Source,
    Detector,
}

impl Default for SimulatorMode {
    fn default() -> Self {
        SimulatorMode::Detector // Default to Detector
    }
}
