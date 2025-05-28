use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub angles: Vec<u8>,
    pub seed: u64,
    pub eta: f64,
    pub qberr: f64,
    pub pulse_distance: f64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            angles: vec![0, 32, 64, 96],
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
                seed: 33,
                eta: 0.1,
                qberr: 0.02,
                pulse_distance: 1e-8,
                // mode: crate::backend::role::SimulatorMode::Detector, // This field is not part of backend::config::Configuration
            },
            config_input
        );
        // If mode needs to be checked, it should be done after parsing into the main Configuration struct,
        // not the backend::config::Configuration which doesn't store it.
        // For example, if the main config struct (e.g. crate::config::Configuration) holds a backend_config
        // and a mode separately, that's where `mode` would be asserted.
        // Based on the current structure of backend::config::Configuration, `mode` is not a field here.
        // Assuming the test intends to check the fields of `crate::backend::config::Configuration` only.
    }
}
