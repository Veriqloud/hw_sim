use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

/// Fixed phase offset applied by the simulated optical setup.
///
/// One full turn is represented by 128 phase steps, so a quarter turn is 32
/// steps. This setting only affects the simulated measurement correlation. It
/// does not rotate or otherwise modify the two-bit setting codes written to the
/// Alice and Bob FIFOs.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq, Clone, Copy, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceSettingOffset {
    None,
    #[default]
    QuarterTurn,
    HalfTurn,
    ThreeQuarterTurns,
}

impl SourceSettingOffset {
    /// Converts the configured fraction of a turn to the simulator's 128-step
    /// phase representation.
    pub const fn phase_steps(self) -> u8 {
        match self {
            Self::None => 0,
            Self::QuarterTurn => 32,
            Self::HalfTurn => 64,
            Self::ThreeQuarterTurns => 96,
        }
    }
}

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
pub struct DecoyStatesConfig {
    /// Mean photon number for signal pulses.
    pub mu1: f64,
    /// Mean photon number for decoy pulses.
    pub mu2: f64,
    /// Probability of selecting signal intensity mu1 (vs decoy mu2).
    pub p1: f64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
pub struct Configuration {
    pub angles: Vec<u8>,
    /// Phase offset used by the optical correlation model. The default keeps
    /// the historical `+32` behavior.
    #[serde(default)]
    pub source_setting_offset: SourceSettingOffset,
    pub seed: u64,
    pub eta: f64,
    #[serde(deserialize_with = "deserialize_qber")]
    pub qberr: QberConfig,
    pub pulse_distance: f64,
    /// Decoy-state parameters. Absent means decoy mode is disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoy_states: Option<DecoyStatesConfig>,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            angles: vec![0, 32, 64, 96],
            source_setting_offset: SourceSettingOffset::default(),
            seed: 42,
            eta: 1.,
            qberr: QberConfig::default(),
            pulse_distance: 1e-8,
            decoy_states: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SourceSettingOffset;

    #[test]
    fn source_setting_offset_uses_quarter_turn_by_default() {
        let offset = SourceSettingOffset::default();

        assert_eq!(offset, SourceSettingOffset::QuarterTurn);
        assert_eq!(offset.phase_steps(), 32);
    }

    #[test]
    fn source_setting_offset_deserializes_all_config_values() {
        for (json, expected, phase_steps) in [
            (r#""none""#, SourceSettingOffset::None, 0),
            (r#""quarter_turn""#, SourceSettingOffset::QuarterTurn, 32),
            (r#""half_turn""#, SourceSettingOffset::HalfTurn, 64),
            (
                r#""three_quarter_turns""#,
                SourceSettingOffset::ThreeQuarterTurns,
                96,
            ),
        ] {
            let offset: SourceSettingOffset = serde_json::from_str(json).unwrap();
            assert_eq!(offset, expected);
            assert_eq!(offset.phase_steps(), phase_steps);
        }
    }
}
