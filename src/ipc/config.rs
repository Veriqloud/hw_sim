use nix::{sys::stat::Mode, unistd::mkfifo};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::{fs, path::Path, str::FromStr};

use crate::config::errors::{Error, FifoCreationSnafu};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub command_file_path: String,      // Should be /dev/cmd
    pub angle_file_path: String,        // Should be /dev/c2h_angles
    pub click_result_file_path: String, // Should be /dev/c2h_click_results
    pub gc_file_path: String,           // Should be /dev/h2c_gc
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            command_file_path: String::from_str("/dev/cmd").unwrap(),
            angle_file_path: String::from_str("/dev/c2h_angles").unwrap(),
            click_result_file_path: String::from_str("/dev/c2h_click_results").unwrap(),
            gc_file_path: String::from_str("/dev/h2c_gc").unwrap(),
        }
    }
}

impl Configuration {
    pub fn setup_ipc_fifos(&self) -> Result<(), Error> {
        tracing::info!("Ensuring IPC FIFOs are set up...");
        let ipc_paths = [
            &self.command_file_path,
            &self.angle_file_path,
            &self.click_result_file_path,
            &self.gc_file_path,
        ];

        for path_str in &ipc_paths {
            ensure_fifo_exists(path_str).context(FifoCreationSnafu {
                path: path_str.to_string(),
            })?;
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

#[cfg(test)]
mod tests {
    #[test]
    fn valid_config() {
        let config_input_string =
            std::fs::read_to_string("src/ipc/test_data/valid_config.json").unwrap();

        let config_input: crate::ipc::config::Configuration =
            serde_json::from_str(&config_input_string).unwrap();
        assert_eq!(
            crate::ipc::config::Configuration {
                command_file_path: "/dev/cmd_test".to_owned(),
                angle_file_path: "/dev/c2h_angles_test".to_owned(),
                click_result_file_path: "/dev/c2h_click_results_test".to_owned(),
                gc_file_path: "/dev/h2c_gc_test".to_owned()
            },
            config_input
        );
    }
}
