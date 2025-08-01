use nix::{sys::stat::Mode, unistd::mkfifo};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    path::Path,
};

pub mod errors;
use self::errors::{Error, FifoCreationSnafu, MockMmioFileSetupSnafu};

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct AliceIpcConfig {
    pub command_path: String,
    pub angle_file_path: String,
    pub gc_read_file_path: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct BobIpcConfig {
    pub command_path: String,
    pub angle_file_path: String,
    pub gcr_file_path: String,
    pub gc_read_file_path: String,
}

impl Default for AliceIpcConfig {
    fn default() -> Self {
        AliceIpcConfig {
            command_path: "/tmp/fpga_alice".to_string(),
            angle_file_path: "/tmp/gc_alice_angle.fifo".to_string(),
            gc_read_file_path: "/tmp/gc_alice_gc.fifo".to_string(),
        }
    }
}

impl Default for BobIpcConfig {
    fn default() -> Self {
        BobIpcConfig {
            command_path: "/tmp/fpga_bob".to_string(),
            angle_file_path: "/tmp/gc_bob_angle.fifo".to_string(),
            gcr_file_path: "/tmp/gc_bob_gcr.fifo".to_string(),
            gc_read_file_path: "/tmp/gc_bob_gc.fifo".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(untagged)]
pub enum Configuration {
    Bob(BobIpcConfig),
    Alice(AliceIpcConfig),
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration::Bob(BobIpcConfig::default())
    }
}

impl Configuration {
    pub fn setup_ipc_fifos(&self) -> Result<(), Error> {
        tracing::info!("Ensuring IPC FIFOs are set up for IPC config...");
        match self {
            Configuration::Alice(config) => {
                let ipc_fifo_paths = [&config.angle_file_path, &config.gc_read_file_path];
                for path_str in &ipc_fifo_paths {
                    ensure_fifo_exists(path_str).context(FifoCreationSnafu {
                        path: path_str.to_string(),
                    })?;
                }

                if config.command_path.starts_with("./files/")
                    || config.command_path.starts_with("/tmp/")
                {
                    let required_file_size = 0x12000 + 0x1000;
                    ensure_mock_mmio_file_exists(&config.command_path, required_file_size)
                        .context(MockMmioFileSetupSnafu {
                            path: config.command_path.clone(),
                        })?;
                }
            }
            Configuration::Bob(config) => {
                let ipc_fifo_paths = [
                    &config.angle_file_path,
                    &config.gcr_file_path,
                    &config.gc_read_file_path,
                ];
                for path_str in &ipc_fifo_paths {
                    ensure_fifo_exists(path_str).context(FifoCreationSnafu {
                        path: path_str.to_string(),
                    })?;
                }

                // Also setup command path if it's a mock MMIO file for Bob
                if config.command_path.starts_with("./files/")
                    || config.command_path.starts_with("/tmp/")
                {
                    let required_file_size = 0x12000 + 0x1000; // Consistent with Alice
                    ensure_mock_mmio_file_exists(&config.command_path, required_file_size)
                        .context(MockMmioFileSetupSnafu {
                            path: config.command_path.clone(),
                        })?;
                }
            }
        }
        tracing::info!("IPC FIFOs setup complete for IPC config.");
        Ok(())
    }
}

/// Ensures a regular file exists at the given path with at least the required size,
/// typically for mock MMIO.
pub fn ensure_mock_mmio_file_exists(
    path_str: &str,
    required_size: usize,
) -> Result<(), std::io::Error> {
    let path = Path::new(path_str);

    if let Some(parent_dir) = path.parent() {
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)?;
            tracing::info!("Created directory for mock MMIO file: {:?}", parent_dir);
        }
    }

    let mut recreate_file = true;
    if path.exists() {
        if let Ok(metadata) = fs::metadata(path) {
            if metadata.is_file() && metadata.len() >= required_size as u64 {
                tracing::info!(
                    "Mock MMIO file {} already exists with sufficient size.",
                    path_str
                );
                recreate_file = false;
            } else {
                tracing::info!(
                    "Mock MMIO file {} exists but is not suitable (not a file, or too small). It will be recreated.",
                    path_str
                );
                if metadata.is_dir() {
                    fs::remove_dir_all(path)?;
                } else {
                    fs::remove_file(path)?;
                }
            }
        } else {
            tracing::warn!(
                "Could not get metadata for {}. Attempting to remove and recreate.",
                path_str
            );
            let _ = fs::remove_file(path);
        }
    }

    if recreate_file {
        tracing::info!(
            "Creating/resizing mock MMIO file: {} with size {} bytes",
            path_str,
            required_size
        );
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let metadata = file.metadata()?;
        if metadata.len() < required_size as u64 {
            file.set_len(required_size as u64)?;
            tracing::info!(
                "Mock MMIO file {} resized to {} bytes.",
                path_str,
                required_size
            );
        }
        tracing::info!(
            "Mock MMIO file {} created/ensured with size {} bytes.",
            path_str,
            required_size
        );
    }

    // Always zero out the command trigger location to ensure a clean "stopped" state (value 0).
    let command_trigger_abs_offset = 0x12000u64 + 16u64;
    let zero_value_bytes = 0u32.to_le_bytes();

    let mut file_to_write = fs::OpenOptions::new().write(true).open(path)?;

    if command_trigger_abs_offset + 4 <= required_size as u64 {
        file_to_write.seek(SeekFrom::Start(command_trigger_abs_offset))?;
        file_to_write.write_all(&zero_value_bytes)?;
        tracing::info!(
            "Zeroed command trigger at offset {:#X} in mock MMIO file {}.",
            command_trigger_abs_offset,
            path_str
        );
    } else {
        tracing::warn!(
            "Command trigger offset {:#X} is beyond mock MMIO file size {}. Cannot zero.",
            command_trigger_abs_offset,
            required_size
        );
    }

    Ok(())
}

/// Ensures that a FIFO exists at the given path.
/// If a file (or old FIFO) exists at the path, it is removed first.
/// Parent directories are created if they don't exist.
pub fn ensure_fifo_exists(path_str: &str) -> Result<(), std::io::Error> {
    let path = Path::new(path_str);

    if let Some(parent_dir) = path.parent() {
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)?;
            tracing::info!("Created directory: {:?}", parent_dir);
        }
    }

    match fs::remove_file(path) {
        Ok(_) => tracing::info!("Removed existing file/FIFO at: {}", path_str),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                "No existing file/FIFO at: {}. Proceeding to create.",
                path_str
            );
        }
        Err(e) => {
            tracing::error!("Error removing existing file/FIFO at {}: {}", path_str, e);
            return Err(e);
        }
    }

    tracing::info!("Creating FIFO at: {} with mode 0666", path_str);
    let mode = Mode::from_bits(0o666).expect("Failed to create mode from bits");
    mkfifo(path, mode)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AliceIpcConfig, BobIpcConfig, Configuration};

    #[test]
    fn valid_bob_config() {
        let config_json = r#"{
    "command_path": "/dev/command_test_bob",
    "angle_file_path": "/dev/c2h_angles_test",
    "gcr_file_path": "/dev/h2c_gcr_test",
    "gc_read_file_path": "/dev/h2c_gc_read_test"
}"#;

        let config_input: Configuration = serde_json::from_str(&config_json).unwrap();

        // Test data file represents a Bob (Detector) config.
        let expected_bob_config = BobIpcConfig {
            command_path: "/dev/command_test_bob".to_owned(),
            angle_file_path: "/dev/c2h_angles_test".to_owned(),
            gcr_file_path: "/dev/h2c_gcr_test".to_owned(),
            gc_read_file_path: "/dev/h2c_gc_read_test".to_owned(),
        };

        assert_eq!(Configuration::Bob(expected_bob_config), config_input);
    }

    #[test]
    fn valid_alice_config() {
        let config_json = r#"{
    "command_path": "/dev/command_test_alice",
    "angle_file_path": "/dev/c2h_angles_test_alice",
    "gc_read_file_path": "/dev/h2c_gc_read_test_alice"
}"#;

        let config_input: Configuration = serde_json::from_str(&config_json).unwrap();

        // Test data file represents an Alice (Source) config.
        let expected_alice_config = AliceIpcConfig {
            command_path: "/dev/command_test_alice".to_owned(),
            angle_file_path: "/dev/c2h_angles_test_alice".to_owned(),
            gc_read_file_path: "/dev/h2c_gc_read_test_alice".to_owned(),
        };

        assert_eq!(Configuration::Alice(expected_alice_config), config_input);
    }
}
