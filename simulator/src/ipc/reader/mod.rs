pub mod errors;

use memmap2::MmapOptions;
use std::fs::OpenOptions as StdOpenOptions;
use std::time::Duration;
use std::{fs::File, io::Read};

use crate::backend::simulation::BATCH_SIZE;
use crate::{backend::actor::ActorHandle as SimulatorHandle, ipc::Command};

use super::writer::actor::IPCWriterActorHandle;

// --- MMIO Constants ---
const MMIO_MAP_OFFSET: u64 = 0x12000;
const MMIO_MAP_LEN: usize = 0x1000;
const COMMAND_TRIGGER_ADDR_BYTES: usize = 16;
const POLLING_INTERVAL_MS: u64 = 50;

pub struct IPCReader {
    command_path: String, // Path is now mandatory
    gc_read_file: File,
    writer_handle: IPCWriterActorHandle,
    simulator_handle: SimulatorHandle,
    last_known_command_trigger_value: u32,
    simulator_mode: crate::backend::role::SimulatorMode,
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
        Ok(ptr.read_volatile())
    }
}

impl IPCReader {
    /// Reads a batch of GC values from the gc_read_file.
    /// Expects BATCH_SIZE (1024) 16-byte records, and extracts a u64 GC from the first 8 bytes of each.
    async fn read_gc_batch_from_file(&mut self) -> Result<Vec<u64>, errors::Error> {
        let mut gc_values = Vec::with_capacity(BATCH_SIZE);
        let mut record_buffer = [0u8; 16];
        tracing::debug!(
            "IPCReader: Attempting to read {} 16-byte GC records from gc_read_file.",
            BATCH_SIZE
        );
        for i in 0..BATCH_SIZE {
            match self.gc_read_file.read_exact(&mut record_buffer) {
                Ok(_) => {
                    let gc_bytes: [u8; 8] = record_buffer[0..8].try_into().unwrap();
                    gc_values.push(u64::from_le_bytes(gc_bytes));
                }
                Err(e) => {
                    let reason = format!(
                        "Failed to read 16-byte record #{} from gc_read_file (read {} so far): {}",
                        i,
                        gc_values.len(),
                        e
                    );
                    tracing::error!("{}", &reason);
                    return Err(errors::Error::Unexpected { reason });
                }
            }
        }
        tracing::debug!(
            "IPCReader: Successfully read {} GC values.",
            gc_values.len()
        );
        Ok(gc_values)
    }

    pub fn new(
        command_path: String,
        gc_read_file: File,
        simulator_handle: SimulatorHandle,
        writer_handle: IPCWriterActorHandle,
        simulator_mode: crate::backend::role::SimulatorMode,
    ) -> Self {
        IPCReader {
            command_path,
            gc_read_file,
            writer_handle,
            simulator_handle,
            last_known_command_trigger_value: 0,
            simulator_mode,
        }
    }

    async fn await_next_command(&mut self) -> Result<Command, errors::Error> {
        loop {
            let device_path_clone = self.command_path.clone();
            let read_result = read_u32_from_mmio(
                &device_path_clone,
                MMIO_MAP_OFFSET,
                MMIO_MAP_LEN,
                COMMAND_TRIGGER_ADDR_BYTES,
            );

            match read_result {
                Ok(current_value) => {
                    if current_value == 1 && self.last_known_command_trigger_value == 0 {
                        tracing::info!(
                            "Start command detected via MMIO (0->1 transition at addr {:#X}).",
                            COMMAND_TRIGGER_ADDR_BYTES
                        );
                        self.last_known_command_trigger_value = 1;
                        return Ok(Command::Start);
                    } else if current_value == 0 && self.last_known_command_trigger_value == 1 {
                        tracing::info!(
                            "Stop command detected via MMIO (1->0 transition at addr {:#X}).",
                            COMMAND_TRIGGER_ADDR_BYTES
                        );
                        self.last_known_command_trigger_value = 0;
                        return Ok(Command::Stop);
                    } else if current_value != self.last_known_command_trigger_value
                        && (current_value == 0 || current_value == 1)
                    {
                        tracing::debug!(
                            "MMIO command trigger changed to {} without a processed edge from {}. Updating last known value.",
                            current_value, self.last_known_command_trigger_value
                        );
                        self.last_known_command_trigger_value = current_value;
                    }
                }
                Err(join_err) => {
                    tracing::warn!(
                        "Task join error for MMIO command trigger read: {}. Continuing.",
                        join_err
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(POLLING_INTERVAL_MS));
        }
    }

    /// Runs the Detector (Bob) workflow.
    async fn run_detector_workflow(&mut self) -> Result<(), errors::Error> {
        self.last_known_command_trigger_value = 0; // Initialize for command polling

        loop {
            tracing::info!(
                "IPCReader (Bob): Awaiting next command via MMIO (last known trigger value: {})...",
                self.last_known_command_trigger_value
            );
            let cmd = self.await_next_command().await?;
            tracing::info!("IPCReader (Bob): Processing command: {:?}", &cmd);

            match cmd {
                Command::Start => {
                    tracing::info!(
                        "IPCReader (Bob): Start command received. Initiating generation loop."
                    );
                    self.simulator_handle.start_session().map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator start_session failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPCReader (Bob): Simulator session started.");

                    loop {
                        tracing::debug!(
                            "IPCReader (Bob): Requesting GCR and angles batch from simulator..."
                        );
                        let gcr_data_to_write = self
                            .simulator_handle
                            .generate_gcr_and_angles_batch()
                            .await
                            .map_err(|e| errors::Error::Unexpected {
                                reason: format!(
                                    "Simulator generate_gcr_and_angles_batch failed: {}",
                                    e
                                ),
                            })?;
                        tracing::info!(
                            "IPCReader (Bob): Received GCR data batch ({} items) from simulator.",
                            gcr_data_to_write.len()
                        );

                        tracing::debug!("IPCReader (Bob): Sending GCR batch to writer...");
                        self.writer_handle
                            .write_gcr_batch(gcr_data_to_write)
                            .await
                            .map_err(|e| errors::Error::Unexpected {
                                reason: format!("IPCWriter write_gcr_batch failed: {}", e),
                            })?;
                        tracing::info!("IPCReader (Bob): GCR batch sent to writer.");

                        tracing::debug!(
                            "IPCReader (Bob): Reading echoed GC batch from controller..."
                        );
                        let echoed_gc_values = match self.read_gc_batch_from_file().await {
                            Ok(vals) => vals,
                            Err(e) => {
                                tracing::warn!("IPCReader (Bob): Failed to read GC batch, ending generation loop. Error: {}", e);
                                break;
                            }
                        };
                        tracing::info!(
                            "IPCReader (Bob): Received echoed GC batch ({} items) from controller.",
                            echoed_gc_values.len()
                        );
                        if let (Some(first), Some(last)) =
                            (echoed_gc_values.first(), echoed_gc_values.last())
                        {
                            tracing::debug!(
                                "IPCReader (Bob): Echoed GCs - First: {}, Last: {}",
                                first,
                                last
                            );
                        }
                        if echoed_gc_values.len() != BATCH_SIZE {
                            let reason = format!(
                                "Expected {} echoed GC values from controller, got {}. Stopping.",
                                BATCH_SIZE,
                                echoed_gc_values.len()
                            );
                            tracing::error!("{}", reason);
                            self.simulator_handle.stop_session().await.ok();
                            return Err(errors::Error::Unexpected { reason });
                        }

                        tracing::debug!(
                            "IPCReader (Bob): Requesting pending angles batch from simulator..."
                        );
                        let angles_batch = self
                            .simulator_handle
                            .retrieve_pending_angles_batch(echoed_gc_values)
                            .await
                            .map_err(|e| errors::Error::Unexpected {
                                reason: format!(
                                    "Simulator retrieve_pending_angles_batch failed: {}",
                                    e
                                ),
                            })?;
                        tracing::info!(
                            "IPCReader (Bob): Received angles batch ({} bytes) from simulator.",
                            angles_batch.len()
                        );

                        tracing::debug!("IPCReader (Bob): Sending angles batch to writer...");
                        self.writer_handle
                            .write_angles_batch(angles_batch)
                            .await
                            .map_err(|e| errors::Error::Unexpected {
                                reason: format!("IPCWriter write_angles_batch failed: {}", e),
                            })?;
                        tracing::info!("IPCReader (Bob): Angles batch sent to writer.");
                    }

                    tracing::info!("IPCReader (Bob): Generation loop finished. Stopping session.");
                    self.simulator_handle.stop_session().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!(
                                "Simulator stop_session failed after generation loop: {}",
                                e
                            ),
                        }
                    })?;
                }
                Command::Stop => {
                    tracing::info!("IPCReader (Bob): Stop command received.");
                    self.simulator_handle.stop_session().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator stop_session failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPCReader (Bob): Simulator session stopped.");

                    self.writer_handle
                        .stop()
                        .await
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("IPCWriter stop failed: {}", e),
                        })?;
                    tracing::info!("IPCReader (Bob): IPCWriter stop signal sent.");
                    tracing::info!(
                        "IPCReader (Bob): Successfully processed Stop command. Exiting."
                    );
                    return Ok(());
                }
            }
        }
    }

    /// Runs the Source (Alice) workflow.
    async fn run_source_workflow(&mut self) -> Result<(), errors::Error> {
        self.last_known_command_trigger_value = 0;

        loop {
            tracing::info!(
                "Awaiting next command via MMIO (last known trigger value: {})...",
                self.last_known_command_trigger_value
            );
            let cmd = self.await_next_command().await?;
            tracing::info!("IPCReader (Alice): Processing command: {:?}", &cmd);

            match cmd {
                Command::Start => {
                    tracing::info!(
                        "IPCReader (Alice): Start command received. Initiating generation loop."
                    );
                    self.simulator_handle.start_session().map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator start_session failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPCReader (Alice): Simulator session started.");

                    loop {
                        tracing::debug!("IPCReader (Alice): Reading GC batch from gc_client...");
                        let received_gc_values = match self.read_gc_batch_from_file().await {
                            Ok(vals) => vals,
                            Err(e) => {
                                tracing::warn!("IPCReader (Alice): Failed to read GC batch, ending generation loop. Error: {}", e);
                                break;
                            }
                        };
                        tracing::info!(
                            "IPCReader (Alice): Received GC batch ({} items) from gc_client.",
                            received_gc_values.len()
                        );
                        if let (Some(first), Some(last)) =
                            (received_gc_values.first(), received_gc_values.last())
                        {
                            tracing::debug!(
                                "IPCReader (Alice): Received GCs - First: {}, Last: {}",
                                first,
                                last
                            );
                        }
                        if received_gc_values.len() != BATCH_SIZE {
                            let reason = format!(
                                "Expected {} GC values from gc_client, got {}. Stopping.",
                                BATCH_SIZE,
                                received_gc_values.len()
                            );
                            tracing::error!("{}", reason);
                            self.simulator_handle.stop_session().await.ok();
                            return Err(errors::Error::Unexpected { reason });
                        }

                        tracing::debug!("IPCReader (Alice): Requesting angles batch from simulator using received GCs...");
                        let angles_batch = self
                            .simulator_handle
                            .generate_angles_for_gcs(received_gc_values)
                            .await
                            .map_err(|e| errors::Error::Unexpected {
                                reason: format!("Simulator generate_angles_for_gcs failed: {}", e),
                            })?;
                        tracing::info!(
                            "IPCReader (Alice): Received angles batch ({} bytes) from simulator.",
                            angles_batch.len()
                        );

                        tracing::debug!("IPCReader (Alice): Sending angles batch to writer...");
                        self.writer_handle
                            .write_angles_batch(angles_batch)
                            .await
                            .map_err(|e| errors::Error::Unexpected {
                                reason: format!("IPCWriter write_angles_batch failed: {}", e),
                            })?;
                        tracing::info!("IPCReader (Alice): Angles batch sent to writer.");
                    }

                    tracing::info!(
                        "IPCReader (Alice): Generation loop finished. Stopping session."
                    );
                    self.simulator_handle.stop_session().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!(
                                "Simulator stop_session failed after generation loop: {}",
                                e
                            ),
                        }
                    })?;
                }
                Command::Stop => {
                    tracing::info!("IPCReader (Alice): Stop command received.");
                    self.simulator_handle.stop_session().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator stop_session failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPCReader (Alice): Simulator session stopped.");

                    self.writer_handle
                        .stop()
                        .await
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("IPCWriter stop failed: {}", e),
                        })?;
                    tracing::info!("IPCReader (Alice): IPCWriter stop signal sent.");
                    tracing::info!("IPCReader (Alice): Successfully processed Stop command. Exiting current command processing loop.");
                    return Ok(());
                }
            }
        }
    }

    pub async fn start(mut self) -> Result<(), errors::Error> {
        match self.simulator_mode {
            crate::backend::role::SimulatorMode::Detector => {
                tracing::info!("IPCReader starting in Detector (Bob) mode. Awaiting commands.");
                self.run_detector_workflow().await
            }
            crate::backend::role::SimulatorMode::Source => {
                tracing::info!("IPCReader starting in Source (Alice) mode. Awaiting commands.");
                self.run_source_workflow().await
            }
        }
    }
}
