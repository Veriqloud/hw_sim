/// Role defines the multiparty configuration (number of parties, position).
#[derive(Debug, PartialEq, Clone)]
pub enum Role {
    /// Multiparty: tuple arguments are (number of parties, my position)
    OneOfMany(Multiparty),
}

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

#[derive(Debug, PartialEq, Clone)]
pub struct Multiparty {
    pub number_of_parties: u32,
    /// My position amongst parties
    pub position: u32,
}

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

impl Default for Role {
    fn default() -> Self {
        Role::OneOfMany(Multiparty {
            number_of_parties: 1,
            position: 0,
        })
    }
}
