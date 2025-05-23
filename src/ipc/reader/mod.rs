pub mod errors;

use memmap2::MmapOptions;
use rand::Rng;
// Removed unused: use snafu::ResultExt;
use std::fs::OpenOptions as StdOpenOptions;
use std::time::Duration;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    task, time,
};

use crate::{backend::actor::ActorHandle as SimulatorHandle, ipc::Command};

use super::writer::actor::IPCWriterActorHandle;

// --- MMIO Constants ---
// These should be verified and match your hardware's memory map.
const MMIO_MAP_OFFSET: u64 = 0x12000; // Base offset for the command registers
const MMIO_MAP_LEN: usize = 0x1000; // Size of the memory map region

const COMMAND_TRIGGER_ADDR_BYTES: usize = 16; // Byte address for the Start/Stop trigger u32

const POLLING_INTERVAL_MS: u64 = 50; // Polling interval for MMIO commands

pub struct IPCReader {
    command_path: String,
    gc_file: File,
    gcr_file: File,
    writer_handle: IPCWriterActorHandle,
    simulator_handle: SimulatorHandle,
    last_known_command_trigger_value: u32, // Stores the last seen value (0 or 1) of the command trigger
}

/// Synchronously reads a u32 value from a memory-mapped device.
/// This function is intended to be run in a blocking thread.
fn read_u32_from_mmio(
    device_path: &str,
    map_offset: u64,
    map_len: usize,
    value_addr_bytes: usize,
) -> Result<u32, std::io::Error> {
    if value_addr_bytes % 4 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "MMIO address must be u32-aligned",
        ));
    }
    if value_addr_bytes + 4 > map_len {
        // Ensure the address is within the mapped region for a u32 read
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "MMIO address out of bounds for the mapped length",
        ));
    }

    let file = StdOpenOptions::new().read(true).open(device_path)?;
    unsafe {
        let mmap = MmapOptions::new()
            .len(map_len)
            .offset(map_offset)
            .map(&file)?;
        let ptr = mmap.as_ptr().add(value_addr_bytes) as *const u32;
        Ok(ptr.read_volatile()) // Use read_volatile for MMIO
    }
}

/// Synchronously writes a u32 value to a memory-mapped device.
/// This function is intended to be run in a blocking thread.
fn write_u32_to_mmio(
    device_path: &str,
    map_offset: u64,
    map_len: usize,
    value_addr_bytes: usize,
    value: u32,
) -> Result<(), std::io::Error> {
    if value_addr_bytes % 4 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "MMIO address must be u32-aligned",
        ));
    }
    if value_addr_bytes + 4 > map_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "MMIO address out of bounds for the mapped length",
        ));
    }

    let file = StdOpenOptions::new().read(true).write(true).open(device_path)?;
    unsafe {
        let mut mmap = MmapOptions::new()
            .len(map_len)
            .offset(map_offset)
            .map_mut(&file)?; // map_mut for writing
        let ptr = mmap.as_mut_ptr().add(value_addr_bytes) as *mut u32;
        ptr.write_volatile(value); // Use write_volatile for MMIO
        // For some devices/memory types, a flush might be needed.
        // mmap.flush()?;
    }
    Ok(())
}

impl IPCReader {
    async fn read_gc_from_file(&mut self) -> Result<u64, errors::Error> {
        // FIFOs are stream-based and do not support seek.
        // The file is opened fresh for each session, so reads will start from the beginning
        // of what the controller writes for that session.
        self.gc_file.read_u64_le().await.map_err(|e| {
            let reason = format!("Failed to read GC from file: {}", e);
            tracing::error!("{}", &reason);
            errors::Error::Unexpected { reason }
        })
    }

    pub fn new(
        command_path: String,
        gc_file: File,
        gcr_file: File,
        simulator_handle: SimulatorHandle,
        writer_handle: IPCWriterActorHandle,
    ) -> Self {
        IPCReader {
            command_path,
            gc_file,
            gcr_file,
            writer_handle,
            simulator_handle,
            last_known_command_trigger_value: 0, // Default to 0 (stopped), will be updated by initial read
        }
    }

    async fn await_next_command(&mut self) -> Result<Command, errors::Error> {
        loop {
            let device_path_clone = self.command_path.clone();
            let read_result = task::spawn_blocking(move || {
                read_u32_from_mmio(
                    &device_path_clone,
                    MMIO_MAP_OFFSET,
                    MMIO_MAP_LEN,
                    COMMAND_TRIGGER_ADDR_BYTES,
                )
            })
            .await;

            match read_result {
                Ok(Ok(current_value)) => {
                    if current_value == 1 && self.last_known_command_trigger_value == 0 {
                        tracing::info!(
                            "Start command detected via MMIO (0->1 transition at addr {:#X}). Current value: {}, Last known: {}",
                            COMMAND_TRIGGER_ADDR_BYTES, current_value, self.last_known_command_trigger_value
                        );
                        self.last_known_command_trigger_value = 1;
                        return Ok(Command::Start);
                    } else if current_value == 0 && self.last_known_command_trigger_value == 1 {
                        tracing::info!(
                            "Stop command detected via MMIO (1->0 transition at addr {:#X}). Current value: {}, Last known: {}",
                            COMMAND_TRIGGER_ADDR_BYTES, current_value, self.last_known_command_trigger_value
                        );
                        self.last_known_command_trigger_value = 0;
                        return Ok(Command::Stop);
                    } else if current_value != self.last_known_command_trigger_value && (current_value == 0 || current_value == 1) {
                        // MMIO changed but not an edge we were expecting (e.g. 0->0 or 1->1 from our perspective because we missed an intermediate state,
                        // or it was set to the current state externally). Update our known state.
                        tracing::debug!(
                            "MMIO command trigger changed to {} without a processed edge from {}. Updating last known value.",
                            current_value, self.last_known_command_trigger_value
                        );
                        self.last_known_command_trigger_value = current_value;
                    }
                    // If current_value is same as last_known, or an unexpected value (not 0 or 1), just continue polling.
                }
                Ok(Err(io_err)) => {
                    tracing::warn!("Error reading MMIO for command trigger: {}. Continuing.", io_err);
                }
                Err(join_err) => {
                    tracing::warn!("Task join error for MMIO command trigger read: {}. Continuing.", join_err);
                }
            }
            time::sleep(Duration::from_millis(POLLING_INTERVAL_MS)).await;
        }
    }

    pub async fn start(mut self) -> Result<(), errors::Error> {
        // last_known_command_trigger_value is initialized to 0 in new().
        // The first call to await_next_command will read the actual current state
        // and detect an edge if the controller has already set it to 1.
        tracing::info!(
            "IPCReader starting. Initial last_known_command_trigger_value is {}.",
            self.last_known_command_trigger_value
        );

        loop {
            tracing::info!("Awaiting next command via MMIO (last known trigger value: {})...", self.last_known_command_trigger_value);
            let cmd = self.await_next_command().await?;
            tracing::info!("Processing command: {:?}", &cmd);

            match cmd {
                Command::Start => {
                    self.simulator_handle.start().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator start command failed: {}", e),
                        }
                    })?;
                    tracing::info!("Simulator acknowledged start. Generating and writing GCR data...");

                    let gcr_data: [u8; 8] = rand::thread_rng().gen();
                    tracing::info!("Generated GCR data: {:?}", gcr_data);

                    self.gcr_file
                        .write_all(&gcr_data)
                        .await
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("Failed to write GCR data to file: {}", e),
                        })?;
                    self.gcr_file.flush().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Failed to flush GCR data to file: {}", e),
                        }
                    })?;
                    tracing::info!("Successfully wrote GCR data. Reading GC...");

                    let gc = self.read_gc_from_file().await?;
                    tracing::info!("Read GC: {}. Seeding simulator...", gc);

                    self.simulator_handle
                        .seed_and_start_generation(gc)
                        .await
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("Simulator seed_and_start_generation failed: {}", e),
                        })?;
                    tracing::info!("Simulator seeded and generation started. Starting writer...");

                    self.writer_handle.start().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("IPC Writer start failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPC Writer started.");
                }
                Command::Stop => {
                    tracing::info!("Stopping IPC Writer...");
                    self.writer_handle.stop().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("IPC Writer stop failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPC Writer stopped. Stopping simulator...");

                    self.simulator_handle.stop().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator stop command failed: {}", e),
                        }
                    })?;
                    tracing::info!("Simulator stopped.");
                }
            }
            tracing::info!("Successfully processed command: {:?}", &cmd);
        }
    }
}
