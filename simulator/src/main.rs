pub mod backend;
pub mod cli_args;
pub mod errors;
pub mod ipc;

use clap::Parser;
use configs::{
    ipc::{AliceIpcConfig, BobIpcConfig},
    Configuration,
};
use ipc::writer::actor::IPCWriterActorHandle;
use nix::sys::signal::{SigSet, Signal};
use sim_lib::{hardware::modes::SimulatorMode, simulation::builder::SimulatorBuilder};
use snafu::ResultExt;
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    sync::OnceLock,
    thread::{self, sleep},
    time::Duration,
};
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

pub static CONFIG: OnceLock<Configuration> = OnceLock::new();

fn main() {
    if let Err(e) = app_main() {
        eprintln!("Application exited with error: {}", e);
        std::process::exit(1);
    }
}

// Core application logic
fn app_main() -> Result<(), crate::errors::Error> {
    let span = trace_span!("app_main");
    let _guard = span.enter();

    // Block SIGUSR1 and SIGUSR2 signals early so all threads inherit the mask.
    let mut sigset = SigSet::empty();
    sigset.add(Signal::SIGUSR1);
    sigset.add(Signal::SIGUSR2);
    sigset.thread_block().expect("Failed to block signals");

    let args = cli_args::CliArgs::parse();

    let configuration: Configuration = if let Some(path) = args.conf.config_path {
        Configuration::new(path).context(errors::ConfigLoadSnafu)?
    } else {
        Configuration::default()
    };

    CONFIG
        .set(configuration)
        .expect("failed to set the config global var\n");

    tracing::info!(
        "Running with configuration: {}",
        serde_json::to_string_pretty(&CONFIG.get().unwrap())
            .unwrap_or_else(|e| format!("Failed to serialize config for logging: {}", e))
    );

    // Initialize logger
    let log_level_filter = TryInto::<tracing_subscriber::filter::LevelFilter>::try_into(
        CONFIG.get().unwrap().log_level.to_owned(),
    )
    .context(errors::LoggerInitializationSnafu)?;

    let log_id = Uuid::new_v4();
    let logfile_name = format!("simu_logs_{log_id}.log");
    let logfile_path_str = format!("{}/{}", args.logs_location, &logfile_name);

    let logfile_appender = tracing_appender::rolling::daily(&args.logs_location, &logfile_name);
    let stdout_level = log_level_filter
        .into_level()
        .unwrap_or(tracing::Level::INFO);
    let stdout_writer = std::io::stdout.with_max_level(stdout_level);

    tracing_subscriber::fmt()
        .with_max_level(log_level_filter)
        .with_writer(stdout_writer.and(logfile_appender))
        .init();

    tracing::info!("log_level: {:?}", stdout_level);
    tracing::info!("log file path: {}", logfile_path_str);

    // Setup IPC FIFOs using the method on ipc_config
    CONFIG
        .get()
        .unwrap()
        .ipc_config
        .setup_ipc_fifos()
        .context(errors::IpcConfigSnafu)?;

    tracing::info!(
        "Simulator with configuration : {:?}",
        CONFIG.get().unwrap().backend_config
    );
    tracing::info!(
        "IPC with configuration : {:?}",
        CONFIG.get().unwrap().ipc_config
    );

    let simulator_mode = match CONFIG.get().unwrap().ipc_config {
        configs::ipc::Configuration::Alice(_) => SimulatorMode::Source,
        configs::ipc::Configuration::Bob(_) => SimulatorMode::Detector,
    };

    let sim = SimulatorBuilder::from_config(&CONFIG.get().unwrap().backend_config, simulator_mode);
    let simu_handle = backend::actor::ActorHandle::new(sim);

    // Spawn the signal handling thread.
    let signal_handle = simu_handle.clone();
    thread::spawn(move || {
        let mut sigset = SigSet::empty();
        sigset.add(Signal::SIGUSR1);
        sigset.add(Signal::SIGUSR2);
        loop {
            match sigset.wait() {
                Ok(Signal::SIGUSR1) => {
                    tracing::warn!("Signal SIGUSR1 received: Starting QKD Attack (QBER -> 50%)");
                    if let Err(e) = signal_handle.start_attack() {
                        tracing::error!("Failed to start attack via signal: {:?}", e);
                    }
                }
                Ok(Signal::SIGUSR2) => {
                    tracing::info!("Signal SIGUSR2 received: Stopping QKD Attack (Returning to configured QBER)");
                    if let Err(e) = signal_handle.stop_attack() {
                        tracing::error!("Failed to stop attack via signal: {:?}", e);
                    }
                }
                _ => {}
            }
        }
    });

    // The logic now diverges based on the IPC configuration type
    match &CONFIG.get().unwrap().ipc_config {
        configs::ipc::Configuration::Alice(alice_config) => {
            tracing::info!("Attempting to trigger initial PPS for Alice...");
            if let Err(e) = trigger_pps(&alice_config.command_path) {
                tracing::error!(
                    "Failed to trigger initial PPS for Alice: {}. Continuing...",
                    e
                );
            } else {
                tracing::info!("Initial PPS for Alice triggered successfully.");
            }
            run_alice_workflow(alice_config, simu_handle, simulator_mode);
            tracing::error!("Alice's workflow function returned unexpectedly.");
        }
        configs::ipc::Configuration::Bob(bob_config) => {
            tracing::info!("Attempting to trigger initial PPS for Bob...");
            if let Err(e) = trigger_pps(&bob_config.command_path) {
                tracing::error!(
                    "Failed to trigger initial PPS for Bob: {}. Continuing...",
                    e
                );
            } else {
                tracing::info!("Initial PPS for Bob triggered successfully.");
            }
            run_bob_workflow(bob_config, simu_handle, simulator_mode);
            tracing::error!("Bob's workflow function returned unexpectedly.");
        }
    }

    Ok(())
}

/// Writes a '1' to a specific offset in the command file to trigger a PPS signal.
fn trigger_pps(command_path: &str) -> Result<(), std::io::Error> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(command_path)?;

    // As per the sample function, write to offset 0x1000 + 48.
    let absolute_offset = 0x1000u64 + 48u64;

    // Seek to the position and write the value.
    file.seek(SeekFrom::Start(absolute_offset))?;
    file.write_all(&1u32.to_le_bytes())?;
    file.flush()?;

    tracing::info!(
        "Successfully wrote 1u32 to offset 0x{:X} in file {} for PPS trigger.",
        absolute_offset,
        command_path
    );
    Ok(())
}

// Alice's (Source) workflow: waits for a controller connection in a loop.
fn run_alice_workflow(
    config: &AliceIpcConfig,
    simu_handle: backend::actor::ActorHandle,
    simulator_mode: SimulatorMode,
) {
    loop {
        tracing::info!("Alice (Source) workflow: Waiting for a controller...");

        // Reset FIFOs for the new session.
        tracing::info!("Resetting FIFOs for new Alice session.");
        if let Err(e) = CONFIG.get().unwrap().ipc_config.reset_ipc_fifos() {
            tracing::error!("Failed to reset FIFOs for Alice: {}. Retrying in 500ms.", e);
            sleep(Duration::from_millis(500));
            continue;
        }

        // For Alice, the GCR file is not used for writing data, but the IPCWriterActor
        // requires a file handle. We open /dev/null as a black hole for any potential writes.
        let gcr_file_writer = match std::fs::OpenOptions::new().write(true).open("/dev/null") {
            Ok(file) => file,
            Err(e) => {
                tracing::error!("Failed to open /dev/null for GCR writer: {}. This is required for Alice's workflow. Exiting.", e);
                return;
            }
        };
        // Assuming `config` is available in this scope.
        let (angles_res, gc_read_res) = thread::scope(|s| {
            let angles_handle = s.spawn(|| {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&config.angle_file_path)
            });

            let gc_read_handle = s.spawn(|| {
                fs::OpenOptions::new()
                    .read(true)
                    .open(&config.gc_read_file_path)
            });

            // The .join() calls will block until the threads complete.
            // The `unwrap()` here will propagate panics from the threads.
            (
                angles_handle.join().unwrap(),
                gc_read_handle.join().unwrap(),
            )
        });

        let angles_file_writer = match angles_res {
            Ok(file) => file,
            Err(e) => {
                tracing::error!(
                    "Failed to open angles_file_path '{}': {}. Retrying in 500ms.",
                    &config.angle_file_path,
                    e
                );
                sleep(Duration::from_millis(500));
                continue;
            }
        };

        let gc_read_file_handle = match gc_read_res {
            Ok(file) => file,
            Err(e) => {
                tracing::error!(
                    "Failed to open gc_read_file '{}': {}. Retrying in 500ms.",
                    &config.gc_read_file_path,
                    e
                );
                sleep(Duration::from_millis(500));
                continue;
            }
        };

        let writer_handle = IPCWriterActorHandle::new(gcr_file_writer, angles_file_writer);

        tracing::info!("IPC files opened. Initializing IPCReader for Alice.");
        let ipc_reader = ipc::reader::IPCReader::new(
            config.command_path.clone(),
            gc_read_file_handle,
            simu_handle.clone(),
            writer_handle.clone(),
            simulator_mode,
        );

        tracing::info!("Starting IPC command processing loop for Alice.");
        if let Err(e) = ipc_reader.start() {
            tracing::warn!(
                "IPC processing for Alice ended with an error: {:?}. Preparing for new connection.",
                e
            );
        } else {
            tracing::info!("IPCReader for Alice exited cleanly. Preparing for new connection.");
        }
        sleep(Duration::from_millis(250));
    }
}

// Bob's (Detector) workflow: starts immediately, does not wait for controller.
fn run_bob_workflow(
    config: &BobIpcConfig,
    simu_handle: backend::actor::ActorHandle,
    simulator_mode: SimulatorMode,
) {
    loop {
        tracing::info!("Bob (Detector) workflow: Waiting for a controller...");

        // Reset FIFOs for the new session.
        tracing::info!("Resetting FIFOs for new Bob session.");
        if let Err(e) = CONFIG.get().unwrap().ipc_config.reset_ipc_fifos() {
            tracing::error!("Failed to reset FIFOs for Bob: {}. Retrying in 500ms.", e);
            sleep(Duration::from_millis(500));
            continue;
        }

        // Open file handles concurrently to prevent deadlocks with other processes.
        // The `OpenOptions` structs must outlive the futures created by `open()`.
        let (angles_res, gcr_res, gc_read_res) = thread::scope(|s| {
            let angles_handle = s.spawn(|| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&config.angle_file_path)
            });

            let gcr_handle = s.spawn(|| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&config.gcr_file_path)
            });

            let gc_read_handle = s.spawn(|| {
                std::fs::OpenOptions::new()
                    .read(true) // <-- This must be opened for READING for the IPCReader
                    .open(&config.gc_read_file_path)
            });

            (
                angles_handle.join().unwrap(),
                gcr_handle.join().unwrap(),
                gc_read_handle.join().unwrap(),
            )
        });

        let angles_file_writer = match angles_res {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(
                    "Failed to open angles_file_path '{}': {}. Retrying in 500ms.",
                    &config.angle_file_path,
                    e
                );
                sleep(Duration::from_millis(500));
                continue;
            }
        };

        let gcr_file_writer = match gcr_res {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(
                    "Failed to open gcr_file_path '{}': {}. Retrying in 500ms.",
                    &config.gcr_file_path,
                    e
                );
                sleep(Duration::from_millis(500));
                continue;
            }
        };

        let gc_read_file_handle = match gc_read_res {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(
                    "Failed to open gc_read_file_path '{}': {}. Retrying in 500ms.",
                    &config.gc_read_file_path,
                    e
                );
                sleep(Duration::from_millis(500));
                continue;
            }
        };

        let writer_handle = IPCWriterActorHandle::new(gcr_file_writer, angles_file_writer);

        tracing::info!("IPC files opened. Initializing IPCReader for Bob.");
        let ipc_reader = ipc::reader::IPCReader::new(
            config.command_path.clone(),
            gc_read_file_handle,
            simu_handle.clone(),
            writer_handle.clone(),
            simulator_mode,
        );

        tracing::info!("Starting IPC command processing loop for Bob.");
        if let Err(e) = ipc_reader.start() {
            tracing::error!(
                "IPC processing for Bob ended with an error: {:?}. Preparing for new connection.",
                e
            );
        } else {
            tracing::info!("IPCReader for Bob exited cleanly. Preparing for new connection.");
        }
        sleep(Duration::from_millis(250));
    }
}
