use clap::Parser;
use memmap2::MmapOptions;
use serde::Deserialize;
use std::f64::consts::PI;
use std::fs::OpenOptions as StdOpenOptions; // For synchronous file operations in xdma_write
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::thread::sleep;
use std::time as std_time; // For std_time::Instant and std_time::Duration
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

// Command-line arguments structure
/// This controller is updated to run against BOTH Alice and Bob simulators simultaneously
/// to verify their outputs are correlated as expected.
/// It requires two separate configuration files, one for each simulator.
#[derive(Parser, Debug)]
#[clap(author, version, about = "Simulation Controller CLI", long_about = None)]
struct CliArgs {
    /// Path to the JSON configuration file for Alice's (Source) simulator.
    #[clap(long)]
    alice_config_path: String,
    /// Path to the JSON configuration file for Bob's (Detector) simulator.
    #[clap(long)]
    bob_config_path: String,
}

// MMIO Constants
const COMMAND_TRIGGER_OFFSET: u64 = 0x12000; // Base offset for the command trigger register
const COMMAND_TRIGGER_ADDR_BYTES: usize = 16; // Byte address for the command trigger u32
const MMIO_MAP_LEN: usize = 0x1000; // General memory map length, ensure it covers addresses

// Structs to deserialize the relevant parts of config/valid_config_alice.json
#[derive(Deserialize, Debug, Clone)]
struct IpcConfig {
    command_path: String,
    angle_file_path: String,
    gc_read_file_path: String, // Renamed from gc_file_path to match hw_sim's ipc_config
    gcr_file_path: Option<String>, // Make optional, as it's only present for Bob
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

#[derive(Deserialize, Debug, Clone)]
struct BackendConfig {
    pulse_distance: f64,
    eta: f64,
    #[serde(rename = "qberr")]
    qb_err: f64, // Renamed to match the JSON field "qberr"
    angles: Vec<u8>,
    #[serde(default = "default_gc_offset")]
    gc_offset: u64,
}

#[derive(Deserialize, Debug, Clone)]
struct ControllerConfig {
    ipc_config: IpcConfig,
    simulator_mode: SimulatorMode,
    #[serde(default)] // For backward compatibility with old configs
    backend_config: Option<BackendConfig>,
}

const BATCH_SIZE: usize = 1024; // Matching hw_sim's BATCH_SIZE

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging
    tracing_subscriber::fmt::init();

    let result = {
        let cli_args = CliArgs::parse();

        // --- Configuration Loading for Alice and Bob ---
        tracing::info!(
            "SimuController: Reading Alice's config from '{}'",
            &cli_args.alice_config_path
        );
        let alice_config_content = std::fs::read_to_string(&cli_args.alice_config_path)?;
        let alice_config: ControllerConfig = serde_json::from_str(&alice_config_content)?;

        tracing::info!(
            "SimuController: Reading Bob's config from '{}'",
            &cli_args.bob_config_path
        );
        let bob_config_content = std::fs::read_to_string(&cli_args.bob_config_path)?;
        let bob_config: ControllerConfig = serde_json::from_str(&bob_config_content)?;

        tracing::info!(
            "SimuController: Parsed Alice Config: IPC={:?}, Mode={:?}",
            alice_config.ipc_config,
            alice_config.simulator_mode
        );
        tracing::info!(
            "SimuController: Parsed Bob Config: IPC={:?}, Mode={:?}",
            bob_config.ipc_config,
            bob_config.simulator_mode
        );

        // We need backend_config from one of the simulators (they should be identical) for correlation check
        let backend_config = bob_config.backend_config.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Bob's config must contain 'backend_config' for correlation check",
            )
        })?;

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
        // NOTE: For simplicity, we will open files inside each controller task.
        // This is less efficient but avoids complex file handle sharing.

        // --- Step 1: Send Start command via MMIO ---
        tracing::info!("SimuController: Sending Start command to Alice...");
        xdma_write(
            &alice_config.ipc_config.command_path,
            COMMAND_TRIGGER_OFFSET,
            MMIO_MAP_LEN,
            COMMAND_TRIGGER_ADDR_BYTES,
            1, // Value for Start
        )?;
        tracing::info!("SimuController: Sending Start command to Bob...");
        xdma_write(
            &bob_config.ipc_config.command_path,
            COMMAND_TRIGGER_OFFSET,
            MMIO_MAP_LEN,
            COMMAND_TRIGGER_ADDR_BYTES,
            1, // Value for Start
        )?;
        sleep(std_time::Duration::from_millis(100));
        tracing::info!("SimuController: Start commands sent via MMIO.");

        // --- Step 2: Run concurrent controller workflows ---
        let num_batches_to_process: usize = 3;
        let alice_config_arc = Arc::new(alice_config);
        let bob_config_arc = Arc::new(bob_config.clone());

        let alice_task = std::thread::spawn({
            let config = Arc::clone(&alice_config_arc);
            move || {
                // This block contains the logic for the Source (Alice)
                tracing::info!("Alice Controller: Opening files...");
                let mut angle_file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&config.ipc_config.angle_file_path)?;
                let mut gc_file = OpenOptions::new()
                    .write(true)
                    .read(true)
                    .open(&config.ipc_config.gc_read_file_path)?;
                tracing::info!("Alice Controller: Files opened.");

                let start_time = std_time::Instant::now();
                let mut all_alice_angles = Vec::new();
                let backend_config = config.backend_config.as_ref().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Alice's config must contain 'backend_config'",
                    )
                })?;

                for batch_num in 0..num_batches_to_process {
                    tracing::info!(
                        "Alice Controller (Source): Processing batch #{}",
                        batch_num + 1
                    );

                    let t_elapsed_secs = start_time.elapsed().as_secs_f64();
                    let pulse_periods = t_elapsed_secs / backend_config.pulse_distance;
                    let effective_periods = pulse_periods - backend_config.gc_offset as f64;
                    let calculated_l_float = if effective_periods > 0.0 {
                        effective_periods * backend_config.eta
                    } else {
                        0.0
                    };
                    let base_gc_for_batch = calculated_l_float as u64;

                    for i in 0..BATCH_SIZE {
                        let gc_val = base_gc_for_batch + i as u64;
                        gc_file.write_all(&gc_val.to_le_bytes())?;
                        gc_file.write_all(&[0u8; 8])?;
                    }
                    gc_file.flush()?;

                    let mut angle_batch_buffer = vec![0u8; BATCH_SIZE / 2];
                    angle_file.read_exact(&mut angle_batch_buffer)?;

                    for &packed_byte in &angle_batch_buffer {
                        let index1 = packed_byte & 0b0000_0011;
                        let index2 = (packed_byte >> 4) & 0b0000_0011; // Correctly extracts bits 4 and 5
                        all_alice_angles.push(index1);
                        all_alice_angles.push(index2);
                    }
                    sleep(Duration::from_millis(100));
                }
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(all_alice_angles)
            }
        });

        let bob_task = std::thread::spawn({
            let config = Arc::clone(&bob_config_arc);
            move || {
                // This block contains the logic for the Detector (Bob)
                tracing::info!("Bob Controller: Opening files...");
                let mut angle_file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&config.ipc_config.angle_file_path)?;
                let mut gcr_file = OpenOptions::new().read(true).write(true).open(
                    config
                        .ipc_config
                        .gcr_file_path
                        .as_ref()
                        .ok_or("gcr_file_path missing for Bob")?,
                )?;
                let mut gc_file = OpenOptions::new()
                    .write(true)
                    .read(true)
                    .open(&config.ipc_config.gc_read_file_path)?;
                tracing::info!("Bob Controller: Files opened.");

                let mut all_bob_angles = Vec::new();
                let mut all_click_results = Vec::new();

                for batch_num in 0..num_batches_to_process {
                    tracing::info!(
                        "Bob Controller (Detector): Processing batch #{}",
                        batch_num + 1
                    );

                    let mut gcr_batch_buffer = vec![0u8; BATCH_SIZE * 16];
                    gcr_file.read_exact(&mut gcr_batch_buffer)?;

                    let mut received_gc_values = Vec::with_capacity(BATCH_SIZE);
                    for gcr_chunk in gcr_batch_buffer.chunks_exact(16) {
                        let gcr_item: [u8; 8] = gcr_chunk[0..8].try_into().unwrap();
                        let (gc, result) = split_gcr(gcr_item);
                        received_gc_values.push(gc);
                        all_click_results.push(result);
                    }

                    for gc_val in received_gc_values {
                        gc_file.write_all(&gc_val.to_le_bytes())?;
                        gc_file.write_all(&[0u8; 8])?;
                    }
                    gc_file.flush()?;

                    let mut angle_batch_buffer = vec![0u8; BATCH_SIZE / 2];
                    angle_file.read_exact(&mut angle_batch_buffer)?;

                    for &packed_byte in &angle_batch_buffer {
                        let index1 = packed_byte & 0b0000_0011;
                        let index2 = (packed_byte >> 4) & 0b0000_0011; // Correctly extracts bits 4 and 5
                        all_bob_angles.push(index1);
                        all_bob_angles.push(index2);
                    }
                    sleep(Duration::from_millis(100));
                }

                Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
                    all_bob_angles,
                    all_click_results,
                ))
            }
        });

        let bob_results = bob_task.join().expect("Bob's controller thread panicked");
        let alice_results = alice_task
            .join()
            .expect("Alice's controller thread panicked");

        // Extract results from the spawned tasks
        let (bob_angles, click_results) = bob_results.unwrap();

        let alice_angles = alice_results.unwrap();

        // --- Step 3: Check Correlation ---
        tracing::info!("SimuController: Performing correlation check...");
        check_correlation(&alice_angles, &bob_angles, &click_results, &backend_config);
        tracing::info!("SimuController: Correlation check finished.");

        // --- Step 4: Send Stop command via MMIO ---
        tracing::info!("SimuController: Sending Stop command to Alice...");
        xdma_write(
            &alice_config_arc.ipc_config.command_path,
            COMMAND_TRIGGER_OFFSET,
            MMIO_MAP_LEN,
            COMMAND_TRIGGER_ADDR_BYTES,
            0, // Value for Stop
        )?;
        tracing::info!("SimuController: Sending Stop command to Bob...");
        xdma_write(
            &bob_config_arc.ipc_config.command_path,
            COMMAND_TRIGGER_OFFSET,
            MMIO_MAP_LEN,
            COMMAND_TRIGGER_ADDR_BYTES,
            0, // Value for Stop
        )?;
        tracing::info!("SimuController: Stop commands sent via MMIO.");

        // Helper function to decode GCR item
        // Matches the logic in hw_sim's tests and simulator's encode_gcr.
        fn split_gcr(buf_gcr: [u8; 8]) -> (u64, u8) {
            let buf: [u8; 8] = buf_gcr; // Removed mut
                                        // The original GC value was shifted right by 1 (shifted_gc), and its LSB (gc_lsb) stored in bit 0 of buf_gcr[6].
                                        // encode_gcr stores shifted_gc into an 8-byte buffer, then modifies buf_gcr[6] by clearing its
                                        // lowest 2 bits and ORing in gc_lsb and the result_bit.
                                        // So, gc_upper_part correctly reconstructs shifted_gc.
            let gc_upper_part = {
                // This is shifted_gc
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

        // Add a small delay to allow the simulator to process the stop command and close its FIFOs.
        // This prevents a deadlock where the controller closes its read ends before the simulator is done.
        sleep(Duration::from_secs(1));

        tracing::info!("SimuController: Main logic finished. Files will be dropped now.");
        Ok(())
    }; // Run the inner async block

    if result.is_ok() {
        tracing::info!("SimuController: Files have been dropped. Runtime should exit shortly.");
    } else if let Err(e) = &result {
        tracing::error!("SimuController: Error occurred: {}", e);
    }

    // The final tokio::time::sleep (previously here for diagnostics) has been removed.

    result
}

/// Checks the correlation between Alice's and Bob's choices and the final click result.
fn check_correlation(
    alice_angles_indices: &[u8],
    bob_angles_indices: &[u8],
    click_results: &[u8],
    backend_config: &BackendConfig,
) {
    let num_events = alice_angles_indices.len();
    if bob_angles_indices.len() != num_events || click_results.len() != num_events {
        tracing::error!(
            "Correlation Check: Mismatched data lengths! Alice angles: {}, Bob angles: {}, Clicks: {}",
            num_events, bob_angles_indices.len(), click_results.len()
        );
        return;
    }

    // This HashMap will store statistics for each pair of angle choices.
    // Key: (alice_angle_value, bob_angle_value)
    // Value: (count_of_result_0, count_of_result_1)
    let mut correlation_stats: HashMap<(u8, u8), (u32, u32)> = HashMap::new();

    let test_angles = &backend_config.angles;

    for i in 0..num_events {
        let alice_idx = alice_angles_indices[i] as usize;
        let bob_idx = bob_angles_indices[i] as usize;
        let result = click_results[i];

        if alice_idx >= test_angles.len() || bob_idx >= test_angles.len() {
            tracing::warn!(
                "Invalid angle index found (Alice: {}, Bob: {}), skipping event {}.",
                alice_idx,
                bob_idx,
                i
            );
            continue;
        }

        let alice_angle = test_angles[alice_idx];
        let bob_angle = test_angles[bob_idx];

        let stats = correlation_stats
            .entry((alice_angle, bob_angle))
            .or_insert((0, 0));
        if result == 0 {
            stats.0 += 1;
        } else {
            stats.1 += 1;
        }
    }

    tracing::info!("--- Correlation Verification Results ---");
    let mut sorted_keys: Vec<_> = correlation_stats.keys().collect();
    sorted_keys.sort();

    for key in sorted_keys {
        let (angle_a, angle_b) = *key;
        let (zeros, ones) = correlation_stats[key];
        let total = zeros + ones;
        if total == 0 {
            continue;
        }

        let measured_prob_of_1 = ones as f64 / total as f64;

        // This logic is copied from the `random.rs` test to calculate the theoretical probability.
        // The protocol adds a +32 offset to simulate starting from |+> state.
        let total_angle_offset = (angle_a as u32 + angle_b as u32 + 32) as u8 & 127;
        let angle_rad = (total_angle_offset as f64 / 128.0) * PI;
        let ideal_prob_of_1 = angle_rad.sin().powi(2);

        // The final probability of a '1' is P(final=1) = P(ideal=1)*(1-qber) + P(ideal=0)*qber.
        let qber = backend_config.qb_err;
        let expected_prob_of_1 = ideal_prob_of_1 * (1.0 - qber) + (1.0 - ideal_prob_of_1) * qber;

        let difference = (measured_prob_of_1 - expected_prob_of_1).abs();

        tracing::info!(
            "Angles(A:{:2}, B:{:2}) | Total: {:5} | P(1) Measured: {:.4}, Expected: {:.4} | Diff: {:.4}",
            angle_a, angle_b, total, measured_prob_of_1, expected_prob_of_1, difference
        );

        // Assert with a tolerance. For random processes, we expect some deviation.
        // A 5% tolerance is reasonable for a few thousand events.
        if difference > 0.05 {
            tracing::warn!(
                "  -> Large deviation detected for angle pair (A:{}, B:{}).",
                angle_a,
                angle_b
            );
        }
    }
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
