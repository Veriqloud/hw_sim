use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
#[serde(tag = "type")]
pub enum QberConfig {
    Fixed {
        value: f64,
    },
    Uniform {
        min: f64,
        max: f64,
    },
    Gaussian {
        mean: f64,
        #[serde(rename = "std_dev")]
        std_dev: f64,
    },
}

impl Default for QberConfig {
    fn default() -> Self {
        QberConfig::Fixed { value: 0.0 }
    }
}

/// Custom deserializer for QberConfig to support both the enum format and a raw f64 (for backward compatibility).
fn deserialize_qber<'de, D>(deserializer: D) -> Result<QberConfig, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum UntaggedQber {
        Fixed(f64),
        Config(QberConfig),
    }

    match UntaggedQber::deserialize(deserializer)? {
        UntaggedQber::Fixed(value) => Ok(QberConfig::Fixed { value }),
        UntaggedQber::Config(config) => Ok(config),
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
pub struct Configuration {
    pub angles: Vec<u8>,
    pub seed: u64,
    pub eta: f64,
    #[serde(deserialize_with = "deserialize_qber")]
    pub qberr: QberConfig,
    pub pulse_distance: f64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            angles: vec![0, 32, 64, 96],
            seed: 42,
            eta: 0.,
            qberr: QberConfig::default(),
            pulse_distance: 1e-8,
        }
    }
}
