pub mod backend;
pub mod errors;
pub mod ipc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::str::FromStr;

use self::errors::Error;

#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
pub struct Configuration {
    pub backend_config: backend::Configuration,
    pub ipc_config: ipc::Configuration,
    pub log_level: LogLevel,
}


impl Configuration {
    pub fn new(path: String) -> Result<Self, Error> {
        if path.is_empty() {
            return Ok(Configuration::default());
        }
        let config_string =
            std::fs::read_to_string(path.as_str()).context(errors::ReadConfigSnafu { path })?;

        serde_json::from_str(config_string.as_str()).context(errors::ParseConfigSnafu)
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, JsonSchema)]
pub struct LogLevel(pub String);

impl Default for LogLevel {
    fn default() -> Self {
        LogLevel("Info".to_string())
    }
}

impl TryFrom<LogLevel> for tracing_subscriber::filter::LevelFilter {
    type Error = tracing_subscriber::filter::LevelParseError;

    fn try_from(value: LogLevel) -> Result<Self, tracing_subscriber::filter::LevelParseError> {
        match tracing_subscriber::filter::LevelFilter::from_str(&value.0) {
            Ok(l) => Ok(l),
            Err(e) => Err(e),
        }
    }
}

impl TryFrom<String> for Configuration {
    type Error = errors::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match serde_json::from_str(s.as_str()) {
            Ok(c) => Ok(c),
            Err(e) => Err(errors::Error::ParseConfig { source: e }),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::to_string_pretty;

    use super::*;

    #[test]
    fn test_default_config() {
        let config = Configuration::default();

        assert_eq!(config.ipc_config, ipc::Configuration::default());

        assert_eq!(config.backend_config, backend::Configuration::default());

        assert_eq!(config.log_level, LogLevel::default());
    }

    #[test]
    fn test_try_from() {
        let config = Configuration::default();

        let config_string = serde_json::to_string(&config).unwrap();

        println!(
            "config string {}",
            to_string_pretty(&config_string).unwrap()
        );

        let config_from_string = Configuration::try_from(config_string).unwrap();

        assert_eq!(config, config_from_string);
    }

    #[test]
    #[ignore]
    fn export_schema() {
        let schema = schemars::schema_for!(Configuration);
        let schema_json = serde_json::to_string_pretty(&schema).unwrap();
        std::fs::write("../schema.json", schema_json).unwrap();

        let default_config = Configuration::default();
        let default_json = serde_json::to_string_pretty(&default_config).unwrap();
        std::fs::write("../default_config.json", default_json).unwrap();
    }

    #[test]
    #[ignore]
    fn validate_generated_configs() {
        let stack_config_dir =
            std::env::var("STACK_CONFIG_DIR").unwrap_or_else(|_| "../../stack_config".to_string());
        let generated_dir = std::path::Path::new(&stack_config_dir).join("generated");

        for file_name in ["alice_hw_sim.json", "bob_hw_sim.json"] {
            let path = generated_dir.join(file_name);
            Configuration::new(path.to_string_lossy().into_owned())
                .unwrap_or_else(|error| panic!("failed to load {}: {}", path.display(), error));
        }
    }
}
