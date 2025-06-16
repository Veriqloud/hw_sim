use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::str::FromStr;

pub mod errors;

use self::errors::Error;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub backend_config: crate::backend::config::Configuration,
    pub ipc_config: crate::ipc::config::Configuration,
    pub log_level: LogLevel,
    // simulator_mode is removed, as it's now determined by the structure of ipc_config
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            backend_config: Default::default(),
            ipc_config: Default::default(), // Defaults to BobIpcConfig
            log_level: Default::default(),
        }
    }
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

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct LogLevel(String);

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

        assert_eq!(
            config.ipc_config,
            crate::ipc::config::Configuration::default()
        );

        assert_eq!(
            config.backend_config,
            crate::backend::config::Configuration::default()
        );

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
}
