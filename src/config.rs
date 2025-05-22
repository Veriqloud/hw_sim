use nix::sys::stat::Mode;
use nix::unistd::mkfifo;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

pub mod errors;

use self::errors::Error;

#[derive(Debug, Deserialize, Serialize, PartialEq, Default)]
pub struct Configuration {
    pub backend_config: crate::backend::config::Configuration,
    pub ipc_config: crate::ipc::config::Configuration,
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

    pub fn setup_ipc_fifos(&self) -> Result<(), Error> {
        tracing::info!("Ensuring IPC FIFOs are set up...");
        let ipc_paths = [
            &self.ipc_config.command_file_path,
            &self.ipc_config.angle_file_path,
            &self.ipc_config.click_result_file_path,
            &self.ipc_config.gc_file_path,
        ];

        for path_str in &ipc_paths {
            ensure_fifo_exists(path_str)
                .context(errors::FifoCreationSnafu { path: path_str.to_string() })?;
        }
        tracing::info!("IPC FIFOs setup complete.");
        Ok(())
    }
}

/// Ensures that a FIFO exists at the given path.
/// If a file (or old FIFO) exists at the path, it is removed first.
/// Parent directories are created if they don't exist.
fn ensure_fifo_exists(path_str: &str) -> Result<(), std::io::Error> {
    let path = Path::new(path_str);

    // Create parent directory if it doesn't exist
    if let Some(parent_dir) = path.parent() {
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)?;
            tracing::info!("Created directory: {:?}", parent_dir);
        }
    }

    // Attempt to remove the file if it exists. This handles cases where it's a regular file
    // or an old FIFO that needs to be replaced.
    match fs::remove_file(path) {
        Ok(_) => tracing::info!("Removed existing file/FIFO at: {}", path_str),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File not found, which is fine, we'll create it.
            tracing::debug!(
                "No existing file/FIFO at: {}. Proceeding to create.",
                path_str
            );
        }
        Err(e) => {
            // For other errors during removal, log and return the error.
            tracing::error!("Error removing existing file/FIFO at {}: {}", path_str, e);
            return Err(e);
        }
    }

    // Create the FIFO.
    tracing::info!("Creating FIFO at: {} with mode 0666", path_str);
    // Permissions: rw-rw-rw- (0o666)
    let mode = Mode::S_IRUSR
        | Mode::S_IWUSR
        | Mode::S_IRGRP
        | Mode::S_IWGRP
        | Mode::S_IROTH
        | Mode::S_IWOTH;
    mkfifo(path, mode)?;
    Ok(())
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
