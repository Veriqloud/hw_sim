pub mod backend;
pub mod cli_args;
pub mod config;
pub mod errors;
pub mod ipc;

use crate::config::Configuration;
use backend::simulation::builder::SimulatorBuilder;
use clap::Parser;
use ipc::writer::actor::IPCWriterActorHandle;
use snafu::ResultExt;
use std::time::Duration;
use tokio::time::sleep;
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

// Main entry point
#[tokio::main]
async fn main() {
    if let Err(e) = app_main().await {
        // If logger is initialized, error would have been traced.
        // This println is for errors before logger setup or if tracing fails.
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
    
    // Initialize logger
    let log_level_filter =
        TryInto::<tracing_subscriber::filter::LevelFilter>::try_into(configuration.log_level.clone()) // Clone LogLevel
            .context(errors::LoggerInitializationSnafu)?;

    let log_id = Uuid::new_v4();
    let logfile_name = format!("simu_logs_{log_id}.log");
    let logfile_path_str = format!("{}/{}", args.logs_location, &logfile_name);

    let logfile_appender = tracing_appender::rolling::daily(&args.logs_location, &logfile_name);
    let stdout_level = log_level_filter.into_level().unwrap_or(tracing::Level::INFO);
    let stdout_writer = std::io::stdout.with_max_level(stdout_level);
    
    // Attempt to initialize logger. This can fail if called multiple times.
    // For robust applications, consider using `set_global_default` and handling its Result,
    // or ensuring `init` is only called once. For this refactor, we assume it's called once.
    tracing_subscriber::fmt()
        .with_writer(stdout_writer.and(logfile_appender))
        .init();

    tracing::info!("log_level: {:?}", stdout_level);
    tracing::info!("log file path: {}", logfile_path_str);

    tracing::info!(
        "Running with configuration: {}",
        serde_json::to_string_pretty(&configuration)
            .unwrap_or_else(|e| format!("Failed to serialize config for logging: {}", e))
    );
    
    // Setup IPC FIFOs using the method on ipc_config
    configuration.ipc_config.setup_ipc_fifos().context(errors::IpcConfigSnafu)?;

    tracing::info!(
        "Simulator with configuration : {:?}",
        &configuration.backend_config
    );
    tracing::info!("IPC with configuration : {:?}", &configuration.ipc_config);

    let sim = SimulatorBuilder::from_config(configuration.backend_config.clone()); // Clone if not Copy
    tracing::info!("Simulator modulator: {:?}", sim.role);
    let simu_handle = backend::actor::ActorHandle::new(sim);
    
    run_ipc_connection_loop(&configuration, simu_handle).await;

    Ok(())
}

// Extracted IPC connection loop
async fn run_ipc_connection_loop(
    config: &Configuration,
    simu_handle: backend::actor::ActorHandle,
) {
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
