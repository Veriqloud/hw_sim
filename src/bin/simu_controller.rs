use memmap2::MmapOptions;
use serde::Deserialize;
use std::fs::OpenOptions as StdOpenOptions; // For synchronous file operations in xdma_write
use std::thread; // For the sleep in ddr_data_init sequence
use std::time as std_time; // For the sleep in ddr_data_init sequence
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, Duration};

// MMIO Constants
const COMMAND_TRIGGER_OFFSET: u64 = 0x12000; // Base offset for the command trigger register
const COMMAND_TRIGGER_ADDR_BYTES: usize = 16; // Byte address for the command trigger u32
const MMIO_MAP_LEN: usize = 0x1000; // General memory map length, ensure it covers addresses

// Structs to deserialize the relevant parts of config/valid_config_alice.json
#[derive(Deserialize, Debug)]
struct IpcConfig {
    command_path: String, // Renamed from xdma_device_path
    angle_file_path: String,
    click_result_file_path: String,
    gc_file_path: String,
    gcr_file_path: String, // Added for reading GCR data
}

#[derive(Deserialize, Debug)]
struct ControllerConfig {
    ipc_config: IpcConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging
    tracing_subscriber::fmt::init();

    let config_path = "config/valid_config_alice.json";
    tracing::info!(
        "SimuController: Reading configuration from '{}'",
        config_path
    );

    let config_content = tokio::fs::read_to_string(config_path).await?;
    let config: ControllerConfig = serde_json::from_str(&config_content)?;

    tracing::info!("SimuController: Parsed IPC Config: {:?}", config.ipc_config);

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
    // 2. click_result_file (read by controller)
    // 3. gc_file (write by controller)
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

    // 2. Click Result file (read-only for controller)
    tracing::info!(
        "SimuController: Opening click result file: {}",
        config.ipc_config.click_result_file_path
    );
    let mut click_result_file = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.click_result_file_path)
        .await?;
    tracing::info!("SimuController: Opened click result file successfully.");

    // 3. Global Counter file (write-only for controller)
    tracing::info!(
        "SimuController: Opening GC file: {}",
        config.ipc_config.gc_file_path
    );
    let mut gc_file = OpenOptions::new()
        .write(true)
        .open(&config.ipc_config.gc_file_path)
        .await?;
    tracing::info!("SimuController: Opened GC file successfully.");

    // 4. GCR file (read-only for controller)
    tracing::info!(
        "SimuController: Opening GCR file: {}",
        config.ipc_config.gcr_file_path
    );
    let mut gcr_file = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.gcr_file_path)
        .await?;
    tracing::info!("SimuController: Opened GCR file successfully.");

    // MMIO device path is taken from config.ipc_config.xdma_device_path
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
    thread::sleep(std_time::Duration::from_millis(100)); // From ddr_data_init
    tracing::info!("SimuController: Start command sent via MMIO.");

    // --- Step 2: Send GC value ---
    // This should trigger seed_and_start_generation in hw_sim
    let gc_value = 12345u64; // Example Global Counter value
    tracing::info!("SimuController: Sending GC value: {}", gc_value);
    gc_file.write_u64_le(gc_value).await?;
    gc_file.flush().await?; // Ensure the GC value is sent
    tracing::info!("SimuController: GC value sent. Waiting for GCR data from hw_sim...");

    // --- Step 2b: Read GCR data from hw_sim ---
    // hw_sim writes 8 bytes to gcr_file after it processes the Start command and before reading GC.
    let mut gcr_buffer = [0u8; 8];
    match tokio::time::timeout(Duration::from_secs(5), gcr_file.read_exact(&mut gcr_buffer)).await {
        Ok(Ok(_)) => {
            tracing::info!("SimuController: Read GCR data: {:?}", gcr_buffer);
            // Optionally, decode using split_gcr and log the (gc, result)
            // fn split_gcr(buf_gcr: [u8;8]) -> (u64, u8){
            //     let mut buf: [u8; 8] = buf_gcr;
            //     buf[6] = 0;
            //     buf[7] = 0;
            //     let mut gc: u64 = u64::from_le_bytes(buf);
            //     gc = gc*2 + (buf_gcr[6] & 1) as u64;
            //     let result: u8 = (buf_gcr[6] >> 1) & 1;
            //     return (gc, result)
            // }
            // let (decoded_gc, decoded_result) = split_gcr(gcr_buffer);
            // tracing::info!("SimuController: Decoded GCR: gc={}, result={}", decoded_gc, decoded_result);
        }
        Ok(Err(e)) => {
            tracing::error!("SimuController: Failed to read GCR data: {}", e);
            // Decide if this is fatal or if the controller should continue
        }
        Err(_) => {
            tracing::error!("SimuController: Timeout reading GCR data.");
            // Decide if this is fatal
        }
    }

    tracing::info!(
        "SimuController: Waiting a moment for hw_sim to start processing angles/clicks..."
    );
    sleep(Duration::from_millis(500)).await; // Give hw_sim time to react

    // --- Step 3: Read angles and click results ---
    tracing::info!("SimuController: Attempting to read data for ~2 seconds...");
    let mut angle_buffer = vec![0u8; 1024]; // Based on simulator's read_angles output size
    let mut click_buffer = vec![0u8; 1024]; // Assuming similar size for click results

    for i in 0..10 {
        // Loop 10 times, sleeping 200ms each time
        tracing::debug!("SimuController: Read attempt #{}", i + 1);

        // Try reading angles with a timeout for each attempt
        match tokio::time::timeout(
            Duration::from_millis(100),
            angle_file.read(&mut angle_buffer),
        )
        .await
        {
            Ok(Ok(0)) => tracing::info!(
                "SimuController: Angle file - EOF or no data read at attempt {}.",
                i + 1
            ),
            Ok(Ok(n)) => tracing::info!(
                "SimuController: Read {} bytes from angle_file at attempt {}.",
                n,
                i + 1
            ),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => tracing::info!(
                "SimuController: Angle file - No data available (WouldBlock) at attempt {}.",
                i + 1
            ),
            Ok(Err(e)) => tracing::warn!(
                "SimuController: Error reading from angle_file at attempt {}: {}",
                i + 1,
                e
            ),
            Err(_) => tracing::info!(
                "SimuController: Angle file - Read attempt {} timed out.",
                i + 1
            ),
        }

        // Try reading click results with a timeout
        match tokio::time::timeout(
            Duration::from_millis(100),
            click_result_file.read(&mut click_buffer),
        )
        .await
        {
            Ok(Ok(0)) => tracing::info!(
                "SimuController: Click result file - EOF or no data read at attempt {}.",
                i + 1
            ),
            Ok(Ok(n)) => tracing::info!(
                "SimuController: Read {} bytes from click_result_file at attempt {}.",
                n,
                i + 1
            ),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => tracing::info!(
                "SimuController: Click result file - No data available (WouldBlock) at attempt {}.",
                i + 1
            ),
            Ok(Err(e)) => tracing::warn!(
                "SimuController: Error reading from click_result_file at attempt {}: {}",
                i + 1,
                e
            ),
            Err(_) => tracing::info!(
                "SimuController: Click result file - Read attempt {} timed out.",
                i + 1
            ),
        }

        if i < 9 {
            // Avoid sleep after the last iteration
            sleep(Duration::from_millis(200)).await;
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

    // --- Step 5: Drain remaining data ---
    tracing::info!("SimuController: Attempting to empty FIFOs...");
    let mut drained_something_in_iteration;
    let mut drain_attempts = 0;
    const MAX_DRAIN_ATTEMPTS: u32 = 20; // Safety break for the drain loop

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
        sleep(Duration::from_millis(50)).await;

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
                // Potentially break or handle specific errors if they are persistent and critical
            }
            Err(_) => {
                // Timeout
                tracing::debug!("SimuController: Angle file (drain) - Read attempt timed out.");
            }
        }

        // Try draining click_result_file
        match tokio::time::timeout(
            Duration::from_millis(50),
            click_result_file.read(&mut click_buffer),
        )
        .await
        {
            Ok(Ok(0)) => {
                tracing::debug!("SimuController: Click result file (drain) - EOF.");
            }
            Ok(Ok(n)) => {
                tracing::info!(
                    "SimuController: Drained {} bytes from click_result_file.",
                    n
                );
                drained_something_in_iteration = true;
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tracing::debug!(
                    "SimuController: Click result file (drain) - No data (WouldBlock)."
                );
            }
            Ok(Err(e)) => {
                tracing::warn!("SimuController: Error draining click_result_file: {}", e);
            }
            Err(_) => {
                // Timeout
                tracing::debug!(
                    "SimuController: Click result file (drain) - Read attempt timed out."
                );
            }
        }

        if !drained_something_in_iteration {
            tracing::info!("SimuController: Draining complete (no data read from either FIFO in last iteration).");
            break;
        }
    }

    tracing::info!("SimuController: Finished.");
    Ok(())
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
