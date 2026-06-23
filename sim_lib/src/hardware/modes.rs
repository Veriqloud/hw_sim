// Role and Multiparty are removed as SimulatorMode now dictates behavior.
// The system is fixed to a 2-party setup:
// - SimulatorMode::Source implies position 0.
// - SimulatorMode::Detector implies position 1.

/// SimulatorMode defines the operational mode of the simulator (Source or Detector).
#[derive(Debug, Default, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum SimulatorMode {
    Source,
    #[default]
    Detector,
}
