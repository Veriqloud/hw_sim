pub mod errors;

use memmap2::MmapOptions;
use sim_lib::hardware::modes::SimulatorMode;
use sim_lib::simulation::batches::QkdBatch;
use sim_lib::BATCH_SIZE;
use std::collections::VecDeque;
use std::fs::OpenOptions as StdOpenOptions;
use std::time::Duration;
use std::{fs::File, io::Read};

use crate::{
    backend::actor::ActorHandle as SimulatorHandle,
    ipc::Command,
    runtime_control::{RuntimeCommand, RuntimeControl},
};

use super::writer::actor::IPCWriterActorHandle;

// --- MMIO Constants ---
const INIT_RESET_MMIO_MAP_OFFSET: u64 = 0x12000;
const COMMAND_TRIGGER_ADDR_BYTES: usize = 16;
const GENERATION_START_MMIO_MAP_OFFSET: u64 = 0x1000;
const MMIO_MAP_LEN: usize = 0x1000;
const GENERATION_START_ADDR_BYTES: usize = 24;
const POLLING_INTERVAL_MS: u64 = 50;

pub struct IPCReader {
    command_path: String,
    gc_read_file: File,
    writer_handle: IPCWriterActorHandle,
    simulator_handle: SimulatorHandle,
    last_known_command_trigger_value: u32,
    last_known_init_reset_value: u32,
    simulator_mode: SimulatorMode,
    /// Pre-generated batches waiting to be consumed by the next GC arrival.
    batch_queue: VecDeque<QkdBatch>,
    /// Whether to interleave 8-byte zero pads between GCR records (hardware protocol).
    use_gcr_padding: bool,
    runtime_control: RuntimeControl,
}

/// Synchronously reads a u32 value from a memory-mapped device.
fn read_u32_from_mmio(
    device_path: &str,
    map_offset: u64,
    map_len: usize,
    value_addr_bytes: usize,
) -> Result<u32, std::io::Error> {
    if !value_addr_bytes.is_multiple_of(4) {
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

fn update_binary_mmio_state(last_known_value: &mut u32, current_value: u32) -> (u32, u32) {
    let previous_value = *last_known_value;
    if current_value == 0 || current_value == 1 {
        *last_known_value = current_value;
    }
    (previous_value, current_value)
}

fn classify_generation_start_transition(
    last_known_value: &mut u32,
    current_value: u32,
) -> Option<Command> {
    let (previous_value, current_value) = update_binary_mmio_state(last_known_value, current_value);

    if current_value == 1 && previous_value == 0 {
        Some(Command::Start)
    } else if current_value == 0 && previous_value == 1 {
        Some(Command::Stop)
    } else {
        None
    }
}

impl IPCReader {
    /// Reads a batch of GC values from the gc_read_file.
    /// Expects BATCH_SIZE (1024) 16-byte records, and extracts a u64 GC from the first 8 bytes of each.
    fn read_gc_batch_from_file(&mut self) -> Result<Vec<u64>, errors::Error> {
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

    /// Pops the next batch from the queue, generating a fresh one if the queue is empty.
    fn next_batch(&mut self) -> Result<QkdBatch, errors::Error> {
        if let Some(batch) = self.batch_queue.pop_front() {
            return Ok(batch);
        }
        self.simulator_handle
            .generate_qkd_batch()
            .map_err(|e| errors::Error::Unexpected {
                reason: format!("generate_qkd_batch failed: {}", e),
            })
    }

    pub fn new(
        command_path: String,
        gc_read_file: File,
        simulator_handle: SimulatorHandle,
        writer_handle: IPCWriterActorHandle,
        simulator_mode: SimulatorMode,
        runtime_control: RuntimeControl,
    ) -> Self {
        let use_gcr_padding = simulator_handle.use_gcr_padding;
        IPCReader {
            command_path,
            gc_read_file,
            writer_handle,
            simulator_handle,
            last_known_command_trigger_value: 0,
            last_known_init_reset_value: 0,
            simulator_mode,
            batch_queue: VecDeque::new(),
            use_gcr_padding,
            runtime_control,
        }
    }

    fn check_runtime_pause(&mut self) -> Result<(), errors::Error> {
        match self.runtime_control.try_recv() {
            Some(RuntimeCommand::Pause { duration }) => {
                tracing::info!("IPCReader: Runtime pause requested for {:?}.", duration);
                self.batch_queue.clear();
                self.simulator_handle
                    .stop_session()
                    .map_err(|e| errors::Error::Unexpected {
                        reason: format!("Simulator stop_session failed before pause: {}", e),
                    })?;
                self.writer_handle
                    .stop()
                    .map_err(|e| errors::Error::Unexpected {
                        reason: format!("IPCWriter stop failed before pause: {}", e),
                    })?;
                Err(errors::Error::PauseRequested { duration })
            }
            None => Ok(()),
        }
    }

    fn observe_init_reset_transition(&mut self, current_value: u32) {
        let previous_value = self.last_known_init_reset_value;
        if current_value == 1 && self.last_known_init_reset_value == 0 {
            tracing::info!(
                "Init/reset detected via MMIO (0->1 transition at map offset {:#X}, addr {:#X}); generation is not started by this signal.",
                INIT_RESET_MMIO_MAP_OFFSET,
                COMMAND_TRIGGER_ADDR_BYTES
            );
        } else if current_value == 0 && self.last_known_init_reset_value == 1 {
            tracing::info!(
                "Init/reset deasserted via MMIO (1->0 transition at map offset {:#X}, addr {:#X}).",
                INIT_RESET_MMIO_MAP_OFFSET,
                COMMAND_TRIGGER_ADDR_BYTES
            );
        }

        update_binary_mmio_state(&mut self.last_known_init_reset_value, current_value);
        if current_value != previous_value && current_value != 0 && current_value != 1 {
            tracing::debug!(
                "Ignoring non-binary init/reset MMIO value {} at map offset {:#X}, addr {:#X}.",
                current_value,
                INIT_RESET_MMIO_MAP_OFFSET,
                COMMAND_TRIGGER_ADDR_BYTES
            );
        }
    }

    fn observe_generation_start_transition(&mut self, current_value: u32) -> Option<Command> {
        let previous_value = self.last_known_command_trigger_value;
        let command = classify_generation_start_transition(
            &mut self.last_known_command_trigger_value,
            current_value,
        );

        match command {
            Some(Command::Start) => {
                tracing::info!(
                    "Generation start detected via MMIO (0->1 transition at map offset {:#X}, addr {:#X}).",
                    GENERATION_START_MMIO_MAP_OFFSET,
                    GENERATION_START_ADDR_BYTES
                );
            }
            Some(Command::Stop) => {
                tracing::info!(
                    "Generation stop detected via MMIO (1->0 transition at map offset {:#X}, addr {:#X}).",
                    GENERATION_START_MMIO_MAP_OFFSET,
                    GENERATION_START_ADDR_BYTES
                );
            }
            None if current_value != previous_value && current_value != 0 && current_value != 1 => {
                tracing::debug!(
                    "Ignoring non-binary generation start MMIO value {} at map offset {:#X}, addr {:#X}.",
                    current_value,
                    GENERATION_START_MMIO_MAP_OFFSET,
                    GENERATION_START_ADDR_BYTES
                );
            }
            None => {}
        }
        command
    }

    fn await_next_command(&mut self) -> Result<Command, errors::Error> {
        loop {
            self.check_runtime_pause()?;

            let device_path_clone = self.command_path.clone();
            let init_reset_read_result = read_u32_from_mmio(
                &device_path_clone,
                INIT_RESET_MMIO_MAP_OFFSET,
                MMIO_MAP_LEN,
                COMMAND_TRIGGER_ADDR_BYTES,
            );

            match init_reset_read_result {
                Ok(current_value) => self.observe_init_reset_transition(current_value),
                Err(join_err) => {
                    tracing::warn!(
                        "Task join error for MMIO init/reset read: {}. Continuing.",
                        join_err
                    );
                }
            }

            let generation_start_read_result = read_u32_from_mmio(
                &device_path_clone,
                GENERATION_START_MMIO_MAP_OFFSET,
                MMIO_MAP_LEN,
                GENERATION_START_ADDR_BYTES,
            );

            match generation_start_read_result {
                Ok(current_value) => {
                    if let Some(command) = self.observe_generation_start_transition(current_value) {
                        return Ok(command);
                    }
                }
                Err(join_err) => {
                    tracing::warn!(
                        "Task join error for MMIO generation start read: {}. Continuing.",
                        join_err
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(POLLING_INTERVAL_MS));
        }
    }

    /// Runs the Detector (Bob) workflow.
    fn run_detector_workflow(&mut self) -> Result<(), errors::Error> {
        self.last_known_command_trigger_value = 0;
        self.last_known_init_reset_value = 0;

        loop {
            tracing::info!(
                "IPCReader (Bob): Awaiting next command via MMIO (last known trigger value: {})...",
                self.last_known_command_trigger_value
            );
            let cmd = self.await_next_command()?;
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
                    self.batch_queue.clear();
                    tracing::info!("IPCReader (Bob): Simulator session started.");

                    loop {
                        self.check_runtime_pause()?;

                        // Pop a pre-generated batch or generate a fresh one.
                        let batch = match self.next_batch() {
                            Ok(b) => b,
                            Err(e) => {
                                tracing::warn!("IPCReader (Bob): Failed to get batch, ending generation loop. Error: {}", e);
                                break;
                            }
                        };

                        let gcr_data = batch.to_gcr_batch(self.use_gcr_padding);
                        tracing::info!(
                            "IPCReader (Bob): Sending GCR batch ({} items) to writer.",
                            gcr_data.len()
                        );
                        self.writer_handle.write_gcr_batch(gcr_data).map_err(|e| {
                            errors::Error::Unexpected {
                                reason: format!("IPCWriter write_gcr_batch failed: {}", e),
                            }
                        })?;

                        self.check_runtime_pause()?;

                        let echoed_gc_values = match self.read_gc_batch_from_file() {
                            Ok(vals) => vals,
                            Err(e) => {
                                tracing::warn!("IPCReader (Bob): Failed to read echoed GC batch, ending generation loop. Error: {}", e);
                                break;
                            }
                        };
                        tracing::info!(
                            "IPCReader (Bob): Received echoed GC batch ({} items) from controller.",
                            echoed_gc_values.len()
                        );

                        if echoed_gc_values.len() != BATCH_SIZE {
                            let reason = format!(
                                "Expected {} echoed GC values from controller, got {}. Stopping.",
                                BATCH_SIZE,
                                echoed_gc_values.len()
                            );
                            tracing::error!("{}", reason);
                            self.simulator_handle.stop_session().ok();
                            return Err(errors::Error::Unexpected { reason });
                        }

                        self.check_runtime_pause()?;

                        let angles = batch.to_bob_angle_fifo();
                        tracing::info!(
                            "IPCReader (Bob): Sending angles batch ({} bytes) to writer.",
                            angles.len()
                        );
                        self.writer_handle.write_angles_batch(angles).map_err(|e| {
                            errors::Error::Unexpected {
                                reason: format!("IPCWriter write_angles_batch failed: {}", e),
                            }
                        })?;
                    }

                    tracing::info!("IPCReader (Bob): Generation loop finished. Stopping session.");
                    self.simulator_handle.stop_session().map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!(
                                "Simulator stop_session failed after generation loop: {}",
                                e
                            ),
                        }
                    })?;
                    // The generation loop only ends when a FIFO peer detached, i.e. the
                    // controller session is gone. Tear down completely so the workflow
                    // loop resets the FIFOs and blocks in open() until the next session
                    // attaches; keeping the stale handles and edge-detector state here
                    // would make every subsequent Start undetectable or abort instantly.
                    self.writer_handle
                        .stop()
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("IPCWriter stop failed: {}", e),
                        })?;
                    tracing::info!(
                        "IPCReader (Bob): Session peers detached. Exiting for FIFO reset."
                    );
                    return Ok(());
                }
                Command::Stop => {
                    tracing::info!("IPCReader (Bob): Stop command received.");
                    self.simulator_handle.stop_session().map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator stop_session failed: {}", e),
                        }
                    })?;
                    self.writer_handle
                        .stop()
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("IPCWriter stop failed: {}", e),
                        })?;
                    tracing::info!(
                        "IPCReader (Bob): Successfully processed Stop command. Exiting."
                    );
                    return Ok(());
                }
            }
        }
    }

    /// Runs the Source (Alice) workflow.
    fn run_source_workflow(&mut self) -> Result<(), errors::Error> {
        self.last_known_command_trigger_value = 0;
        self.last_known_init_reset_value = 0;

        loop {
            tracing::info!(
                "Awaiting next command via MMIO (last known trigger value: {})...",
                self.last_known_command_trigger_value
            );
            let cmd = self.await_next_command()?;
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
                    self.batch_queue.clear();
                    tracing::info!("IPCReader (Alice): Simulator session started.");

                    loop {
                        self.check_runtime_pause()?;

                        let received_gc_values = match self.read_gc_batch_from_file() {
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

                        if received_gc_values.len() != BATCH_SIZE {
                            let reason = format!(
                                "Expected {} GC values from gc_client, got {}. Stopping.",
                                BATCH_SIZE,
                                received_gc_values.len()
                            );
                            tracing::error!("{}", reason);
                            self.simulator_handle.stop_session().ok();
                            return Err(errors::Error::Unexpected { reason });
                        }

                        self.check_runtime_pause()?;

                        // Pop a pre-generated batch or generate a fresh one.
                        let batch = match self.next_batch() {
                            Ok(b) => b,
                            Err(e) => {
                                tracing::warn!("IPCReader (Alice): Failed to get batch, ending generation loop. Error: {}", e);
                                break;
                            }
                        };

                        self.check_runtime_pause()?;

                        let angles = batch.to_alice_fifo();
                        tracing::info!(
                            "IPCReader (Alice): Sending angles batch ({} bytes) to writer.",
                            angles.len()
                        );
                        self.writer_handle.write_angles_batch(angles).map_err(|e| {
                            errors::Error::Unexpected {
                                reason: format!("IPCWriter write_angles_batch failed: {}", e),
                            }
                        })?;
                    }

                    tracing::info!(
                        "IPCReader (Alice): Generation loop finished. Stopping session."
                    );
                    self.simulator_handle.stop_session().map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!(
                                "Simulator stop_session failed after generation loop: {}",
                                e
                            ),
                        }
                    })?;
                    // See the detector workflow: a finished generation loop means the
                    // session peers detached, so exit for a full FIFO reset instead of
                    // waiting for a Start edge on stale handles.
                    self.writer_handle
                        .stop()
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("IPCWriter stop failed: {}", e),
                        })?;
                    tracing::info!(
                        "IPCReader (Alice): Session peers detached. Exiting for FIFO reset."
                    );
                    return Ok(());
                }
                Command::Stop => {
                    tracing::info!("IPCReader (Alice): Stop command received.");
                    self.simulator_handle.stop_session().map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator stop_session failed: {}", e),
                        }
                    })?;
                    self.writer_handle
                        .stop()
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("IPCWriter stop failed: {}", e),
                        })?;
                    tracing::info!(
                        "IPCReader (Alice): Successfully processed Stop command. Exiting."
                    );
                    return Ok(());
                }
            }
        }
    }

    pub fn start(mut self) -> Result<(), errors::Error> {
        match self.simulator_mode {
            SimulatorMode::Detector => {
                tracing::info!("IPCReader starting in Detector (Bob) mode. Awaiting commands.");
                self.run_detector_workflow()
            }
            SimulatorMode::Source => {
                tracing::info!("IPCReader starting in Source (Alice) mode. Awaiting commands.");
                self.run_source_workflow()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_generation_start_transition, update_binary_mmio_state, COMMAND_TRIGGER_ADDR_BYTES,
        GENERATION_START_ADDR_BYTES, GENERATION_START_MMIO_MAP_OFFSET, INIT_RESET_MMIO_MAP_OFFSET,
    };
    use crate::ipc::Command;

    #[test]
    fn init_reset_transition_only_updates_reset_state() {
        assert_eq!(INIT_RESET_MMIO_MAP_OFFSET, 0x12000);
        assert_eq!(COMMAND_TRIGGER_ADDR_BYTES, 16);

        let mut init_reset_state = 0;
        let mut generation_state = 0;

        let init_transition = update_binary_mmio_state(&mut init_reset_state, 1);
        let command = classify_generation_start_transition(&mut generation_state, 0);

        assert_eq!(init_transition, (0, 1));
        assert_eq!(init_reset_state, 1);
        assert!(command.is_none());
        assert_eq!(generation_state, 0);
    }

    #[test]
    fn generation_start_uses_sync_at_pps_register() {
        assert_eq!(GENERATION_START_MMIO_MAP_OFFSET, 0x1000);
        assert_eq!(GENERATION_START_ADDR_BYTES, 24);

        let mut generation_state = 0;
        let command = classify_generation_start_transition(&mut generation_state, 1);

        assert!(matches!(command, Some(Command::Start)));
        assert_eq!(generation_state, 1);
    }

    #[test]
    fn generation_stop_is_detected_on_generation_register_falling_edge() {
        let mut generation_state = 1;
        let command = classify_generation_start_transition(&mut generation_state, 0);

        assert!(matches!(command, Some(Command::Stop)));
        assert_eq!(generation_state, 0);
    }

    #[test]
    fn non_binary_mmio_values_do_not_change_generation_state() {
        let mut generation_state = 0;
        let command = classify_generation_start_transition(&mut generation_state, 42);

        assert!(command.is_none());
        assert_eq!(generation_state, 0);
    }
}
