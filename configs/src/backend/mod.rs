use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Configuration {
    pub angles: Vec<u8>,
    pub seed: u64,
    pub eta: f64,
    pub qberr: f64,
    pub pulse_distance: f64,
    #[serde(default = "default_rate_limiting")]
    pub rate_limiting: bool,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            angles: vec![0, 32, 64, 96],
            seed: 42,
            eta: 0.,
            qberr: 0.,
            pulse_distance: 1e-8,
            rate_limiting: default_rate_limiting(),
        }
    }
}

fn default_rate_limiting() -> bool {
    true
}
