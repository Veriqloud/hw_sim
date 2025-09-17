use clap::Parser;
// use hw_sim::backend::role::SimulatorMode; // Removed import
use memmap2::MmapOptions;
// use rand::Rng; // No longer needed for Source mode GC generation
use serde::Deserialize;
use std::fs::OpenOptions as StdOpenOptions; // For synchronous file operations in xdma_write
// use std::thread; // Replaced with tokio::time::sleep
use std::time as std_time; // For std_time::Instant and std_time::Duration
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, Duration};

// Command-line arguments structure
#[derive(Parser, Debug)]
#[clap(author, version, about = "Simulation Controller CLI", long_about = None)]
struct CliArgs {
    /// Path to the JSON configuration file for IPC settings.
    #[clap(short, long)]
    config_path: String,
}

// MMIO Constants
const COMMAND_TRIGGER_OFFSET: u64 = 0x12000; // Base offset for the command trigger register
const COMMAND_TRIGGER_ADDR_BYTES: usize = 16; // Byte address for the command trigger u32
const MMIO_MAP_LEN: usize = 0x1000; // General memory map length, ensure it covers addresses

// Structs to deserialize the relevant parts of config/valid_config_alice.json
#[derive(Deserialize, Debug)]
struct IpcConfig {
    command_path: String,
    angle_file_path: String,
    // click_result_file_path is removed as it's obsolete in hw_sim
    gc_read_file_path: String, // Renamed from gc_file_path to match hw_sim's ipc_config
    gcr_file_path: String,
}

// Define SimulatorMode directly in this file
#[derive(Deserialize, Debug, PartialEq, Clone, Copy)] // Added PartialEq, Clone, Copy
pub enum SimulatorMode {
    Source,
    Detector,
}

fn default_gc_offset() -> u64 {
    0 // Default GC offset if not specified in config, matching hw_sim's typical default
}

#[derive(Deserialize, Debug)]
struct BackendConfig {
    pulse_distance: f64,
    eta: f64,
    #[serde(default = "default_gc_offset")]
    gc_offset: u64,
}

#[derive(Deserialize, Debug)]
struct ControllerConfig {
    backend_config: BackendConfig, // Added backend_config
    ipc_config: IpcConfig,
    simulator_mode: SimulatorMode,
}

const BATCH_SIZE: usize = 1024; // Matching hw_sim's BATCH_SIZE

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging
    tracing_subscriber::fmt::init();

    let result = async {
        let cli_args = CliArgs::parse();
    let config_path = &cli_args.config_path;

    tracing::info!(
        "SimuController: Reading configuration from '{}'",
        config_path
    );

    let config_content = tokio::fs::read_to_string(config_path).await?;
    let config: ControllerConfig = serde_json::from_str(&config_content)?;

    tracing::info!(
        "SimuController: Parsed Config: IPC={:?}, Mode={:?}",
        config.ipc_config,
        config.simulator_mode
    );

    // Open FIFO files in an order complementary to hw_sim's combined opening sequence.
    // hw_sim effective order for FIFOs:
    // 1. angle_file (write by hw_sim)
    // 2. click_result_file (write by hw_sim)
    // 3. gc_file (read by hw_sim)
    // 4. gcr_file (write by hw_sim)
    // MMIO device (command_path) is read by hw_sim for commands.
    //
    // simu_controller complementary order for FIFOs:
    // 1. angle_file (read by controller)
    // 2. gc_read_file (write by controller, named gc_file internally in this controller)
    // MMIO device (command_path) is written to by controller for commands.

    // 1. Angle file (read-only for controller)
    tracing::info!(
        "SimuController: Opening angle file: {}",
        config.ipc_config.angle_file_path
    );
    let mut angle_file = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.angle_file_path)
        .await?;
    tracing::info!("SimuController: Opened angle file successfully.");

    // GCR file (read-only for controller) - Opened before GC file to match hw_sim writer init
    tracing::info!(
        "SimuController: Opening GCR file: {}",
        config.ipc_config.gcr_file_path
    );
    let mut gcr_file = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.gcr_file_path)
        .await?;
    tracing::info!("SimuController: Opened GCR file successfully.");

    // Global Counter file (write-only for controller)
    tracing::info!(
        "SimuController: Opening GC file (from gc_read_file_path): {}",
        config.ipc_config.gc_read_file_path
    );
    let mut gc_file = OpenOptions::new()
        .write(true)
        .open(&config.ipc_config.gc_read_file_path)
        .await?;
    tracing::info!("SimuController: Opened GC file successfully.");

    // MMIO device path is taken from config.ipc_config.command_path
    // No file handle is kept open for MMIO by the controller; writes are discrete operations.

    // --- Step 1: Send Start command via MMIO ---
    // As per new ddr_data_init: xdma_write(16, 1, 0x12000);
    tracing::info!(
        "SimuController: Sending Start command (value 1 to addr {:#X}, offset {:#X}) via MMIO to {}.",
        COMMAND_TRIGGER_ADDR_BYTES, COMMAND_TRIGGER_OFFSET, &config.ipc_config.command_path
    );
    xdma_write(
        &config.ipc_config.command_path,
        COMMAND_TRIGGER_OFFSET,
        MMIO_MAP_LEN,
        COMMAND_TRIGGER_ADDR_BYTES,
        1, // Value for Start
    )?;
    // Replaced std::thread::sleep with tokio::time::sleep for non-blocking behavior in async context
    sleep(std_time::Duration::from_millis(100)).await; // From ddr_data_init
    tracing::info!("SimuController: Start command sent via MMIO.");

    let start_time = std_time::Instant::now(); // Record start time for GC calculation in Source mode

    // Helper function to decode GCR item
    // Matches the logic in hw_sim's tests and simulator's encode_gcr
    fn split_gcr(buf_gcr: [u8; 8]) -> (u64, u8) {
        let buf: [u8; 8] = buf_gcr; // Removed mut
        // The original GC value was shifted right by 1 (shifted_gc), and its LSB (gc_lsb) stored in bit 0 of buf_gcr[6].
        // encode_gcr stores shifted_gc into an 8-byte buffer, then modifies buf_gcr[6] by clearing its
        // lowest 2 bits and ORing in gc_lsb and the result_bit.
        // So, gc_upper_part correctly reconstructs shifted_gc.
        let gc_upper_part = { // This is shifted_gc
            let mut temp_buf = buf;
            temp_buf[6] &= 0b1111_1100; // Clear bits 0 and 1 (gc_lsb and result_bit) from buf_gcr[6]
            u64::from_le_bytes(temp_buf) // The rest of the buffer contains shifted_gc
        };
        let gc_lsb = (buf_gcr[6] & 1) as u64; // Extract the original LSB of gc
        // Reconstruct gc: (shifted_gc * 2) + gc_lsb
        let gc = (gc_upper_part << 1) | gc_lsb;

        let result: u8 = (buf_gcr[6] >> 1) & 1; // Extract the result bit
        (gc, result)
    }

    // Mode-specific interaction loop
    let num_batches_to_process = 3; // Example: process 3 batches
    match config.simulator_mode {
        SimulatorMode::Detector => {
            tracing::info!("SimuController: Operating in Detector-compatible mode.");
            for batch_num in 0..num_batches_to_process {
                tracing::info!("SimuController (Detector): Processing batch #{}", batch_num + 1);

                // Read BATCH_SIZE GCRs
                let mut gcr_batch_buffer = vec![0u8; BATCH_SIZE * 16]; // Each GCR is 8 bytes + 8 bytes padding
                tracing::info!("SimuController (Detector): Attempting to read {} GCR bytes.", gcr_batch_buffer.len());
                match gcr_file.read_exact(&mut gcr_batch_buffer).await {
                    Ok(_) => {
                        tracing::info!("SimuController (Detector): Successfully read GCR batch.");
                        let mut received_gc_values = Vec::with_capacity(BATCH_SIZE);
                        for (i, gcr_chunk) in gcr_batch_buffer.chunks_exact(16).enumerate() {
                            // The GCR data is the first 8 bytes of the 16-byte padded chunk.
                            let gcr_item: [u8; 8] = gcr_chunk[0..8].try_into().unwrap();
                            let (gc, result) = split_gcr(gcr_item);
                            received_gc_values.push(gc);
                            if i < 5 { // Log first few GCRs for brevity
                                tracing::debug!("SimuController (Detector): GCR item {}: gc={}, result={}", i, gc, result);
                            }
                        }

                        // Send back received GC values
                        tracing::info!("SimuController (Detector): Sending {} GC values back to hw_sim.", received_gc_values.len());
                        for gc_val in received_gc_values {
                            gc_file.write_all(&gc_val.to_le_bytes()).await?; // Write 8-byte GC
                            gc_file.write_all(&[0u8; 8]).await?; // Write 8-byte padding
                        }
                        gc_file.flush().await?;
                        tracing::info!("SimuController (Detector): GC values sent.");

                        // Read corresponding packed angles. The simulator packs 2 angle indices into 1 byte.
                        let mut angle_batch_buffer = vec![0u8; BATCH_SIZE / 2];
                        tracing::info!("SimuController (Detector): Attempting to read {} angle bytes.", angle_batch_buffer.len());
                        match angle_file.read_exact(&mut angle_batch_buffer).await {
                            Ok(_) => {
                                tracing::info!("SimuController (Detector): Successfully read angle batch ({} bytes).", angle_batch_buffer.len());
                                // Unpack and log the first few angle indices
                                let mut unpacked_indices = Vec::with_capacity(BATCH_SIZE);
                                for &packed_byte in &angle_batch_buffer {
                                    let index1 = packed_byte & 0x0F;
                                    let index2 = (packed_byte >> 4) & 0x0F;
                                    unpacked_indices.push(index1);
                                    unpacked_indices.push(index2);
                                }

                                for i in 0..std::cmp::min(5, angle_batch_buffer.len()) { // Log first 5 angles
                                    tracing::debug!("SimuController (Detector): Angle item {}: {}", i, angle_batch_buffer[i]);
                                }
                                // Process/log angles if needed
                            }
                            Err(e) => {
                                tracing::error!("SimuController (Detector): Failed to read angle batch: {}", e);
                                break; // Exit loop on error
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("SimuController (Detector): Failed to read GCR batch: {}", e);
                        break; // Exit loop on error
                    }
                }
                sleep(Duration::from_millis(100)).await; // Small delay between batches
            }
        }
        SimulatorMode::Source => {
            tracing::info!("SimuController: Operating in Source-compatible mode.");
            // let mut rng = rand::thread_rng(); // Replaced by time-based calculation
            for batch_num in 0..num_batches_to_process {
                tracing::info!("SimuController (Source): Processing batch #{}", batch_num + 1);

                let t_elapsed_secs = start_time.elapsed().as_secs_f64();
                
                let pulse_periods = t_elapsed_secs / config.backend_config.pulse_distance;
                let effective_periods = pulse_periods - config.backend_config.gc_offset as f64;
                
                let calculated_l_float = if effective_periods > 0.0 {
                    effective_periods * config.backend_config.eta
                } else {
                    0.0
                };
                let base_gc_for_batch = calculated_l_float as u64;

                let mut gc_values_to_send = Vec::with_capacity(BATCH_SIZE);
                for i in 0..BATCH_SIZE {
                    gc_values_to_send.push(base_gc_for_batch + i as u64);
                }

                tracing::info!(
                    "SimuController (Source): t={:.3}s, L_base={}, Sending {} GC values starting from L_base.",
                    t_elapsed_secs,
                    base_gc_for_batch,
                    gc_values_to_send.len()
                );
                for gc_val in &gc_values_to_send {
                    gc_file.write_all(&gc_val.to_le_bytes()).await?; // Write 8-byte GC
                    gc_file.write_all(&[0u8; 8]).await?; // Write 8-byte padding
                }
                gc_file.flush().await?;
                tracing::info!("SimuController (Source): Calculated GC values sent.");

                // Read corresponding packed angles. The simulator packs 2 angle indices into 1 byte.
                let mut angle_batch_buffer = vec![0u8; BATCH_SIZE / 2];
                tracing::info!("SimuController (Source): Attempting to read {} angle bytes.", angle_batch_buffer.len());
                match angle_file.read_exact(&mut angle_batch_buffer).await {
                    Ok(_) => {
                        tracing::info!("SimuController (Source): Successfully read angle batch ({} bytes).", angle_batch_buffer.len());
                        // Unpack and log the first few angle indices
                        let mut unpacked_indices = Vec::with_capacity(BATCH_SIZE);
                        for &packed_byte in &angle_batch_buffer {
                            let index1 = packed_byte & 0x0F;
                            let index2 = (packed_byte >> 4) & 0x0F;
                            unpacked_indices.push(index1);
                            unpacked_indices.push(index2);
                        }

                        for i in 0..std::cmp::min(5, angle_batch_buffer.len()) { // Log first 5 angles
                            tracing::debug!("SimuController (Source): Angle item {}: {}", i, angle_batch_buffer[i]);
                        }
                        // Process/log angles if needed
                    }
                    Err(e) => {
                        tracing::error!("SimuController (Source): Failed to read angle batch: {}", e);
                        break; // Exit loop on error
                    }
                }
                sleep(Duration::from_millis(100)).await; // Small delay between batches
            }
        }
    }

    // --- Step 4: Send Stop command via MMIO ---
    tracing::info!(
        "SimuController: Sending Stop command (value 0 to addr {:#X}, offset {:#X}) via MMIO to {}.",
        COMMAND_TRIGGER_ADDR_BYTES, COMMAND_TRIGGER_OFFSET, &config.ipc_config.command_path
    );
    xdma_write(
        &config.ipc_config.command_path,
        COMMAND_TRIGGER_OFFSET,
        MMIO_MAP_LEN,
        COMMAND_TRIGGER_ADDR_BYTES,
        0, // Value for Stop
    )?;
    tracing::info!("SimuController: Stop command sent via MMIO.");

    // Add a small delay to allow the simulator to process the stop command and close its FIFOs.
    // This prevents a deadlock where the controller closes its read ends before the simulator is done.
    sleep(Duration::from_secs(1)).await;

    // --- Step 5: Drain remaining data from readable FIFOs ---
    tracing::info!("SimuController: Attempting to empty readable FIFOs...");
    let mut angle_buffer = vec![0u8; BATCH_SIZE]; // Re-use buffer for draining
    let mut gcr_buffer_drain = vec![0u8; BATCH_SIZE * 8]; // Buffer for draining GCR

    let mut drained_something_in_iteration;
    let mut drain_attempts = 0;
    const MAX_DRAIN_ATTEMPTS: u32 = 5; // Safety break for the drain loop

    loop {
        if drain_attempts >= MAX_DRAIN_ATTEMPTS {
            tracing::warn!(
                "SimuController: Max drain attempts ({}) reached. Stopping drain.",
                MAX_DRAIN_ATTEMPTS
            );
            break;
        }
        drain_attempts += 1;
        drained_something_in_iteration = false;

        // Small delay to allow hw_sim to write if it's still active after stop,
        // and to prevent busy-looping if FIFOs are immediately empty.
        sleep(Duration::from_millis(100)).await;

        // Try draining angle_file
        match tokio::time::timeout(
            Duration::from_millis(50),
            angle_file.read(&mut angle_buffer),
        )
        .await
        {
            Ok(Ok(0)) => {
                tracing::debug!("SimuController: Angle file (drain) - EOF.");
            }
            Ok(Ok(n)) => {
                tracing::info!("SimuController: Drained {} bytes from angle_file.", n);
                drained_something_in_iteration = true;
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tracing::debug!("SimuController: Angle file (drain) - No data (WouldBlock).");
            }
            Ok(Err(e)) => {
                tracing::warn!("SimuController: Error draining angle_file: {}", e);
            }
            Err(_) => {
                tracing::debug!("SimuController: Angle file (drain) - Read attempt timed out.");
            }
        }

        // Try draining gcr_file only if in Detector mode (as it's opened in that mode)
        if config.simulator_mode == SimulatorMode::Detector {
            match tokio::time::timeout(
                Duration::from_millis(50),
                gcr_file.read(&mut gcr_buffer_drain),
            )
            .await
            {
                Ok(Ok(0)) => {
                    tracing::debug!("SimuController: GCR file (drain) - EOF.");
                }
                Ok(Ok(n)) => {
                    tracing::info!("SimuController: Drained {} bytes from gcr_file.", n);
                    drained_something_in_iteration = true;
                }
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    tracing::debug!("SimuController: GCR file (drain) - No data (WouldBlock).");
                }
                Ok(Err(e)) => {
                    tracing::warn!("SimuController: Error draining gcr_file: {}", e);
                }
                Err(_) => {
                    tracing::debug!("SimuController: GCR file (drain) - Read attempt timed out.");
                }
            }
        }

        if !drained_something_in_iteration {
            tracing::info!("SimuController: Draining complete (no data read from FIFOs in last iteration).");
            break;
        }
    }

        tracing::info!("SimuController: Main logic finished. Files will be dropped now.");
        Ok(())
    }
    .await; // Run the inner async block

    if result.is_ok() {
        tracing::info!("SimuController: Files have been dropped. Runtime should exit shortly.");
    } else if let Err(e) = &result {
        tracing::error!("SimuController: Error occurred: {}", e);
    }
    
    // The final tokio::time::sleep (previously here for diagnostics) has been removed.
    
    result
}

/// Synchronously writes a u32 value to a memory-mapped device.
/// `map_offset` is the offset passed to MmapOptions.
/// `value_addr_bytes` is the byte address *within* the mapped region.
fn xdma_write(
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
        // Ensure address + size of u32 is within map_len
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "MMIO address out of bounds for the mapped length",
        ));
    }

    let file = StdOpenOptions::new()
        .read(true)
        .write(true)
        .open(device_path)?;
    unsafe {
        let mut mmap = MmapOptions::new()
            .len(map_len)
            .offset(map_offset)
            .map_mut(&file)?; // map_mut for writing
        let ptr = mmap.as_mut_ptr().add(value_addr_bytes) as *mut u32;
        ptr.write_volatile(value); // Use write_volatile for MMIO
                                   // mmap.flush() might be needed for some memory types, but often not for MMIO registers.
                                   // If issues occur, consider adding mmap.flush() or mmap.flush_async().
    }
    Ok(())
}
