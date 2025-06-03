pub mod errors;

use memmap2::MmapOptions;
// Removed unused: use rand::Rng;
// Removed unused: use snafu::ResultExt;
use std::fs::OpenOptions as StdOpenOptions;
use std::time::Duration;
use tokio::{
    fs::File,
    io::{AsyncReadExt /* AsyncSeekExt removed, AsyncWriteExt removed */},
    task, time,
};

use crate::{backend::actor::ActorHandle as SimulatorHandle, ipc::Command};
use crate::backend::simulation::BATCH_SIZE; // Use the constant from simulation module

use super::writer::actor::IPCWriterActorHandle;

// --- MMIO Constants ---
const MMIO_MAP_OFFSET: u64 = 0x12000;
const MMIO_MAP_LEN: usize = 0x1000;
const COMMAND_TRIGGER_ADDR_BYTES: usize = 16;
const POLLING_INTERVAL_MS: u64 = 50;

pub struct IPCReader {
    command_path: String,
    gc_read_file: File, // Renamed from gc_file, used for reading GCs from controller
    // gcr_file is removed, as writing GCRs is handled by IPCWriterActor
    writer_handle: IPCWriterActorHandle,
    simulator_handle: SimulatorHandle,
    last_known_command_trigger_value: u32,
    simulator_mode: crate::backend::role::SimulatorMode, // Add simulator_mode field
}

/// Synchronously reads a u32 value from a memory-mapped device.
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

// /// Synchronously writes a u32 value to a memory-mapped device.
// /// This function is intended to be run in a blocking thread.
// fn write_u32_to_mmio(
//     device_path: &str,
//     map_offset: u64,
//     map_len: usize,
//     value_addr_bytes: usize,
//     value: u32,
// ) -> Result<(), std::io::Error> {
//     if value_addr_bytes % 4 != 0 {
//         return Err(std::io::Error::new(
//             std::io::ErrorKind::InvalidInput,
//             "MMIO address must be u32-aligned",
//         ));
//     }
//     if value_addr_bytes + 4 > map_len {
//         return Err(std::io::Error::new(
//             std::io::ErrorKind::InvalidInput,
//             "MMIO address out of bounds for the mapped length",
//         ));
//     }

//     let file = StdOpenOptions::new().read(true).write(true).open(device_path)?;
//     unsafe {
//         let mut mmap = MmapOptions::new()
//             .len(map_len)
//             .offset(map_offset)
//             .map_mut(&file)?; // map_mut for writing
//         let ptr = mmap.as_mut_ptr().add(value_addr_bytes) as *mut u32;
//         ptr.write_volatile(value); // Use write_volatile for MMIO
//         // For some devices/memory types, a flush might be needed.
//         // mmap.flush()?;
//     }
//     Ok(())
// }

impl IPCReader {
    /// Reads a batch of GC values from the gc_read_file.
    /// Expects BATCH_SIZE (1024) u64 values.
    async fn read_gc_batch_from_file(&mut self) -> Result<Vec<u64>, errors::Error> {
        let mut gc_values = Vec::with_capacity(BATCH_SIZE);
        tracing::info!("IPCReader: Attempting to read {} GC values from gc_read_file.", BATCH_SIZE);
        for i in 0..BATCH_SIZE {
            match self.gc_read_file.read_u64_le().await {
                Ok(gc) => gc_values.push(gc),
                Err(e) => {
                    let reason = format!(
                        "Failed to read GC value #{} from gc_read_file (read {} so far): {}",
                        i,
                        gc_values.len(),
                        e
                    );
                    tracing::error!("{}", &reason);
                    // If an error occurs (e.g. FIFO closed or not enough data), return what was read.
                    // The caller might decide if this is a critical error.
                    // For now, we treat partial reads as an error.
                    return Err(errors::Error::Unexpected { reason });
                }
            }
        }
        tracing::info!("IPCReader: Successfully read {} GC values.", gc_values.len());
        Ok(gc_values)
    }

    pub fn new(
        command_path: String,
        gc_read_file: File, // Updated parameter name
        // gcr_file removed
        simulator_handle: SimulatorHandle,
        writer_handle: IPCWriterActorHandle,
        simulator_mode: crate::backend::role::SimulatorMode, // Add simulator_mode parameter
    ) -> Self {
        IPCReader {
            command_path,
            gc_read_file,
            // gcr_file field removed
            writer_handle,
            simulator_handle,
            last_known_command_trigger_value: 0,
            simulator_mode, // Store simulator_mode
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
            tracing::info!("IPCReader: Processing command: {:?}", &cmd);

            match cmd {
                Command::Start => {
                    tracing::info!("IPCReader: Start command received. Initiating generation loop.");
                    // Tell Simulator to prepare for a new session
                    self.simulator_handle.start_session().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator start_session failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPCReader: Simulator session started.");

                    // Main generation and IPC loop
                    loop {
                        match self.simulator_mode {
                            crate::backend::role::SimulatorMode::Detector => {
                                // --- Detector Mode Flow (hw_sim) ---
                                // 1. Tell SimulatorActor to generate GCR and angles batch (calculates GCs internally)
                                tracing::debug!("IPCReader (Detector): Requesting GCR and angles batch from simulator...");
                                let gcr_batch = self.simulator_handle.generate_gcr_and_angles_batch().await.map_err(|e| {
                                    errors::Error::Unexpected {
                                        reason: format!("Simulator generate_gcr_and_angles_batch failed: {}", e),
                                    }
                                })?;
                                tracing::info!("IPCReader (Detector): Received GCR batch ({} items) from simulator.", gcr_batch.len());

                                // 2. Tell IPCWriterActor to write the GCR batch
                                tracing::debug!("IPCReader (Detector): Sending GCR batch to writer...");
                                self.writer_handle.write_gcr_batch(gcr_batch).await.map_err(|e| {
                                    errors::Error::Unexpected {
                                        reason: format!("IPCWriter write_gcr_batch failed: {}", e),
                                    }
                                })?;
                                tracing::info!("IPCReader (Detector): GCR batch sent to writer.");

                                // 3. Read a batch of GC values (echoed) from gc_read_file (from simu_controller)
                                tracing::debug!("IPCReader (Detector): Reading echoed GC batch from controller...");
                                let echoed_gc_values = self.read_gc_batch_from_file().await?;
                                tracing::info!("IPCReader (Detector): Received echoed GC batch ({} items) from controller.", echoed_gc_values.len());
                                if echoed_gc_values.len() != BATCH_SIZE {
                                    let reason = format!("Expected {} echoed GC values from controller, got {}. Stopping.", BATCH_SIZE, echoed_gc_values.len());
                                    tracing::error!("{}", reason);
                                    return Err(errors::Error::Unexpected{ reason });
                                }

                                // 4. Tell SimulatorActor to retrieve pending angles, passing echoed GCs
                                tracing::debug!("IPCReader (Detector): Requesting pending angles batch from simulator...");
                                let angles_batch = self.simulator_handle.retrieve_pending_angles_batch(echoed_gc_values).await.map_err(|e| {
                                    errors::Error::Unexpected {
                                        reason: format!("Simulator retrieve_pending_angles_batch failed: {}", e),
                                    }
                                })?;
                                tracing::info!("IPCReader (Detector): Received angles batch ({} bytes) from simulator.", angles_batch.len());

                                // 5. Tell IPCWriterActor to write the angles batch
                                tracing::debug!("IPCReader (Detector): Sending angles batch to writer...");
                                self.writer_handle.write_angles_batch(angles_batch).await.map_err(|e| {
                                    errors::Error::Unexpected {
                                        reason: format!("IPCWriter write_angles_batch failed: {}", e),
                                    }
                                })?;
                                tracing::info!("IPCReader (Detector): Angles batch sent to writer.");
                            }
                            crate::backend::role::SimulatorMode::Source => {
                                // --- Source Mode Flow (hw_sim) ---
                                // 1. Read a batch of GC values from gc_read_file (from simu_controller)
                                tracing::debug!("IPCReader (Source): Reading GC batch from controller...");
                                let received_gc_values = self.read_gc_batch_from_file().await?;
                                tracing::info!("IPCReader (Source): Received GC batch ({} items) from controller.", received_gc_values.len());
                                if received_gc_values.len() != BATCH_SIZE {
                                    let reason = format!("Expected {} GC values from controller, got {}. Stopping.", BATCH_SIZE, received_gc_values.len());
                                    tracing::error!("{}", reason);
                                    return Err(errors::Error::Unexpected{ reason });
                                }

                                // 2. Tell SimulatorActor to generate angles based on these GCs
                                tracing::debug!("IPCReader (Source): Requesting angles batch from simulator using received GCs...");
                                let angles_batch = self.simulator_handle.generate_angles_for_gcs(received_gc_values).await.map_err(|e| {
                                    errors::Error::Unexpected {
                                        reason: format!("Simulator generate_angles_for_gcs failed: {}", e),
                                    }
                                })?;
                                tracing::info!("IPCReader (Source): Received angles batch ({} bytes) from simulator.", angles_batch.len());

                                // 3. Tell IPCWriterActor to write the angles batch
                                tracing::debug!("IPCReader (Source): Sending angles batch to writer...");
                                self.writer_handle.write_angles_batch(angles_batch).await.map_err(|e| {
                                    errors::Error::Unexpected {
                                        reason: format!("IPCWriter write_angles_batch failed: {}", e),
                                    }
                                })?;
                                tracing::info!("IPCReader (Source): Angles batch sent to writer.");
                                // GCR generation/writing is skipped in Source mode.
                            }
                        }
                        // Check for stop command before starting next iteration
                        // This requires a non-blocking check or integrating MMIO polling into this loop.
                        // For simplicity, this example assumes the loop continues until an external Stop command
                        // is detected by await_next_command in the *next* outer loop iteration,
                        // or an error occurs. A more responsive stop would require refactoring await_next_command
                        // or using a select! macro here.
                        // For now, if a new command is available (e.g. Stop), it will be picked up
                        // after this iteration completes and the outer loop calls await_next_command() again.
                        // If an error occurs in this loop, it will break out and the Stop command logic below will be hit.
                    }
                    // The loop above is infinite until an error or a Stop command is processed by the outer mechanism.
                }
                Command::Stop => {
                    tracing::info!("IPCReader: Stop command received.");
                    tracing::info!("IPCReader: Telling Simulator to stop session...");
                    self.simulator_handle.stop_session().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator stop_session failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPCReader: Simulator session stopped.");

                    tracing::info!("IPCReader: Telling IPCWriter to stop...");
                    self.writer_handle.stop().await.map_err(|e| { // Assuming writer has a stop for cleanup
                        errors::Error::Unexpected {
                            reason: format!("IPCWriter stop failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPCReader: IPCWriter stop signal sent.");
                    tracing::info!("IPCReader: Successfully processed Stop command. Exiting current command processing loop.");
                    return Ok(()); // Exit the start() method, will lead to "Current IPC session ended"
                }
            }
            // This log might not be reached if Start leads to an infinite loop that only breaks on error or explicit return.
            // tracing::info!("IPCReader: Successfully processed command: {:?}", &cmd);
        }
    }
}
