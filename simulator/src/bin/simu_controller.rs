use clap::{Parser, Subcommand};
use memmap2::MmapOptions;
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

// Constants from the simulator's reader implementation
const BATCH_SIZE: usize = 1024; // Number of records in a batch
const GC_RECORD_SIZE: usize = 16; // Size of a GC record in bytes
const ANGLE_RECORD_SIZE: usize = 1; // Size of an angle record in bytes
const MMIO_MAP_OFFSET: u64 = 0x12000;
const MMIO_MAP_LEN: usize = 0x1000;
const COMMAND_TRIGGER_ADDR_BYTES: usize = 16;

#[derive(Parser, Debug)]
#[command(author, version, about = "Hardware simulator controller", long_about = None)]
struct Cli {
    /// Path to the hw_sim configuration file
    #[arg(long)]
    config_path: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Alice's simulator workflow
    Alice {
        /// Number of batches to send
        num_batches: u64,
    },
    /// Bob's simulator workflow
    Bob {
        /// Number of batches to measure
        num_batches: u64,
    },
}

#[derive(Deserialize, Debug)]
struct Config {
    backend_config: BackendConfig,
    ipc_config: IpcConfig,
}

#[derive(Deserialize, Debug)]
struct BackendConfig {
    angles: Vec<u8>,
}

#[derive(Deserialize, Debug)]
struct IpcConfig {
    command_path: PathBuf,
    angle_file_path: PathBuf,
    gc_read_file_path: PathBuf,
    gcr_file_path: Option<PathBuf>,
}

/// Writes a u32 value to a memory-mapped device to trigger a command.
fn trigger_mmio_command(
    device_path: &PathBuf,
    map_offset: u64,
    map_len: usize,
    value_addr_bytes: usize,
    value: u32,
) -> io::Result<()> {
    if value_addr_bytes % 4 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MMIO address must be u32-aligned",
        ));
    }
    if value_addr_bytes + 4 > map_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MMIO address out of bounds for the mapped length",
        ));
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true) // Ensure file exists
        .open(device_path)?;
    // Ensure the file is large enough for the MMIO region
    let required_len = map_offset + map_len as u64;
    if file.metadata()?.len() < required_len {
        file.set_len(required_len)?;
    }
    let mut mmap = unsafe { MmapOptions::new().offset(map_offset).len(map_len).map_mut(&file)? };

    let ptr = unsafe { mmap.as_mut_ptr().add(value_addr_bytes) as *mut u32 };
    unsafe { ptr.write_volatile(value) };
    mmap.flush()?;
    Ok(())
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    let config_str =
        fs::read_to_string(&cli.config_path).expect("Could not read config file");
    let config: Config = serde_json::from_str(&config_str).expect("Could not parse config file");

    match cli.command {
        Commands::Alice { num_batches } => run_alice(num_batches, config),
        Commands::Bob { num_batches } => run_bob(num_batches, config),
    }
}

fn run_alice(num_batches: u64, config: Config) -> io::Result<()> {
    println!(
        "Running Alice's workflow for {} batches of size {}...",
        num_batches, BATCH_SIZE
    );
    println!("This controller acts as the 'GC client' for Alice's simulator.");
    let start_time = Instant::now();

    // Send Start command
    println!("Triggering START command via MMIO...");
    trigger_mmio_command(
        &config.ipc_config.command_path,
        MMIO_MAP_OFFSET,
        MMIO_MAP_LEN,
        COMMAND_TRIGGER_ADDR_BYTES,
        1,
    )?;

    let mut gc_writer = OpenOptions::new()
        .write(true)
        .open(&config.ipc_config.gc_read_file_path)?;
    let mut angle_reader = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.angle_file_path)?;

    println!("Alice: Starting batch processing loop.");

    // The simulator's writer packs two 1-byte angles into a single byte.
    // Therefore, we expect to read half the number of bytes as there are angles.
    let mut angle_buffer = vec![0; (BATCH_SIZE * ANGLE_RECORD_SIZE) / 2];

    for i in 0..num_batches {
        // 1. Write a batch of dummy GC values for the simulator to read.
        // The simulator expects 1024 records of 16 bytes each.
        let gc_batch: Vec<u8> = (0..BATCH_SIZE)
            .flat_map(|j| {
                let gc_val: u64 = (i * BATCH_SIZE as u64) + j as u64;
                let mut record = [0u8; GC_RECORD_SIZE];
                record[0..8].copy_from_slice(&gc_val.to_le_bytes());
                record.into_iter()
            })
            .collect();
        gc_writer.write_all(&gc_batch)?;
        gc_writer.flush()?;

        // 2. Read the batch of angles the simulator wrote in response.
        angle_reader.read_exact(&mut angle_buffer)?;
    }

    let duration = start_time.elapsed();
    println!("Alice's workflow finished in {:?}", duration);

    if duration > Duration::from_secs(0) {
        let batches_per_sec = num_batches as f64 / duration.as_secs_f64();
        let angles_per_sec = (num_batches * BATCH_SIZE as u64) as f64 / duration.as_secs_f64();
        println!("Total batches processed: {}", num_batches);
        println!("Effective batch rate: {:.2} batches/sec", batches_per_sec);
        println!("Effective angle rate: {:.2} angles/sec", angles_per_sec);
    }

    println!("Triggering STOP command via MMIO...");
    trigger_mmio_command(
        &config.ipc_config.command_path,
        MMIO_MAP_OFFSET,
        MMIO_MAP_LEN,
        COMMAND_TRIGGER_ADDR_BYTES,
        0,
    )?;


    Ok(())
}

fn run_bob(num_batches: u64, config: Config) -> io::Result<()> {
    println!(
        "Running Bob's workflow for {} batches of size {}...",
        num_batches, BATCH_SIZE
    );
    println!("This controller acts as the client for Bob's simulator.");
    let start_time = Instant::now();

    // 1. Send the Start command FIRST.
    // This unblocks the simulator from its `await_next_command` loop, causing it to
    // open its ends of the FIFO pipes.
    println!("Triggering START command via MMIO...");
    trigger_mmio_command(
        &config.ipc_config.command_path,
        MMIO_MAP_OFFSET,
        MMIO_MAP_LEN,
        COMMAND_TRIGGER_ADDR_BYTES,
        1,
    )?;

    // 2. Open the file handles. Since the simulator was started, these `open`
    // calls on the FIFOs should not block for long.
    let gcr_path = config
        .ipc_config
        .gcr_file_path
        .expect("gcr_file_path not specified for Bob in config.");
    let mut gcr_reader = OpenOptions::new().read(true).open(gcr_path)?;
    let mut gc_writer = OpenOptions::new()
        .write(true)
        .open(&config.ipc_config.gc_read_file_path)?;
    let mut angle_reader = OpenOptions::new()
        .read(true)
        .open(&config.ipc_config.angle_file_path)?;

    println!("Bob: Starting batch processing loop.");

    // The simulator's writer receives 2048 items of type [u8; 8] and flattens them.
    // So the total size written to the GCR pipe is 2048 * 8 = 16384 bytes.
    // BATCH_SIZE is 1024, so we need a buffer of BATCH_SIZE * 2 * 8.
    let mut gcr_buffer = vec![0; BATCH_SIZE * 2 * 8];
    // The simulator's writer packs two 1-byte angles into a single byte.
    let mut angle_buffer = vec![0; (BATCH_SIZE * ANGLE_RECORD_SIZE) / 2];

    for i in 0..num_batches {
        // 1. Read the GCR batch from the simulator.
        gcr_reader.read_exact(&mut gcr_buffer)?;

        // 2. Write a batch of dummy "echoed" GC values for the simulator to read.
        // The simulator expects 1024 records of 16 bytes each.
        let gc_batch: Vec<u8> = (0..BATCH_SIZE)
            .flat_map(|j| {
                let gc_val: u64 = (i * BATCH_SIZE as u64) + j as u64;
                let mut record = [0u8; GC_RECORD_SIZE];
                record[0..8].copy_from_slice(&gc_val.to_le_bytes());
                record.into_iter()
            })
            .collect();
        gc_writer.write_all(&gc_batch)?;
        gc_writer.flush()?;

        // 3. Read the batch of angles the simulator wrote.
        angle_reader.read_exact(&mut angle_buffer)?;
    }

    let duration = start_time.elapsed();
    println!("Bob's workflow finished in {:?}", duration);

    if duration > Duration::from_secs(0) {
        let batches_per_sec = num_batches as f64 / duration.as_secs_f64();
        let angles_per_sec = (num_batches * BATCH_SIZE as u64) as f64 / duration.as_secs_f64();
        println!("Total batches processed: {}", num_batches);
        println!("Effective batch rate: {:.2} batches/sec", batches_per_sec);
        println!("Effective angle rate: {:.2} angles/sec", angles_per_sec);
    }

    println!("Triggering STOP command via MMIO...");
    trigger_mmio_command(
        &config.ipc_config.command_path,
        MMIO_MAP_OFFSET,
        MMIO_MAP_LEN,
        COMMAND_TRIGGER_ADDR_BYTES,
        0,
    )?;


    Ok(())
}
