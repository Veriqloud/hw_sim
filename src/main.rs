pub mod backend;
pub mod cli_args;
pub mod config;
pub mod errors;
pub mod ipc;

use crate::config::Configuration;
use backend::simulation::builder::SimulatorBuilder;
use clap::Parser;
use ipc::{config::Configuration as IPCConfiguration, writer::actor::IPCWriterActorHandle};
use snafu::ResultExt;
use std::time::Duration;
use tokio::time::sleep;
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    if let Err(e) = app_main().await {
        eprintln!("Application exited with error: {}", e);
        std::process::exit(1);
    }
}

// Core application logic
async fn app_main() -> Result<(), crate::errors::Error> {
    let span = trace_span!("app_main");
    let _guard = span.enter();

    let args = cli_args::CliArgs::parse();

    let configuration: Configuration = if let Some(path) = args.conf.config_path {
        Configuration::new(path).context(errors::ConfigLoadSnafu)?
    } else {
        Configuration::default()
    };

    tracing::info!(
        "Running with configuration: {}",
        serde_json::to_string_pretty(&configuration)
            .unwrap_or_else(|e| format!("Failed to serialize config for logging: {}", e))
    );

    // Initialize logger
    let log_level_filter =
        TryInto::<tracing_subscriber::filter::LevelFilter>::try_into(configuration.log_level)
            .context(errors::LoggerInitializationSnafu)?;

    let log_id = Uuid::new_v4();
    let logfile_name = format!("simu_logs_{log_id}.log");
    let logfile_path_str = format!("{}/{}", args.logs_location, &logfile_name);

    let logfile_appender = tracing_appender::rolling::daily(&args.logs_location, &logfile_name);
    let stdout_level = log_level_filter
        .into_level()
        .unwrap_or(tracing::Level::INFO);
    let stdout_writer = std::io::stdout.with_max_level(stdout_level);

    // Attempt to initialize logger. This can fail if called multiple times.
    // For robust applications, consider using `set_global_default` and handling its Result,
    // or ensuring `init` is only called once. For this refactor, we assume it's called once.
    tracing_subscriber::fmt()
        .with_writer(stdout_writer.and(logfile_appender))
        .init();

    tracing::info!("log_level: {:?}", stdout_level);
    tracing::info!("log file path: {}", logfile_path_str);

    // Setup IPC FIFOs using the method on ipc_config
    configuration
        .ipc_config
        .setup_ipc_fifos()
        .context(errors::IpcConfigSnafu)?;

    tracing::info!(
        "Simulator with configuration : {:?}",
        &configuration.backend_config
    );
    tracing::info!("IPC with configuration : {:?}", &configuration.ipc_config);

    let sim = SimulatorBuilder::from_config(&configuration.backend_config);
    tracing::info!("Simulator modulator: {:?}", sim.role);
    let simu_handle = backend::actor::ActorHandle::new(sim);

    let angle_file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&configuration.ipc_config.angle_file_path)
        .await
        .context(errors::IOSnafu)?;
    tracing::info!(
        "Opened angle_file for writing: {}",
        &configuration.ipc_config.angle_file_path
    );
    let click_result_file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&configuration.ipc_config.click_result_file_path)
        .await
        .context(errors::IOSnafu)?;
    tracing::info!(
        "Opened click_result_file for writing: {}",
        &configuration.ipc_config.click_result_file_path
    );

    tracing::info!("Writer-side IPC files opened successfully. Initializing IPCWriterActorHandle.");
    let writer_handle =
        IPCWriterActorHandle::new(angle_file, click_result_file, simu_handle.clone());
    run_ipc_connection_loop(&configuration.ipc_config, simu_handle, writer_handle).await;

    Ok(())
}

// Extracted IPC connection loop
async fn run_ipc_connection_loop(
    config: &IPCConfiguration,
    simu_handle: backend::actor::ActorHandle,
    writer_handle: IPCWriterActorHandle,
) {
    loop {
        tracing::info!("Attempting to establish IPC connections. Waiting for a controller...");

        // Open gc_file (hw_sim: Read, controller: Write)
        let gc_file = match tokio::fs::OpenOptions::new()
            .read(true)
            .open(&config.gc_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!("Opened gc_file: {}", &config.gc_file_path);
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open gc_file '{}': {}. Retrying in 5s.",
                    &config.gc_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open cmd_file (hw_sim: Read, controller: Write)
        let cmd_file = match tokio::fs::OpenOptions::new()
            .read(true)
            .open(&config.command_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!("Opened cmd_file: {}", &config.command_file_path);
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open cmd_file '{}': {}. Retrying in 5s.",
                    &config.command_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open gcr_file (hw_sim: Write, controller: Read)
        let gcr_file = match tokio::fs::OpenOptions::new()
            .write(true) // Open for writing
            .open(&config.gcr_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!("Opened gcr_file for writing: {}", &config.gcr_file_path);
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open gcr_file '{}': {}. Retrying in 5s.",
                    &config.gcr_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        tracing::info!("All IPC files opened successfully. Initializing IPC handlers.");
        let ipc_reader = ipc::reader::IPCReader::new(
            cmd_file,
            gc_file,
            gcr_file, // Pass the opened gcr_file
            simu_handle.clone(),
            writer_handle.clone(),
        );

        tracing::info!("IPC handlers initialized. Starting IPC command processing loop.");
        if let Err(e) = ipc_reader.start().await {
            match e {
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
