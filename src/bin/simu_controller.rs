use serde::Deserialize;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, Duration};

// Constants for commands understood by hw_sim's IPCReader
const CMD_START: u8 = 0x27;
const CMD_STOP: u8 = 0x26;

// Structs to deserialize the relevant parts of config/valid_config_alice.json
#[derive(Deserialize, Debug)]
struct IpcConfig {
    command_file_path: String,
    angle_file_path: String,
    click_result_file_path: String,
    gc_file_path: String,
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
    tracing::info!("SimuController: Reading configuration from '{}'", config_path);

    let config_content = tokio::fs::read_to_string(config_path).await?;
    let config: ControllerConfig = serde_json::from_str(&config_content)?;

    tracing::info!("SimuController: Parsed IPC Config: {:?}", config.ipc_config);

    // Open FIFO files in an order complementary to hw_sim's combined opening sequence.
    // hw_sim effective order:
    // 1. angle_file (write)
    // 2. click_result_file (write)
    // 3. gc_file (read)
    // 4. cmd_file (read)
    //
    // simu_controller complementary order:
    // 1. angle_file (read)
    // 2. click_result_file (read)
    // 3. gc_file (write)
    // 4. cmd_file (write)

    // 1. Angle file (read-only for controller)
    tracing::info!("SimuController: Opening angle file: {}", config.ipc_config.angle_file_path);
    let mut angle_file = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.angle_file_path)
        .await?;
    tracing::info!("SimuController: Opened angle file successfully.");

    // 2. Click Result file (read-only for controller)
    tracing::info!("SimuController: Opening click result file: {}", config.ipc_config.click_result_file_path);
    let mut click_result_file = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.click_result_file_path)
        .await?;
    tracing::info!("SimuController: Opened click result file successfully.");

    // 3. Global Counter file (write-only for controller)
    tracing::info!("SimuController: Opening GC file: {}", config.ipc_config.gc_file_path);
    let mut gc_file = OpenOptions::new()
        .write(true)
        .open(&config.ipc_config.gc_file_path)
        .await?;
    tracing::info!("SimuController: Opened GC file successfully.");

    // 4. Command file (write-only for controller)
    tracing::info!("SimuController: Opening command file: {}", config.ipc_config.command_file_path);
    let mut cmd_file = OpenOptions::new()
        .write(true)
        .open(&config.ipc_config.command_file_path)
        .await?;
    tracing::info!("SimuController: Opened command file successfully.");

    // --- Step 1: Send Start command ---
    tracing::info!("SimuController: Sending Start command (0x{:02X})", CMD_START);
    cmd_file.write_u8(CMD_START).await?;
    cmd_file.flush().await?; // Ensure the command is sent through the FIFO

    // --- Step 2: Send GC value ---
    // This should trigger seed_and_start_generation in hw_sim
    let gc_value = 12345u64; // Example Global Counter value
    tracing::info!("SimuController: Sending GC value: {}", gc_value);
    gc_file.write_u64_le(gc_value).await?;
    gc_file.flush().await?; // Ensure the GC value is sent

    tracing::info!("SimuController: Waiting a moment for hw_sim to start processing...");
    sleep(Duration::from_millis(500)).await; // Give hw_sim time to react

    // --- Step 3: Read angles and click results ---
    tracing::info!("SimuController: Attempting to read data for ~2 seconds...");
    let mut angle_buffer = vec![0u8; 1024]; // Based on simulator's read_angles output size
    let mut click_buffer = vec![0u8; 1024]; // Assuming similar size for click results

    for i in 0..10 { // Loop 10 times, sleeping 200ms each time
        tracing::debug!("SimuController: Read attempt #{}", i + 1);

        // Try reading angles with a timeout for each attempt
        match tokio::time::timeout(Duration::from_millis(100), angle_file.read(&mut angle_buffer)).await {
            Ok(Ok(0)) => tracing::info!("SimuController: Angle file - EOF or no data read at attempt {}.", i + 1),
            Ok(Ok(n)) => tracing::info!("SimuController: Read {} bytes from angle_file at attempt {}.", n, i + 1),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => tracing::info!("SimuController: Angle file - No data available (WouldBlock) at attempt {}.", i + 1),
            Ok(Err(e)) => tracing::warn!("SimuController: Error reading from angle_file at attempt {}: {}", i + 1, e),
            Err(_) => tracing::info!("SimuController: Angle file - Read attempt {} timed out.", i + 1),
        }

        // Try reading click results with a timeout
        match tokio::time::timeout(Duration::from_millis(100), click_result_file.read(&mut click_buffer)).await {
            Ok(Ok(0)) => tracing::info!("SimuController: Click result file - EOF or no data read at attempt {}.", i + 1),
            Ok(Ok(n)) => tracing::info!("SimuController: Read {} bytes from click_result_file at attempt {}.", n, i + 1),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => tracing::info!("SimuController: Click result file - No data available (WouldBlock) at attempt {}.", i + 1),
            Ok(Err(e)) => tracing::warn!("SimuController: Error reading from click_result_file at attempt {}: {}", i + 1, e),
            Err(_) => tracing::info!("SimuController: Click result file - Read attempt {} timed out.", i + 1),
        }
        
        if i < 9 { // Avoid sleep after the last iteration
            sleep(Duration::from_millis(200)).await;
        }
    }

    // --- Step 4: Send Stop command ---
    tracing::info!("SimuController: Sending Stop command (0x{:02X})", CMD_STOP);
    cmd_file.write_u8(CMD_STOP).await?;
    cmd_file.flush().await?;

    // --- Step 5: Drain remaining data ---
    tracing::info!("SimuController: Attempting to drain remaining data for ~0.5 seconds...");
    // Set a shorter timeout for draining, as hw_sim should be stopping.
    for i in 0..5 { // Loop 5 times, sleeping 100ms each time
        sleep(Duration::from_millis(50)).await; // Short sleep before attempting read

        match tokio::time::timeout(Duration::from_millis(50), angle_file.read(&mut angle_buffer)).await {
            Ok(Ok(0)) => { tracing::info!("SimuController: Angle file (drain) - EOF at attempt {}.", i + 1); break; }
            Ok(Ok(n)) => tracing::info!("SimuController: Drained {} bytes from angle_file at attempt {}.", n, i + 1),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => tracing::info!("SimuController: Angle file (drain) - No data available (WouldBlock) at attempt {}.", i + 1),
            Ok(Err(e)) => tracing::warn!("SimuController: Error draining angle_file at attempt {}: {}", i + 1, e),
            Err(_) => { tracing::info!("SimuController: Angle file (drain) - Read attempt {} timed out.", i + 1); break; }
        }

        match tokio::time::timeout(Duration::from_millis(50), click_result_file.read(&mut click_buffer)).await {
            Ok(Ok(0)) => { tracing::info!("SimuController: Click result file (drain) - EOF at attempt {}.", i + 1); break; }
            Ok(Ok(n)) => tracing::info!("SimuController: Drained {} bytes from click_result_file at attempt {}.", n, i + 1),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => tracing::info!("SimuController: Click result file (drain) - No data available (WouldBlock) at attempt {}.", i + 1),
            Ok(Err(e)) => tracing::warn!("SimuController: Error draining click_result_file at attempt {}: {}", i + 1, e),
            Err(_) => { tracing::info!("SimuController: Click result file (drain) - Read attempt {} timed out.", i + 1); break; }
        }
    }

    tracing::info!("SimuController: Finished.");
    Ok(())
}
