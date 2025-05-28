use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub angles: Vec<u8>,
    // pub number_of_parties: u32, // Removed
    // pub position: u32, // Removed
    pub seed: u64,
    pub eta: f64,
    pub qberr: f64,
    pub pulse_distance: f64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            angles: vec![0, 32, 64, 96],
            // number_of_parties: 1, // Removed
            // position: 0, // Removed
            seed: 42,
            eta: 0.,
            qberr: 0.,
            pulse_distance: 1e-8,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_config() {
        let config_input_string =
            std::fs::read_to_string("src/backend/test_data/valid_config.json").unwrap();

        let config_input: crate::backend::config::Configuration =
            serde_json::from_str(&config_input_string).unwrap();

        println!("Backend Config {:?}", &config_input);

        assert_eq!(
            crate::backend::config::Configuration {
                angles: vec![0, 10, 11, 12],
                // number_of_parties: 2, // Removed
                // position: 1, // Removed
                seed: 33,
                eta: 0.1,
                qberr: 0.02,
                pulse_distance: 1e-8,
                mode: crate::backend::role::SimulatorMode::Detector, // mode is still relevant
            },
            config_input
        );
    }
}
