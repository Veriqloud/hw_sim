pub mod backend;
pub mod cli_args;
pub mod config;
pub mod errors;
pub mod ipc;

use crate::config::Configuration;
use backend::simulation::builder::SimulatorBuilder;
use clap::Parser;
// errors::UnixStreamSnafu is not used directly in main after refactor
use ipc::writer::actor::IPCWriterActorHandle;
// nix, nix::sys::stat::Mode, std::fs, std::path::Path are no longer needed here
use snafu::ResultExt;
use std::time::Duration;
// tokio::net::UnixListener is not used in the current main loop logic
use tokio::time::sleep; // Added for delays in the loop
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let span = trace_span!("main");

    let args = cli_args::CliArgs::parse();

    let configuration: Configuration = if let Some(path) = args.conf.config_path {
        // Error handling for Configuration::new will be done in main_app with Snafu
        // For now, this direct match is kept from the original main structure.
        // A full refactor would move this into main_app as well.
        match Configuration::new(path) {
            Ok(c) => c,
            Err(e) => {
                println!("ERROR loading configuration: {}", e);
                span.in_scope(|| tracing::error!("Configuration error: {}", e));
                return; // Exit if config loading fails before logger setup
            }
        }
    } else {
        Configuration::default()
    };

    tracing::info!(
        "Running with configuration: {}",
        serde_json::to_string_pretty(&configuration)
            .unwrap_or_else(|_| "Failed to serialize config".to_string())
    );

    // Initialize logger
    // This part should ideally be in main_app and use Snafu context.
    // For now, keeping it as is to match the provided main.rs structure.
    let log_level_filter =
        match TryInto::<tracing_subscriber::filter::LevelFilter>::try_into(configuration.log_level)
        {
            Ok(level_filter) => level_filter,
            Err(e) => {
                println!("Could not initialize logger (invalid log level): {}", e);
                return;
            }
        };

    let log_id = Uuid::new_v4();
    let logfile_name = format!("simu_logs_{log_id}.log");
    let logfile_path_str = format!("{}/{}", args.logs_location, &logfile_name);

    let logfile = tracing_appender::rolling::daily(&args.logs_location, &logfile_name);
    let stdout_level = log_level_filter
        .into_level()
        .unwrap_or(tracing::Level::INFO);
    let stdout = std::io::stdout.with_max_level(stdout_level);
    tracing_subscriber::fmt()
        .with_writer(stdout.and(logfile))
        .init(); // This can panic if called multiple times.

    tracing::info!("log_level: {:?}", stdout_level);
    tracing::info!("log file path: {}", logfile_path_str);

    &configuration.ipc_config.setup_ipc_fifos().unwrap();

    tracing::info!(
        "Simulator with configuration : {:?}",
        &configuration.backend_config
    );
    tracing::info!("IPC with configuration : {:?}", &configuration.ipc_config);

    let sim = SimulatorBuilder::from_config(configuration.backend_config);
    tracing::info!("Simulator modulator: {:?}", sim.role);
    let simu_handle = backend::actor::ActorHandle::new(sim);

    // Import the specific error type for matching IPC errors
    use crate::ipc::reader::errors::Error as IpcReaderError;

    loop {
        tracing::info!("Attempting to establish IPC connections. Waiting for a controller...");

        // Open angle_file (hw_sim: Write, controller: Read)
        // This will block until the controller opens its end for reading.
        let angle_file = match tokio::fs::OpenOptions::new()
            .write(true)
            .open(&configuration.ipc_config.angle_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!(
                    "Opened angle_file: {}",
                    &configuration.ipc_config.angle_file_path
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open angle_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.angle_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open gc_file (hw_sim: Read, controller: Write)
        let gc_file = match tokio::fs::OpenOptions::new()
            .read(true)
            .open(&configuration.ipc_config.gc_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!("Opened gc_file: {}", &configuration.ipc_config.gc_file_path);
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open gc_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.gc_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open cmd_file (hw_sim: Read, controller: Write)
        let cmd_file = match tokio::fs::OpenOptions::new()
            .read(true)
            .open(&configuration.ipc_config.command_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!(
                    "Opened cmd_file: {}",
                    &configuration.ipc_config.command_file_path
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open cmd_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.command_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open click_result_file (hw_sim: Write, controller: Read)
        let click_result_file = match tokio::fs::OpenOptions::new()
            .write(true)
            .open(&configuration.ipc_config.click_result_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!(
                    "Opened click_result_file: {}",
                    &configuration.ipc_config.click_result_file_path
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open click_result_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.click_result_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        tracing::info!("All IPC files opened successfully. Initializing IPC handlers.");
        let writer_handle =
            IPCWriterActorHandle::new(angle_file, click_result_file, simu_handle.clone());
        let ipc_reader =
            ipc::reader::IPCReader::new(cmd_file, gc_file, simu_handle.clone(), writer_handle);

        tracing::info!("IPC handlers initialized. Starting IPC command processing loop.");
        if let Err(e) = ipc_reader.start().await {
            match e {
                IpcReaderError::CommandFileIo { source }
                    if source.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    tracing::info!("Controller disconnected (EOF on command channel). Preparing for new connection.");
                }
                // The GcFileIo variant does not exist in ipc::reader::errors::Error, so this arm is removed.
                // Other IpcReaderError types, including any potential EOF on other files if they were
                // to be specifically added to the enum, will be caught by the wildcard below.
                _ => {
                    tracing::warn!(
                        "IPC processing ended with an error: {:?}. Preparing for new connection.",
                        e
                    );
                }
            }
        } else {
            tracing::info!("IPCReader exited cleanly (this is unexpected if it's meant to run indefinitely). Preparing for new connection.");
        }

        tracing::info!(
            "Current IPC session ended. Will attempt to listen for a new controller connection."
        );
        // Files and handlers are dropped here as they go out of scope.
        // A small delay before restarting the loop to prevent tight looping on persistent errors.
        sleep(Duration::from_secs(1)).await;
    }
}
