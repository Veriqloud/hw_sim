use nix::{sys::stat::Mode, unistd::mkfifo};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    path::Path,
    str::FromStr,
};

// Use the new local error type
pub mod errors;
use self::errors::{Error, FifoCreationSnafu, MockMmioFileSetupSnafu};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub command_path: String,    // e.g., /dev/xdma0_user or a mock file path
    pub angle_file_path: String, // Should be /dev/c2h_angles
    // click_result_file_path is no longer used as click results are part of the GCR stream.
    // pub click_result_file_path: String, 
    pub gcr_file_path: String, // Renamed: FIFO for writing GCR data (GC + result bit)
    pub gc_read_file_path: String,  // FIFO for reading GC values echoed by controller
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            command_path: String::from_str("/dev/xdma0_user").unwrap(),
            angle_file_path: String::from_str("/dev/c2h_angles").unwrap(),
            // click_result_file_path: String::from_str("/dev/c2h_click_results").unwrap(), // Removed
            gcr_file_path: String::from_str("./files/gcr").unwrap(), // Renamed and adjusted default
            gc_read_file_path: String::from_str("./files/gc_read").unwrap(),   // Adjusted default
        }
    }
}

impl Configuration {
    pub fn setup_ipc_fifos(&self) -> Result<(), Error> {
        tracing::info!("Ensuring IPC FIFOs are set up for IPC config...");
        // Note: command_path (xdma_device_path) is not a FIFO if it's a real device,
        // or it's a regular file if mocked (handled separately).
        let ipc_fifo_paths = [
            &self.angle_file_path,
            // &self.click_result_file_path, // No longer a separate FIFO
            &self.gcr_file_path, // Renamed
            &self.gc_read_file_path,
        ];

        for path_str in &ipc_fifo_paths {
            ensure_fifo_exists(path_str).context(FifoCreationSnafu {
                path: path_str.to_string(),
            })?;
        }
        tracing::info!("IPC FIFOs setup complete for IPC config.");

        // If command_path looks like a local mock file, ensure it exists and is sized.
        if self.command_path.starts_with("./files/") {
            // Constants from ipc/reader: MMIO_MAP_OFFSET (0x12000), MMIO_MAP_LEN (0x1000)
            // Constants from simu_controller: START_TRIGGER_OFFSET (0x12000), MMIO_MAP_LEN (0x1000)
            // The reset command in simu_controller uses offset 0x1000.
            // Max offset used is 0x12000 for start/stop triggers.
            let max_offset_in_mapping = 0x12000; // e.g. START_TRIGGER_OFFSET
            let map_region_len = 0x1000;   // e.g. MMIO_MAP_LEN in ipc_reader
                                           // The actual file needs to be large enough to contain the data at map_offset + address_within_map
                                           // The map_offset itself can be large.
                                           // simu_controller uses map_offset 0x1000 (for reset) and 0x12000 (for start/stop)
                                           // ipc_reader uses map_offset 0x12000.
                                           // The largest address within any map is small (e.g., 16 or 20 bytes).
                                           // So, the file size must be at least max_map_offset + map_region_len.
            let required_file_size = max_offset_in_mapping + map_region_len;

            ensure_mock_mmio_file_exists(&self.command_path, required_file_size).context(
                MockMmioFileSetupSnafu {
                    path: self.command_path.clone(),
                },
            )?;
        }
        Ok(())
    }
}

/// Ensures a regular file exists at the given path with at least the required size,
/// typically for mock MMIO.
fn ensure_mock_mmio_file_exists(path_str: &str, required_size: usize) -> Result<(), std::io::Error> {
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
                tracing::info!("Mock MMIO file {} already exists with sufficient size.", path_str);
                recreate_file = false;
            } else {
                tracing::info!(
                    "Mock MMIO file {} exists but is not suitable (not a file, or too small). It will be recreated.",
                    path_str
                );
                // Attempt to remove it before recreating
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
            // Try to remove, ignore error if it didn't exist or couldn't be removed
            let _ = fs::remove_file(path);
        }
    }

    if recreate_file {
        tracing::info!(
            "Creating/resizing mock MMIO file: {} with size {} bytes",
            path_str,
            required_size
        );
        let file = fs::OpenOptions::new() // Removed mut
            .read(true) // Need read for existing content check, write for modification
            .write(true)
            .create(true) // Create if it doesn't exist
            .open(path)?;

        // Ensure file is at least `required_size`
        let metadata = file.metadata()?;
        if metadata.len() < required_size as u64 {
            file.set_len(required_size as u64)?;
            tracing::info!(
                "Mock MMIO file {} resized to {} bytes.",
                path_str,
                required_size
            );
        }
        tracing::info!("Mock MMIO file {} created/ensured with size {} bytes.", path_str, required_size);
    } else {
        // File exists and is large enough, open it for modification
        // This branch is taken if recreate_file is false.
    }

    // Always zero out the command trigger location to ensure a clean "stopped" state (value 0).
    // MMIO_MAP_OFFSET from ipc::reader is 0x12000
    // COMMAND_TRIGGER_ADDR_BYTES (formerly START_TRIGGER_ADDR_BYTES) from ipc::reader is 16 (0x10)
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
    let mode = Mode::from_bits(0o666).expect("Failed to create mode from bits");
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
        // Test data file "src/ipc/test_data/valid_config.json" was updated in a previous step.
        // This assertion should now match the structure of that updated file.
        assert_eq!(
            crate::ipc::config::Configuration {
                command_path: "/dev/xdma0_user_test".to_owned(),
                angle_file_path: "/dev/c2h_angles_test".to_owned(),
                // click_result_file_path removed
                gcr_file_path: "/dev/h2c_gcr_test".to_owned(), // Renamed
                gc_read_file_path: "/dev/h2c_gc_read_test".to_owned()
            },
            config_input
        );
    }
}
