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

#[cfg(test)]
mod tests {
    #[test]
    fn valid_config() {
        let config_json = r#"{
    "angles": [0, 10, 11, 12],
    "seed": 33,
    "eta": 0.1,
    "qberr": 0.02,
    "pulse_distance": 1e-8,
    "rate_limiting": false
}"#;

        let config_input: crate::backend::Configuration =
            serde_json::from_str(&config_json).unwrap();

        println!("Backend Config {:?}", &config_input);

        assert_eq!(
            crate::backend::Configuration {
                angles: vec![0, 10, 11, 12],
                seed: 33,
                eta: 0.1,
                qberr: 0.02,
                pulse_distance: 1e-8,
                rate_limiting: false,
            },
            config_input
        );
    }
}
